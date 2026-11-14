//! Explicit menu restart: pinned native identity, independent helper, nonforcing exit.
#[cfg(not(target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(any(windows, target_os = "macos"))] use std::{sync::atomic::{AtomicBool,Ordering},time::Duration};
#[cfg(windows)] use std::ffi::OsString;
#[cfg(any(windows, target_os = "macos"))] static PENDING:AtomicBool=AtomicBool::new(false);
fn valid_target(port:u16,pid:u32,created_ms:u64)->bool{port!=0&&pid!=0&&created_ms!=0}
#[cfg(not(target_os = "macos"))]
fn launcher(root:&Path)->Result<PathBuf,String>{
 workspace_service::application_paths::validate_install_root(root).map_err(|e|e.to_string())?;
 let path=root.join("CodeCodex.exe");let m=std::fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
 if !m.is_file()||m.file_type().is_symlink(){return Err("The managed launcher is not a regular file.".into());}
 #[cfg(windows)]{use std::os::windows::fs::MetadataExt;if m.file_attributes()&0x400!=0{return Err("Redirected managed launcher refused.".into());}}
 Ok(path)
}
#[cfg(windows)] fn installation()->Result<crate::discovery::CodexInstallation,String>{
 crate::discovery::discover_codex(None,None,crate::discovery::ChannelPreference::Stable).map_err(|e|e.to_string())
}
pub(crate) async fn schedule(port:u16,root:PathBuf,current:impl Fn()->bool)->Result<(),String>{
 #[cfg(target_os = "macos")]{macos::schedule(port,root,current).await}
 #[cfg(not(any(windows, target_os = "macos")))]{let _=(port,root,current);Err("Restart is unavailable on this platform.".into())}
 #[cfg(windows)]{
 if PENDING.swap(true,Ordering::AcqRel){return Err("A Code-Codex restart is already pending.".into());}
 let result=async{
  launcher(&root)?;let app=installation()?;let image=app.executable;
  let target=tokio::task::spawn_blocking(move||crate::process_guard::AttachedProcess::for_listener(port,&image)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
  let args:Vec<OsString>=vec!["menu-restart".into(),"--port".into(),port.to_string().into(),"--pid".into(),target.pid().to_string().into(),"--created-ms".into(),target.created_ms().to_string().into()];
  let exe=std::env::current_exe().map_err(|e|e.to_string())?;
  if !current(){return Err("The restart request belongs to an inactive document. No shutdown was scheduled.".into());}
  let started=std::time::SystemTime::now();
  let pid=crate::activation_lifetime::broker(&exe,&args).await.map_err(|e|e.to_string())?;
  let handle=crate::process_guard::retain_native_identity(pid,started,&exe,None).map_err(|e|e.to_string())?;
  tokio::spawn(async move{loop{match crate::process_guard::process_handle_exit_code(&handle){Ok(None)=>tokio::time::sleep(Duration::from_millis(500)).await,Ok(Some(_))=>{PENDING.store(false,Ordering::Release);break;},Err(_)=>break,}}});
  crate::runtime_log::record("launcher","menu restart","scheduled",serde_json::json!({"code":"CC-MENU-RESTART-001","workerPid":pid,"targetPid":target.pid(),"force":false}));Ok(())
 }.await;
 if result.is_err(){PENDING.store(false,Ordering::Release);}result
 }
}
pub(crate) async fn run(port:u16,pid:u32,created_ms:u64)->Result<(),String>{
 #[cfg(target_os = "macos")]{macos::run(port,pid,created_ms).await}
 #[cfg(not(any(windows, target_os = "macos")))]{let _=(port,pid,created_ms);Err("Restart is unavailable on this platform.".into())}
 #[cfg(windows)]{
 if !valid_target(port,pid,created_ms){return Err("Invalid restart target identity.".into());}
 let app=installation()?;let image=app.executable;
 let root=workspace_service::application_paths::install_root().map_err(|e|e.to_string())?.ok_or("Managed installation unavailable.")?;
 let root=dunce::canonicalize(root).map_err(|e|e.to_string())?;
 if !dunce::canonicalize(std::env::current_exe().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?.starts_with(&root){return Err("The restart helper does not belong to this installation.".into());}
 let executable=launcher(&root)?;let check=image.clone();
 let target=tokio::task::spawn_blocking(move||crate::process_guard::AttachedProcess::for_listener(port,&check)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
 if pid==0||target.pid()!=pid||target.created_ms()!=created_ms{return Err("The restart target identity changed. No shutdown was requested.".into());}
 // Give the authenticated menu response a chance to finish; no task is replayed.
 tokio::time::sleep(Duration::from_millis(750)).await;
 target.verify_listener(port).map_err(|e|e.to_string())?;
 let close_image=image.clone();tokio::task::spawn_blocking(move||crate::startup_conflict::close_for_restart(pid,created_ms,&close_image)).await.map_err(|e|e.to_string())??;
 tokio::time::timeout(Duration::from_secs(30),target.wait_for_exit()).await.map_err(|_|"Codex did not exit within 30 seconds; no forced termination or relaunch was attempted.".to_owned())?.map_err(|e|e.to_string())?;
 if tokio::task::spawn_blocking(move||crate::process_preflight::after_restart(&image)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?{return Err("Another Codex instance is still running; relaunch was not attempted.".into());}
 // This worker was brokered outside the caller job and survives the old GUI.
 use std::os::windows::process::CommandExt;
 std::process::Command::new(executable).current_dir(std::env::temp_dir()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).creation_flags(0x08000000).spawn().map_err(|e|format!("Managed relaunch failed: {e}"))?;
 crate::runtime_log::record("launcher","menu restart","relaunched",serde_json::json!({"code":"CC-MENU-RESTART-002","oldProcessExited":true,"managedLauncher":true,"force":false}));Ok(())
 }
}
#[cfg(test)] mod tests{
 use super::*;
 #[test]fn restart_target_requires_concrete_identity(){assert!(valid_target(1234,42,123));assert!(!valid_target(0,42,123));assert!(!valid_target(1234,0,123));assert!(!valid_target(1234,42,0));}
 #[cfg(not(target_os = "macos"))]
 #[test]fn restart_requires_managed_regular_launcher(){let d=tempfile::tempdir().unwrap();assert!(launcher(d.path()).is_err());std::fs::write(d.path().join("storage-layout.json"),br#"{"schema":1,"product":"Code-Codex"}"#).unwrap();assert!(launcher(d.path()).is_err());std::fs::create_dir(d.path().join("CodeCodex.exe")).unwrap();assert!(launcher(d.path()).is_err());std::fs::remove_dir(d.path().join("CodeCodex.exe")).unwrap();std::fs::write(d.path().join("CodeCodex.exe"),b"never executed fixture").unwrap();assert_eq!(launcher(d.path()).unwrap(),d.path().join("CodeCodex.exe"));}
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::discovery::{ChannelPreference, DiscoverySource, discover_codex};
    use crate::process_guard::AttachedProcess;
    use crate::startup_conflict::{macos as lifecycle, macos_user_lock};
    use std::process::{Command, Stdio};

    fn target(port: u16) -> Result<(AttachedProcess, PathBuf, ChannelPreference), String> {
        let mut verified = None;
        for channel in [ChannelPreference::Stable, ChannelPreference::Beta] {
            let Ok(installation) = discover_codex(None, None, channel) else {
                continue;
            };
            if installation.source != DiscoverySource::MacApplication {
                continue;
            }
            if verified.as_ref().is_some_and(|(_, image, _)| *image == installation.executable) {
                continue;
            }
            if let Ok(process) = AttachedProcess::for_listener(port, &installation.executable) {
                if verified.is_some() {
                    return Err("More than one application matches the restart listener".into());
                }
                verified = Some((process, installation.executable, channel));
            }
        }
        verified.ok_or("The native restart listener does not match an official application".into())
    }

    pub(super) async fn schedule(
        port: u16,
        root: PathBuf,
        current: impl Fn() -> bool,
    ) -> Result<(), String> {
        if PENDING.swap(true, Ordering::AcqRel) {
            return Err("A Code-Codex restart is already pending".into());
        }
        let result = async {
            let managed = crate::login_startup::macos::managed_launcher(&root).map_err(|error| error.to_string())?;
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let managed_directory = managed.parent().ok_or("Managed launcher directory is missing")?;
            if std::fs::canonicalize(&executable).map_err(|error| error.to_string())?.parent()
                != Some(std::fs::canonicalize(managed_directory).map_err(|error| error.to_string())?.as_path())
            {
                return Err("The restart helper does not belong to this installation".into());
            }
            let (target, _, _) = tokio::task::spawn_blocking(move || target(port)).await.map_err(|error| error.to_string())??;
            if !current() {
                return Err("The restart request belongs to an inactive document".into());
            }

            // This new session can outlive the supervisor and its launching terminal.
            use std::os::unix::process::CommandExt as _;
            let mut worker = Command::new(executable);
            worker.args([
                "menu-restart", "--port", &port.to_string(), "--pid",
                &target.pid().to_string(), "--created-ms", &target.created_ms().to_string(),
            ]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            // SAFETY: setsid is async-signal-safe and the closure uses no
            // data capture or locks in the forked child.
            unsafe {
                worker.pre_exec(|| {
                    if libc::setsid() == -1 { return Err(std::io::Error::last_os_error()); }
                    Ok(())
                });
            }
            let mut worker = tokio::process::Command::from(worker).spawn()
                .map_err(|error| format!("Could not start restart helper: {error}"))?;
            crate::runtime_log::record("launcher", "menu restart", "scheduled", serde_json::json!({
                "code":"CC-MENU-RESTART-001", "workerPid":worker.id(), "targetPid":target.pid(), "force":false
            }));
            tokio::spawn(async move {
                let _ = worker.wait().await;
                PENDING.store(false, Ordering::Release);
            });
            Ok(())
        }.await;
        if result.is_err() { PENDING.store(false, Ordering::Release); }
        result
    }

    pub(super) async fn run(port: u16, pid: u32, created_ms: u64) -> Result<(), String> {
        if !valid_target(port, pid, created_ms) {
            return Err("Invalid restart target identity".into());
        }
        let launcher = crate::login_startup::macos::installed_launcher().map_err(|error| error.to_string())?
            .ok_or("Managed installation unavailable")?;
        let data = workspace_service::application_paths::data_root().map_err(|error| error.to_string())?;
        let _lock = macos_user_lock(&data.join("startup-recovery.lock")).map_err(|error| error.to_string())?;
        let (target, image, channel) = tokio::task::spawn_blocking(move || target(port)).await.map_err(|error| error.to_string())??;
        if target.pid() != pid || target.created_ms() != created_ms {
            return Err("The restart target identity changed before confirmation".into());
        }
        let targets = tokio::task::spawn_blocking(move || {
            if lifecycle::prompt(false)? == lifecycle::Consent::Cancel {
                return Ok::<_, String>(None);
            }
            target.verify_listener(port).map_err(|error| error.to_string())?;
            lifecycle::operate(&target, &image, true)?;
            Ok(Some((target, image)))
        }).await.map_err(|error| error.to_string())??;
        let Some((target, image)) = targets else { return Ok(()); };
        lifecycle::wait_for_exit(&target, Duration::from_secs(30)).await?;
        if crate::process_guard::is_executable_running(&image).map_err(|error| error.to_string())? {
            return Err("Another Codex process appeared. Relaunch was cancelled.".into());
        }

        let bundle = image.ancestors().nth(3).ok_or("Verified app bundle is missing")?;
        let channel = match channel {
            ChannelPreference::Beta => "beta",
            _ => "stable",
        };
        Command::new(launcher).args(["run", "--channel", channel, "--codex-exe"]).arg(bundle)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|error| format!("Managed relaunch failed: {error}"))?;
        crate::runtime_log::record("launcher", "menu restart", "relaunched", serde_json::json!({
            "code":"CC-MENU-RESTART-002", "oldProcessExited":true, "managedLauncher":true, "force":false
        }));
        Ok(())
    }
}
