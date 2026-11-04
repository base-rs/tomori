// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-server` process's calls to the operating system that tests cannot reach: creating its scheduler and its console thread, which fail only when the system refuses them, handling signals, and ending the process at once, after which it writes no coverage.

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
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 4,
        threads: 4,
        capacity: 1024,
        ..Default::default()
    })
    .map_err(|error| format!("Could not start the scheduler: {error}"))?;
    let cancellation = Cancellation::new();
    #[cfg(unix)]
    let _signals = {
        use base_scheduler::Signals;
        scheduler
            .signal_subscribe(
                &[libc::SIGINT, libc::SIGTERM],
                Shutdown(cancellation.clone()),
            )
            .inspect_err(|error| eprintln!("Could not handle signals: {error}"))
            .ok()
    };
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

/// Read the console with `read` on a thread of its own, saying so if the system cannot start one.
pub(crate) fn console(read: impl FnOnce() + Send + 'static) {
    let reading = std::thread::Builder::new()
        .name("tomori-console".into())
        .spawn(read);
    if let Err(error) = reading {
        eprintln!("Could not read the console for pairing ({error}); confirm on the status page.");
    }
}

/// Exit at once, without running static destructors or `atexit` handlers, which could free state that a native engine thread is still using.
#[cfg(unix)]
pub(crate) fn exit_now(code: libc::c_int) -> ! {
    // SAFETY: `_exit` only ends the process; it is async-signal-safe and touches no Rust state.
    unsafe { libc::_exit(code) }
}

/// Give SIGINT and SIGTERM their default action, which ends the process without running destructors.
#[cfg(unix)]
fn restore_default_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `SIG_DFL` installs no handler, so nothing runs in signal context.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
}

/// Shut down in order on the first signal, and exit immediately on a repeated one in case shutdown hangs.
#[cfg(unix)]
struct Shutdown(Cancellation);

#[cfg(unix)]
impl base_scheduler::Notify<libc::c_int> for Shutdown {
    fn notify(&self, signal: libc::c_int) {
        if self.0.cancel() {
            eprintln!("Shutting down; signal again to exit immediately.");
        } else {
            // Engine threads may still be running, so static destructors must not run.
            exit_now(128 + signal);
        }
    }
}
