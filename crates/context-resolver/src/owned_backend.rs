//! Own only backends launched in a fresh macOS session. Keep the direct child
//! unreaped until signaling its group, so its PID cannot name a recycled group.

use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::{Pid, getpgrp};
use process_wrap::tokio::{KillOnDrop, ProcessSession, TokioChildWrapper, TokioCommandWrap};
use std::future::Future;
use std::io;
use std::process::ExitStatus;
use tokio::process::{Child, Command};

#[derive(Debug)]
struct OwnedBackend {
    child: Option<Child>,
    group: Option<Pid>,
}

pub fn spawn_owned_backend(command: Command) -> io::Result<Box<dyn TokioChildWrapper>> {
    let mut command = TokioCommandWrap::from(command);
    command.wrap(KillOnDrop);
    // ProcessSession makes setsid(2) part of successful spawn and makes the
    // direct child's PID the new group ID. Unrelated processes cannot join.
    command.wrap(ProcessSession);
    let child = command.spawn()?.into_inner();
    let pid = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|&pid| pid > 1)
        .map(Pid::from_raw)
        .ok_or_else(|| io::Error::other("spawned backend has no usable group ID"))?;
    if pid == getpgrp() {
        return Err(io::Error::other(
            "backend did not receive a separate process group",
        ));
    }
    Ok(Box::new(OwnedBackend {
        child: Some(child),
        group: Some(pid),
    }))
}

impl OwnedBackend {
    fn stop_group(&mut self) -> io::Result<()> {
        let Some(group) = self.group else {
            return Ok(());
        };
        if self.inner().id() != Some(group.as_raw() as u32) || group == getpgrp() {
            return Err(io::Error::other(
                "owned backend group identity is unavailable",
            ));
        }
        match killpg(group, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => {
                self.group = None;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for OwnedBackend {
    fn drop(&mut self) {
        if self.child.is_some() {
            // The inner KillOnDrop also handles the directly spawned process if
            // group stop failed. It never sends a signal to any process group.
            let _ = self.stop_group();
        }
    }
}

impl TokioChildWrapper for OwnedBackend {
    fn inner(&self) -> &Child {
        self.child
            .as_ref()
            .expect("owned backend retains its child")
    }

    fn inner_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("owned backend retains its child")
    }

    fn into_inner(mut self: Box<Self>) -> Child {
        // The caller no longer retains the group ownership wrapper.
        let _ = self.stop_group();
        self.child.take().expect("owned backend retains its child")
    }

    fn start_kill(&mut self) -> io::Result<()> {
        self.stop_group()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // Tokio try_wait reaps the session leader on success. Remain
        // conservative while it still reserves the group identity.
        if self.group.is_some() {
            return Ok(None);
        }
        self.inner_mut().try_wait()
    }

    fn wait(&mut self) -> Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_> {
        // This wrapper is only used for owned, cancellable backends. A caller
        // that retires the leader must close the whole group before reaping.
        Box::new(async {
            self.stop_group()?;
            self.inner_mut().wait().await
        })
    }
}
