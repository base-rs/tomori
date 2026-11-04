// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ending a child process with its parent. On Linux this runs in the forked child before it executes its program, where the child's coverage counters are discarded with its memory, so it is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`.

use std::process::Command;

/// Have Linux end the process `command` starts, with `SIGTERM`, when this process ends, even if it is killed and cannot run `Drop`. Elsewhere, the child is only ended by `Drop`.
pub fn end_with_parent(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: this pre-exec closure only calls async-signal-safe libc
        // functions and does not access shared Rust state.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have ended before the request took effect.
                if libc::getppid() != parent {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}
