//! Whether a terminal pane's shell sits idle at its prompt, with nothing of
//! its own running: what lets a double Ctrl+C close the pane without killing
//! a job (`gentle_core::double_interrupt`). Every doubt answers "busy".
//!
//! - **macOS and Linux:** the PTY's foreground process group is the shell's
//!   own (the shell leads its session and group), and the shell is still the
//!   program started (not `exec`'d into `tmux` or an editor). Background
//!   jobs (`cmd &`) don't count, as they don't for ⌘W.
//! - **Windows:** ConPTY has no foreground group; the shell is idle while it
//!   has no child processes. Something started with `start` from `cmd` is a
//!   child too, so it keeps the pane open until it ends.

use alacritty_terminal::tty::Pty;

/// Asks the operating system about one terminal's shell.
pub struct ShellIdle(imp::Probe);

impl ShellIdle {
    /// For the shell `pty` runs, started as `program`. `None` when the
    /// shell's process can't be followed.
    pub fn new(pty: &Pty, program: &str) -> Option<Self> {
        imp::Probe::new(pty, program).map(Self)
    }

    /// Whether the shell waits at its prompt with nothing in the foreground.
    /// A few system calls: cheap enough for each Ctrl+C.
    pub fn idle(&self) -> bool {
        self.0.idle()
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::{OsString, c_int};
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::path::Path;

    use alacritty_terminal::tty::Pty;
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    unsafe extern "C" {
        /// The terminal's foreground process group (POSIX; `pid_t` is an
        /// `i32` on macOS and Linux).
        fn tcgetpgrp(fd: c_int) -> i32;
    }

    pub struct Probe {
        /// A duplicate of the PTY's master side, owned here.
        master: File,
        pid: u32,
        name: OsString,
    }

    impl Probe {
        pub fn new(pty: &Pty, program: &str) -> Option<Self> {
            Some(Self {
                master: pty.file().try_clone().ok()?,
                pid: pty.child().id(),
                name: Path::new(program).file_name()?.to_os_string(),
            })
        }

        pub fn idle(&self) -> bool {
            // SAFETY: a read-only query on a descriptor this probe owns.
            let group = unsafe { tcgetpgrp(self.master.as_raw_fd()) };
            if u32::try_from(group).ok() != Some(self.pid) {
                return false;
            }
            let pid = Pid::from_u32(self.pid);
            let mut system = System::new();
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                ProcessRefreshKind::new(),
            );
            system
                .process(pid)
                .is_some_and(|process| process.name() == self.name)
        }
    }
}

#[cfg(windows)]
mod imp {
    use alacritty_terminal::tty::Pty;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    pub struct Probe {
        pid: u32,
    }

    impl Probe {
        pub fn new(pty: &Pty, _program: &str) -> Option<Self> {
            let pid = pty.child_watcher().pid()?.get();
            Some(Self { pid })
        }

        pub fn idle(&self) -> bool {
            has_children(self.pid) == Some(false)
        }
    }

    /// Whether any running process was started by `pid`; `None` when the
    /// process list can't be read.
    fn has_children(pid: u32) -> Option<bool> {
        // SAFETY: a snapshot handle this function owns and closes; `entry`
        // is a valid, sized PROCESSENTRY32W.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            if entry.th32ParentProcessID == pid && entry.th32ProcessID != pid {
                found = true;
                break;
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        let _ = unsafe { CloseHandle(snapshot) };
        Some(found)
    }
}
