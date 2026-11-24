//! Ctrl-C during a headless model run: the first press ends the run like
//! the app's Stop, so what it made is saved; a second quits at once.
use super::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

/// The exit code of a process ended by Ctrl-C.
const INTERRUPTED: i32 = 130;

/// The run's cancel flag, set by the first Ctrl-C from here on.
pub(super) fn stop_on_ctrl_c() -> Result<Arc<AtomicBool>> {
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
                        eprintln!(
                            "stopping: the run ends at the model's next piece of output or the \
                             end of this step, and what it made is saved. Ctrl-C again quits \
                             without saving"
                        );
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
