use crate::{Error, Result};
use std::{
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

/// An owned adapter leader with a fresh process group. Keep its PID unreaped
/// until group termination completes, so cleanup cannot target a reused PGID.
pub struct NativeProcess {
    child: Option<Child>,
}
impl NativeProcess {
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    pub fn spawn(mut command: Command) -> Result<Self> {
        command.process_group(0);
        Ok(Self {
            child: Some(command.spawn()?),
        })
    }
    pub fn wait(
        &mut self,
        timeout: Duration,
        mut monitor: impl FnMut() -> Result<()>,
    ) -> Result<ExitStatus> {
        let result = self.wait_inner(timeout, &mut monitor);
        if result.is_err() && self.child.is_some() {
            let _ = self.terminate();
        }
        result
    }
    fn wait_inner(
        &mut self,
        timeout: Duration,
        monitor: &mut impl FnMut() -> Result<()>,
    ) -> Result<ExitStatus> {
        let start = Instant::now();
        loop {
            monitor()?;
            let child = self
                .child
                .as_ref()
                .ok_or_else(|| crate::contracts::invalid("adapter already reaped"))?;
            let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            // WNOWAIT observes exit without releasing the PID/group identity.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { info.si_pid() } != 0 {
                return self.terminate();
            }
            if start.elapsed() >= timeout {
                return Err(Error::Resource("native time budget exhausted".into()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn terminate(&mut self) -> Result<ExitStatus> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| crate::contracts::invalid("adapter already reaped"))?;
        let group = -(child.id() as i32);
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(200));
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let status = child.wait()?;
        self.child = None;
        Ok(status)
    }
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.terminate();
        }
    }
}
