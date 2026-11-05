// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Starting the headset's connection and speaker threads, which fails only when the system runs out of threads or descriptors. The headset reports such a failure and carries on. A test refuses the next start of one on its own thread through `refuse`, to check that.

use std::io;
use std::thread::{Builder, JoinHandle};

use tomori_client::{Client, Options};

/// What [`connect`] starts, as a test names it to refuse it.
#[cfg(any(test, feature = "testing"))]
pub const CONNECTION: &str = "connection";

/// Start `run` on a thread named `name`.
pub fn spawn<T: Send + 'static>(
    name: &str,
    run: impl FnOnce() -> T + Send + 'static,
) -> io::Result<JoinHandle<T>> {
    #[cfg(any(test, feature = "testing"))]
    refused(name)?;
    Builder::new().name(name.to_owned()).spawn(run)
}

/// Start a client connecting as `options` say, which fails only if its thread or scheduler cannot start.
pub fn connect(options: Options) -> io::Result<Client> {
    #[cfg(any(test, feature = "testing"))]
    refused(CONNECTION)?;
    Client::connect(options)
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static REFUSED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Refuse the next start of `what`, a thread's name or [`CONNECTION`], on this thread, as a system out of threads or descriptors would.
#[cfg(any(test, feature = "testing"))]
pub fn refuse(what: &str) {
    REFUSED.with_borrow_mut(|refused| refused.push(what.to_owned()));
}

#[cfg(any(test, feature = "testing"))]
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
