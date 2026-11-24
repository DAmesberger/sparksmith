//! Ctrl-C during a headless model run or an export: the first press ends it
//! like the app's Stop (what a run made is saved, an export writes nothing);
//! a second quits at once.
use super::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

/// The exit code of a process ended by Ctrl-C.
const INTERRUPTED: i32 = 130;

/// What the first Ctrl-C of a model run says.
pub(super) const RUN: &str = "stopping: the run ends now, or at the end of the step it computes, and \
     what it made is saved. Ctrl-C again quits \
     without saving";

/// The run's cancel flag, set by the first Ctrl-C from here on, which says
/// `stopping`.
pub(super) fn stop_on_ctrl_c(stopping: &'static str) -> Result<Arc<AtomicBool>> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("ctrl-c".into())
        .spawn(move || {
            runtime.block_on(async {
                // Without a handler Ctrl-C ends the process, as it always did.
                if tokio::signal::ctrl_c().await.is_err() {
                    return;
                }
                flag.store(true, Ordering::Relaxed);
                // Listening for the second press before inviting it.
                let mut again = std::pin::pin!(tokio::signal::ctrl_c());
                let listening =
                    std::future::poll_fn(|cx| Poll::Ready(again.as_mut().poll(cx))).await;
                let quit = match listening {
                    Poll::Ready(pressed) => pressed,
                    Poll::Pending => {
                        eprintln!("{stopping}");
                        again.await
                    }
                };
                if quit.is_ok() {
                    std::process::exit(INTERRUPTED);
                }
            });
        })?;
    Ok(cancel)
}
