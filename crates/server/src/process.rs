// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-server` process: its scheduler, its console thread, and how it handles SIGINT and SIGTERM. Ending the process at once is in `exit.rs`.

use base_scheduler::Cancellation;
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::block_on;

use crate::Server;
use crate::command::{Arguments, serve};

/// Serve on a scheduler of its own until SIGINT or SIGTERM, then shut down in order, so that the Bonjour advertisement is withdrawn rather than left to expire.
pub(crate) fn serve_until_signalled(
    server: Server,
    arguments: &Arguments,
    fingerprint: Option<String>,
) -> Result<(), String> {
    let scheduler = crate::system::refused("scheduler")
        .and_then(|()| {
            NativeScheduler::with_options(NativeOptions {
                workers: 4,
                threads: 4,
                capacity: 1024,
                ..Default::default()
            })
        })
        .map_err(|error| format!("Could not start the scheduler: {error}"))?;
    let cancellation = Cancellation::new();
    #[cfg(unix)]
    let _signals = handle_signals(&scheduler, &cancellation, crate::exit::now);
    let result = block_on(serve(
        &scheduler,
        server,
        arguments,
        fingerprint,
        &cancellation,
        tomori_discovery::advertise,
    ));
    // Closing the scheduler unregisters its signal handlers, which would then swallow a repeated signal. Restore the default action first, so that a signal during the rest of shutdown still ends the process at once, without running destructors.
    #[cfg(unix)]
    restore_default_signals();
    block_on(scheduler.shutdown());
    result
}

/// Cancel `cancellation` on the first SIGINT or SIGTERM, and `exit` on a repeated one. Without signal handlers, which the system may refuse, the server is still stopped by the default action, at once.
#[cfg(unix)]
fn handle_signals<R: 'static>(
    scheduler: &NativeScheduler,
    cancellation: &Cancellation,
    exit: fn(libc::c_int) -> R,
) -> Option<<NativeScheduler as base_scheduler::Signals>::Subscription> {
    use base_scheduler::Signals;
    crate::system::refused("signals")
        .and_then(|()| {
            scheduler.signal_subscribe(
                &[libc::SIGINT, libc::SIGTERM],
                Shutdown {
                    cancellation: cancellation.clone(),
                    exit,
                },
            )
        })
        .inspect_err(|error| eprintln!("Could not handle signals: {error}"))
        .ok()
}

/// Read the console with `read` on a thread of its own, saying so if the system cannot start one.
pub(crate) fn console(read: impl FnOnce() + Send + 'static) {
    let reading = crate::system::spawn("tomori-console", read);
    if let Err(error) = reading {
        eprintln!("Could not read the console for pairing ({error}); confirm on the status page.");
    }
}

/// Give SIGINT and SIGTERM their default action, which ends the process without running destructors.
#[cfg(unix)]
fn restore_default_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `SIG_DFL` installs no handler, so nothing runs in signal context.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
}

/// Shut down in order on the first signal, and `exit` immediately on a repeated one, with 128 and the signal's number, in case shutdown hangs.
#[cfg(unix)]
struct Shutdown<R> {
    cancellation: Cancellation,
    /// Ends the process without running static destructors, since engine threads may still be running: [`crate::exit::now`] but in tests.
    exit: fn(libc::c_int) -> R,
}

#[cfg(unix)]
impl<R> base_scheduler::Notify<libc::c_int> for Shutdown<R> {
    fn notify(&self, signal: libc::c_int) {
        if self.cancellation.cancel() {
            eprintln!("Shutting down; signal again to exit immediately.");
        } else {
            (self.exit)(128 + signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    #[test]
    fn a_console_thread_that_cannot_start_is_no_failure() {
        crate::system::refuse("tomori-console");
        // The operator is told to confirm on the status page instead; reading is never started.
        console(|| unreachable!("read without a thread"));
    }

    #[cfg(unix)]
    #[test]
    fn the_first_signal_shuts_down_and_a_repeated_one_exits_with_its_status() {
        static EXITED: AtomicI32 = AtomicI32::new(0);
        fn exit(code: libc::c_int) {
            EXITED.store(code, Ordering::Relaxed);
        }
        let cancellation = Cancellation::new();
        let shutdown = Shutdown {
            cancellation: cancellation.clone(),
            exit,
        };
        base_scheduler::Notify::notify(&shutdown, libc::SIGTERM);
        assert!(cancellation.is_cancelled());
        assert_eq!(EXITED.load(Ordering::Relaxed), 0);
        base_scheduler::Notify::notify(&shutdown, libc::SIGINT);
        assert_eq!(EXITED.load(Ordering::Relaxed), 128 + libc::SIGINT);
    }

    #[cfg(unix)]
    #[test]
    fn signals_the_system_will_not_handle_are_reported() {
        let scheduler = NativeScheduler::with_options(NativeOptions::default()).unwrap();
        crate::system::refuse("signals");
        assert!(handle_signals(&scheduler, &Cancellation::new(), crate::exit::now).is_none());
        block_on(scheduler.shutdown());
    }
}
