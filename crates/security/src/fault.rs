// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! File operations whose failures the operating system almost never produces, such as taking a lock or syncing a file, so that tests can inject them. Production builds run each operation directly.
use std::io;
use std::path::Path;

/// An operation on the file at a path that a test can make fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Lock,
    LockShared,
    Write,
    Sync,
}

/// Run `operation`, an [`Operation`] on the file at `path`.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn run<T>(
    _: Operation,
    _: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    operation()
}

#[cfg(test)]
pub(crate) use injected::{inject, run};

#[cfg(test)]
mod injected {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    thread_local! {
        /// The failures injected on this thread, so that tests running in parallel do not see each other's.
        static INJECTED: RefCell<Vec<(Operation, PathBuf)>> = const { RefCell::new(Vec::new()) };
    }

    /// Fail `operation` on the file at `path` on this thread until the returned guard is dropped.
    pub(crate) fn inject(operation: Operation, path: &Path) -> Injected {
        let failure = (operation, path.to_owned());
        INJECTED.with_borrow_mut(|injected| injected.push(failure.clone()));
        Injected(failure)
    }

    pub(crate) struct Injected((Operation, PathBuf));

    impl Drop for Injected {
        fn drop(&mut self) {
            INJECTED.with_borrow_mut(|injected| injected.retain(|failure| *failure != self.0));
        }
    }

    pub(crate) fn run<T>(
        operation: Operation,
        path: &Path,
        run: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let injected = INJECTED.with_borrow(|injected| {
            injected
                .iter()
                .any(|(failing, at)| *failing == operation && at == path)
        });
        if injected {
            return Err(io::Error::other(format!("injected {operation:?} failure")));
        }
        run()
    }
}
