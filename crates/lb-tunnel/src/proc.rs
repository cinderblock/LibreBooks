//! Starting cloudflared so that it can never outlive LibreBooks.
//!
//! - **Windows**: the child goes in a job object with "kill on job close"; when
//!   LibreBooks exits for any reason (including a crash or Task Manager), Windows
//!   closes the job handle and ends the child. No console window is created.
//! - **Linux**: the child asks the kernel for SIGTERM when its parent dies
//!   (`PR_SET_PDEATHSIG`), and leads its own process group so a stop reaches any
//!   helpers it starts.
//! - **macOS / other Unix**: its own process group and kill-on-drop. There is no
//!   parent-death signal, so a hard crash of LibreBooks can leave cloudflared
//!   running until the next start replaces it (a known gap).
//!
//! The token is passed in the environment (`TUNNEL_TOKEN`), never on the command
//! line, which other users of the machine can read (`ps`, `/proc/*/cmdline`).

use std::path::Path;
use std::process::Stdio;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};

/// `CREATE_NO_WINDOW`: no console window for a child of a GUI process.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start `program` with `args` and extra environment, stdout and stderr piped.
pub(crate) fn spawn(
    program: &Path,
    args: &[&str],
    env: &[(String, String)],
) -> std::io::Result<Box<dyn ChildWrapper>> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "linux")]
    die_with_parent(&mut cmd);

    let mut wrap = CommandWrap::from(cmd);
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::PROCESS_CREATION_FLAGS;
        wrap.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)));
        wrap.wrap(JobObject);
    }
    #[cfg(unix)]
    wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
    wrap.wrap(KillOnDrop);
    wrap.spawn()
}

/// Ask Linux to send the child SIGTERM when this process dies.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn die_with_parent(cmd: &mut tokio::process::Command) {
    // SAFETY: the closure runs in the child between fork and exec, where only
    // async-signal-safe calls are allowed. prctl, getppid and _exit are all
    // async-signal-safe, and it touches no memory shared with the parent.
    let parent = std::process::id();
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died before prctl took effect.
            if libc::getppid() as u32 != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

/// Ask the child to stop: SIGTERM to its group on Unix (cloudflared then closes
/// its connections gracefully); on Windows there is no such request, so it is
/// ended.
pub(crate) fn request_stop(child: &mut dyn ChildWrapper) {
    #[cfg(unix)]
    {
        if child.signal(libc_sigterm()).is_ok() {
            return;
        }
    }
    let _ = child.start_kill();
}

#[cfg(unix)]
fn libc_sigterm() -> i32 {
    15
}
