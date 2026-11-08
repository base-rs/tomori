// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Asking the system for a scheduler or a thread, which fails only when it runs out of threads or descriptors. A test refuses the next request on its own thread through `refuse`, to check what its caller does then.
use std::io;

/// Fails as the system would if a test on this thread refused `what`; otherwise does nothing.
#[cfg(not(test))]
pub(crate) fn refused(_: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static REFUSED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Refuse the next request on this thread for `what`, as a system out of threads or descriptors would.
#[cfg(test)]
pub(crate) fn refuse(what: &str) {
    REFUSED.with_borrow_mut(|refused| refused.push(what.to_owned()));
}

/// Fails as the system would if a test on this thread refused `what`; otherwise does nothing.
#[cfg(test)]
pub(crate) fn refused(what: &str) -> io::Result<()> {
    REFUSED.with_borrow_mut(|refused| {
        refused
            .iter()
            .position(|name| name == what)
            .map(|index| refused.remove(index))
            .map_or(Ok(()), |name| {
                Err(io::Error::other(format!("the system refused {name}")))
            })
    })
}
