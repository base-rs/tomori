// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ctrl+C and the signals that ask a program to end: a recording that is interrupted is finished, not cut off, so the first of these is noted, from the handler, and the recording ends at the next frame. A second one ends the program at once, whatever it is waiting for, as it would have before: the encoder, in a process group of its own, finishes the file by itself.
//!
//! A signal that was ignored when the program started, as SIGHUP and SIGINT are for a program started with `nohup` or in the background of a script, stays ignored; the others are put back as they were by [`release`].
use super::force_exit;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicI32, AtomicUsize, Ordering},
};

/// The first signal to arrive, or 0.
static RECEIVED: AtomicI32 = AtomicI32::new(0);

/// What ends the program when a second signal arrives, as a `fn(c_int)`: [`force_exit::end_by`], which [`catch`] stores unless a test has stored another.
static FORCE: AtomicUsize = AtomicUsize::new(0);

/// The signals that end a recording: Ctrl+C, `kill`, and the terminal closing.
const ENDING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// What each signal that [`catch`] took over did before.
static BEFORE: Mutex<Vec<(libc::c_int, libc::sigaction)>> = Mutex::new(Vec::new());

extern "C" fn note(signal: libc::c_int) {
    if RECEIVED
        .compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        return;
    }
    // The second: the program is not to wait for anything more.
    let force = FORCE.load(Ordering::Relaxed);
    // SAFETY: `catch` stores a `fn(c_int)` before it installs this handler, and a test stores another; it has the size of a `usize`.
    unsafe { std::mem::transmute::<usize, fn(libc::c_int)>(force)(signal) }
}

fn zeroed() -> libc::sigaction {
    // SAFETY: an all-zero `sigaction` is a valid one: no handler, no flags, an empty mask.
    unsafe { std::mem::zeroed() }
}

/// Make `signal` run `handler`, returning what it did before.
fn set(signal: libc::c_int, handler: libc::sighandler_t) -> libc::sigaction {
    let mut action = zeroed();
    action.sa_sigaction = handler;
    let mut before = zeroed();
    // SAFETY: the action is given a handler that only touches atomics and calls async-signal-safe functions, an empty mask and no flags, so that a signal interrupts a system call rather than restarting it; both pointers are to live actions.
    let status = unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, &mut before)
    };
    // It fails only for a signal that cannot be handled, and these can.
    assert_eq!(status, 0, "the signal {signal} can be handled");
    before
}

/// Note the signals that end a recording from now on, instead of ending the program at once, but for those that were ignored.
pub fn catch() {
    let _ = FORCE.compare_exchange(
        0,
        force_exit::end_by as fn(libc::c_int) as usize,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    let mut taken = BEFORE.lock().unwrap_or_else(PoisonError::into_inner);
    for signal in ENDING {
        let before = set(
            signal,
            note as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        if before.sa_sigaction == libc::SIG_IGN {
            // Ignored it was, and ignored it stays.
            set(signal, libc::SIG_IGN);
        } else if !taken.iter().any(|(taken, _)| *taken == signal) {
            taken.push((signal, before));
        }
    }
}

/// Put the signals [`catch`] took over back as they were, and forget any that arrived.
pub fn release() {
    let mut taken = BEFORE.lock().unwrap_or_else(PoisonError::into_inner);
    for (signal, before) in taken.drain(..) {
        // SAFETY: `before` is what `sigaction` returned for this signal.
        unsafe { libc::sigaction(signal, &before, std::ptr::null_mut()) };
    }
    RECEIVED.store(0, Ordering::Relaxed);
}

/// The name of the first signal that arrived since [`catch`], if one has.
pub fn received() -> Option<&'static str> {
    match RECEIVED.load(Ordering::Relaxed) {
        0 => None,
        libc::SIGINT => Some("SIGINT"),
        libc::SIGTERM => Some("SIGTERM"),
        libc::SIGHUP => Some("SIGHUP"),
        _ => Some("a signal"),
    }
}

#[cfg(test)]
pub(crate) fn forget() {
    RECEIVED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    static FORCED: Mutex<Vec<libc::c_int>> = Mutex::new(Vec::new());

    fn forced(signal: libc::c_int) {
        FORCED.lock().unwrap().push(signal);
    }

    fn handler(signal: libc::c_int) -> libc::sighandler_t {
        let mut action = zeroed();
        // SAFETY: reading the current action, and putting it back.
        unsafe { libc::sigaction(signal, std::ptr::null(), &mut action) };
        action.sa_sigaction
    }

    fn raise(signal: libc::c_int) {
        // SAFETY: raising a signal on this thread, which is handled or ignored.
        unsafe { libc::raise(signal) };
    }

    #[test]
    fn the_signals_that_end_a_program_are_noted_and_a_second_ends_it() {
        forget();
        FORCE.store(forced as fn(libc::c_int) as usize, Ordering::Relaxed);
        assert_eq!(received(), None);
        RECEIVED.store(libc::SIGHUP, Ordering::Relaxed);
        assert_eq!(received(), Some("SIGHUP"));
        forget();
        // A program started with its hang-ups ignored, as by `nohup`, keeps them ignored.
        // SAFETY: setting a disposition for this test, put back below.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        catch();
        let noted = note as extern "C" fn(libc::c_int) as libc::sighandler_t;
        assert_eq!(handler(libc::SIGINT), noted);
        assert_eq!(handler(libc::SIGTERM), noted);
        assert_eq!(handler(libc::SIGHUP), libc::SIG_IGN);
        raise(libc::SIGHUP);
        assert_eq!(received(), None);
        // Catching again takes nothing over twice.
        catch();
        assert_eq!(BEFORE.lock().unwrap().len(), 2);

        raise(libc::SIGTERM);
        assert_eq!(received(), Some("SIGTERM"));
        assert!(FORCED.lock().unwrap().is_empty());
        // The first stays, and a second, of any, is what ends the program.
        raise(libc::SIGINT);
        assert_eq!(received(), Some("SIGTERM"));
        assert_eq!(*FORCED.lock().unwrap(), [libc::SIGINT]);
        raise(libc::SIGTERM);
        assert_eq!(*FORCED.lock().unwrap(), [libc::SIGINT, libc::SIGTERM]);

        forget();
        raise(libc::SIGINT);
        assert_eq!(received(), Some("SIGINT"));
        // A recording's stop is asked for by the signal; one that does not watch for signals is not.
        let watching = super::super::Stop::on_signals();
        assert!(watching.requested());
        assert_eq!(watching.reason().as_deref(), Some("received SIGINT"));
        assert!(!super::super::Stop::default().requested());
        forget();
        assert!(!super::super::Stop::on_signals().requested());
        RECEIVED.store(libc::SIGUSR1, Ordering::Relaxed);
        assert_eq!(received(), Some("a signal"));

        // Released, they are as they were: the default for two, and the one that was ignored.
        release();
        assert_eq!(received(), None);
        assert_eq!(handler(libc::SIGINT), libc::SIG_DFL);
        assert_eq!(handler(libc::SIGTERM), libc::SIG_DFL);
        assert_eq!(handler(libc::SIGHUP), libc::SIG_IGN);
        // SAFETY: putting the disposition back.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_DFL) };
        FORCE.store(0, Ordering::Relaxed);
    }
}
