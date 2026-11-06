// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Starting the client's threads and scheduler, which fails only when the system runs out of threads or descriptors. A test refuses the next start of one on its own thread through `refuse`, to check what its caller does then.
use std::io;
use std::thread::{Builder, JoinHandle};

use base_scheduler_native::{NativeOptions, NativeScheduler};

/// What [`scheduler`] starts, as a test names it to refuse it.
pub(crate) const SCHEDULER: &str = "scheduler";

/// Start `run` on a thread named `name`.
pub(crate) fn spawn<T: Send + 'static>(
    name: &str,
    run: impl FnOnce() -> T + Send + 'static,
) -> io::Result<JoinHandle<T>> {
    refused(name)?;
    Builder::new().name(name.to_owned()).spawn(run)
}

/// Start a scheduler with `options`.
pub(crate) fn scheduler(options: NativeOptions) -> io::Result<NativeScheduler> {
    refused(SCHEDULER)?;
    NativeScheduler::with_options(options)
}

#[cfg(not(test))]
fn refused(_: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static REFUSED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Refuse the next start of `what`, a thread's name or [`SCHEDULER`], on this thread, as a system out of threads or descriptors would.
#[cfg(test)]
pub(crate) fn refuse(what: &str) {
    REFUSED.with_borrow_mut(|refused| refused.push(what.to_owned()));
}

#[cfg(test)]
fn refused(what: &str) -> io::Result<()> {
    REFUSED.with_borrow_mut(|refused| {
        refused
            .iter()
            .position(|name| name == what)
            .map(|index| refused.remove(index))
            .map_or(Ok(()), |name| {
                Err(io::Error::other(format!(
                    "the system refused to start {name}"
                )))
            })
    })
}
