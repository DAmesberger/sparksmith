//! What the model's and the registry's HTTP share: one request on a thread
//! of its own, given up when nothing arrives for as long as the person
//! allows, and ended at once when its call is cancelled.
//!
//! The limit is on silence, never on a whole reply: a long answer that
//! keeps arriving, or a large part on a slow line, is never cut. It is the
//! person's (`settings::Network`, a connection's own) and may be none. The
//! HTTP client sets no TCP keepalive, so without a limit only Stop ends the
//! wait for a peer that vanished without closing its connection.
use sparksmith_automation::cancel;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How often a waiting request looks at its call's cancellation.
const POLL: Duration = Duration::from_millis(50);

/// How long a request may hear nothing before it is given up (none: no
/// limit), and where the person sets it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Silence {
    pub after: Option<Duration>,
    pub set_in: &'static str,
}

impl Silence {
    /// Why a request was given up: which limit, its value and where to
    /// change it.
    pub fn why(&self, who: &str) -> String {
        let after = self.after.map_or_else(String::new, spoken);
        format!(
            "Nothing arrived from {who} for {after}, the limit set in {}.",
            self.set_in
        )
    }
}

/// "30 minutes", "4 seconds".
fn spoken(time: Duration) -> String {
    let seconds = time.as_secs();
    let (n, unit) = if seconds >= 60 && seconds.is_multiple_of(60) {
        (seconds / 60, "minute")
    } else {
        (seconds, "second")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// Whether `error` is the silence limit's: nothing was read or written, or
/// no connection came about, for as long as it allows.
pub(crate) fn silent(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Timeout(
            ureq::Timeout::PerRead
            | ureq::Timeout::PerWrite
            | ureq::Timeout::Connect
            | ureq::Timeout::Resolve,
        ) => true,
        ureq::Error::Io(error) => silent_io(error),
        _ => false,
    }
}

/// [`silent`] of an error met while a reply's body was read.
pub(crate) fn silent_io(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
        .is_some_and(silent)
}

/// HTTP agents for requests that run at once: each request has one to
/// itself while it runs, so ending it ends no other.
pub(crate) struct Agents {
    config: ureq::config::Config,
    idle: Arc<Mutex<Vec<ureq::Agent>>>,
}

impl Agents {
    /// Agents that follow no redirects; those for this computer (`local`)
    /// never go through a proxy.
    pub(crate) fn new(local: bool) -> Self {
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0);
        if local {
            config = config.proxy(None);
        }
        Self {
            config: config.build(),
            idle: Arc::default(),
        }
    }

    /// Run `work` on a thread of its own; it says what it has through the
    /// sender, whatever ends it last.
    pub(crate) fn start<T: Send + 'static>(
        &self,
        work: impl FnOnce(&ureq::Agent, &Sender<T>) + Send + 'static,
    ) -> Result<Running<T>, String> {
        let kept = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let agent = kept.unwrap_or_else(|| ureq::Agent::new_with_config(self.config.clone()));
        let (send, steps) = channel();
        let own = agent.clone();
        std::thread::Builder::new()
            .name("http-request".into())
            .spawn(move || work(&own, &send))
            .map_err(|e| format!("the request could not be started: {e}"))?;
        Ok(Running {
            steps,
            agent: Some(agent),
            idle: Arc::clone(&self.idle),
        })
    }
}

/// A request running. Dropped before it is [`Running::done`], it is ended
/// and its connection closed.
pub(crate) struct Running<T> {
    steps: Receiver<T>,
    agent: Option<ureq::Agent>,
    idle: Arc<Mutex<Vec<ureq::Agent>>>,
}

/// What a running request says next.
pub(crate) enum Next<T> {
    Step(T),
    /// The call it runs for was cancelled.
    Cancelled,
    /// It ended without an answer.
    Gone,
}

impl<T> Running<T> {
    pub(crate) fn next(&self) -> Next<T> {
        loop {
            if cancel::requested() {
                return Next::Cancelled;
            }
            match self.steps.recv_timeout(POLL) {
                Ok(step) => return Next::Step(step),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Next::Gone,
            }
        }
    }

    /// It said all it had: its connection is kept for the next request.
    pub(crate) fn done(mut self) {
        if let Some(agent) = self.agent.take() {
            self.idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(agent);
        }
    }
}

impl<T> Drop for Running<T> {
    fn drop(&mut self) {
        if let Some(agent) = self.agent.take() {
            agent.shutdown();
        }
    }
}

/// The tests' servers on this computer.
#[cfg(test)]
pub(crate) mod test_server {
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    /// How long a test server waits for its one client, and for each read
    /// or write after: longer than any client limit the tests set.
    pub(crate) const WAIT: Duration = Duration::from_secs(30);

    /// The one client of `listener` within `wait`, its reads and writes
    /// bounded by `wait` too; `None` when none came. A server thread that
    /// accepted without a bound waited forever for a client that gave up
    /// before it connected (its connect limit is the silence limit, 50 ms
    /// in some tests): on a busy machine the test then hung at the join.
    pub(crate) fn accept_within(listener: &TcpListener, wait: Duration) -> Option<TcpStream> {
        listener.set_nonblocking(true).ok()?;
        let until = Instant::now() + wait;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).ok()?;
                    stream.set_read_timeout(Some(wait)).ok()?;
                    stream.set_write_timeout(Some(wait)).ok()?;
                    return Some(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= until {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return None,
            }
        }
    }

    /// A server nobody calls gives up at its wait, and the test goes on.
    #[test]
    fn a_server_nobody_calls_gives_up() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let (said, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = said.send(accept_within(&listener, Duration::from_millis(200)).is_none());
        });
        assert_eq!(heard.recv_timeout(Duration::from_secs(10)), Ok(true));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_is_said_with_its_value_and_its_place() {
        let silence = |seconds| Silence {
            after: Some(Duration::from_secs(seconds)),
            set_in: "Settings › Network",
        };
        assert_eq!(
            silence(1800).why("the model"),
            "Nothing arrived from the model for 30 minutes, the limit set in Settings › Network."
        );
        assert!(silence(60).why("it").contains("for 1 minute,"));
        assert!(silence(90).why("it").contains("for 90 seconds,"));
    }
}
