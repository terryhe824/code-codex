use super::Outcome;
use crate::process_guard::AttachedProcess;
use serde::Deserialize;
use std::ffi::{CStr, c_char, c_void};
use std::fs;
use std::io::{self, Read as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const PREFERENCE: &str = "startup-recovery.json";
const CONSENT_TIMEOUT_SECS: u64 = 120;

pub(super) fn user_lock(path: &Path) -> io::Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing permission directory"))?;
    workspace_service::application_paths::validate_directory_path(parent)?;
    fs::create_dir_all(parent)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(io::Error::other("Invalid startup permission lock"));
    }
    // SAFETY: this owned file remains open for the lifetime of the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consent {
    Cancel,
    CloseOnce,
    Remember,
}

fn resolve_confirmed(
    automatic: bool,
    confirm: impl FnOnce() -> Result<Consent, String>,
    close: impl FnOnce() -> Result<(), String>,
    remember: impl FnOnce() -> Result<(), String>,
) -> Result<Outcome, String> {
    let choice = if automatic {
        Consent::CloseOnce
    } else {
        confirm()?
    };
    if choice == Consent::Cancel {
        return Ok(Outcome::Cancelled);
    }
    close()?;
    if choice == Consent::Remember && remember().is_err() {
        return Ok(Outcome::ReadyWithPreferenceWarning);
    }
    Ok(Outcome::Ready)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Preference {
    schema: u8,
    auto_restart: bool,
}

fn read_preference(path: &Path) -> Result<bool, String> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Could not read automatic restart permission: {error}"
            ));
        }
    };
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Automatic restart permission is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 4096 {
        return Err("Automatic restart permission exceeds its size limit".into());
    }
    let record: Preference =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid automatic restart permission")?;
    if record.schema != 1 {
        return Err("Invalid automatic restart permission schema".into());
    }
    Ok(record.auto_restart)
}

fn remember_permission(path: &Path) -> Result<(), String> {
    crate::renderer_storage::atomic(path, &serde_json::json!({"schema": 1, "autoRestart": true}))
        .map_err(|error| error.to_string())
}

fn revoke_permission(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Automatic restart permission was not cleared: {error}"
        )),
    }
}

pub(super) fn reset() -> Result<(), String> {
    let root =
        workspace_service::application_paths::data_root().map_err(|error| error.to_string())?;
    let _lock =
        user_lock(&root.join("startup-recovery.lock")).map_err(|error| error.to_string())?;
    revoke_permission(&root.join(PREFERENCE))
}

pub(super) async fn resolve(executable: &Path) -> Result<Outcome, String> {
    let runtime = tokio::runtime::Handle::current();
    let image = executable.to_owned();
    tokio::task::spawn_blocking(move || {
        let root =
            workspace_service::application_paths::data_root().map_err(|error| error.to_string())?;
        let lock = match user_lock(&root.join("startup-recovery.lock")) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(Outcome::Cancelled);
            }
            Err(error) => return Err(error.to_string()),
        };
        let Some(target) =
            AttachedProcess::for_running_executable(&image).map_err(|error| error.to_string())?
        else {
            return Ok(Outcome::Ready);
        };
        if session_is_managed(&image)? {
            return Ok(Outcome::Managed);
        }
        let preference = root.join(PREFERENCE);
        let automatic = read_preference(&preference)?;
        let outcome = resolve_confirmed(
            automatic,
            || prompt(true),
            || {
                target.verify_running().map_err(|error| error.to_string())?;
                if session_is_managed(&image)? {
                    return Err("Codex became managed while awaiting confirmation".into());
                }
                operate(&target, &image, true)?;
                runtime.block_on(wait_for_exit(&target, Duration::from_secs(15)))?;
                if crate::process_guard::is_executable_running(&image)
                    .map_err(|error| error.to_string())?
                {
                    return Err("Another Codex instance appeared; no relaunch was attempted".into());
                }
                Ok(())
            },
            || remember_permission(&preference),
        );
        drop(lock);
        outcome
    })
    .await
    .map_err(|_| "Startup recovery worker failed".to_owned())?
}

fn session_is_managed(image: &Path) -> Result<bool, String> {
    crate::process_guard::discover_listener_port(image)
        .map(|port| port.is_some())
        .map_err(|error| error.to_string())
}

pub(super) fn target_from_cli(
    pid: u32,
    created_ms: u64,
    image: &Path,
) -> Result<AttachedProcess, String> {
    if pid == 0 || created_ms == 0 {
        return Err("Invalid native restart process identity".into());
    }
    let target = AttachedProcess::for_running_executable(image)
        .map_err(|error| error.to_string())?
        .ok_or("The requested application is no longer running")?;
    if target.pid() != pid || target.created_ms() != created_ms {
        return Err("Restart process identity changed".into());
    }
    Ok(target)
}

pub(crate) async fn wait_for_exit(
    target: &AttachedProcess,
    duration: Duration,
) -> Result<(), String> {
    tokio::time::timeout(duration, target.wait_for_exit())
        .await
        .map_err(|_| {
            format!(
                "Codex did not exit within {} seconds. Relaunch was cancelled.",
                duration.as_secs()
            )
        })?
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn prompt(allow_remember: bool) -> Result<Consent, String> {
    let translate = crate::gui_support::localization::text;
    let mut warning = translate(
        "Codex is already open. Closing it may interrupt running replies or tasks and lose unsaved changes. Close its windows and reopen through Code-Codex? Saved chats and project files are not deleted.",
    );
    let cancel = translate("Cancel");
    let close = translate("Close and restart");
    let remember = if allow_remember {
        warning.push_str("\n\n");
        warning.push_str(&translate(
            "Automatically close and reopen Codex next time (do not ask again)",
        ));
        warning.push_str("\n");
        warning.push_str(&translate("Close Codex manually and try again. To revoke automatic restart permission, run code-codex reset-startup-recovery."));
        translate("Always restart")
    } else {
        String::new()
    };
    let script = format!(
        r#"
on run argv
    set warningText to item 1 of argv
    set cancelButton to item 2 of argv
    set closeButton to item 3 of argv
    set rememberButton to item 4 of argv
    set buttonList to {{cancelButton, closeButton}}
    if rememberButton is not "" then set end of buttonList to rememberButton
    try
        set reply to display dialog warningText with title "Code-Codex" buttons buttonList default button cancelButton cancel button cancelButton giving up after {CONSENT_TIMEOUT_SECS}
        if gave up of reply then return "cancel"
        if button returned of reply is closeButton then return "close"
        if button returned of reply is rememberButton then return "remember"
    on error number -128
        return "cancel"
    end try
    return "cancel"
end run
"#
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &script, "--", &warning, &cancel, &close, &remember])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not show restart confirmation: {error}"))?;
    if !output.status.success() {
        return Err("macOS could not show restart confirmation. No close was requested.".into());
    }
    match std::str::from_utf8(&output.stdout).map(str::trim) {
        Ok("cancel") => Ok(Consent::Cancel),
        Ok("close") => Ok(Consent::CloseOnce),
        Ok("remember") if allow_remember => Ok(Consent::Remember),
        _ => Err("Invalid restart confirmation response".into()),
    }
}

type Object = *mut c_void;
type Selector = *mut c_void;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Object;
    fn sel_registerName(name: *const c_char) -> Selector;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> Object;
    fn objc_autoreleasePoolPop(pool: Object);
}

struct AutoreleasePool(Object);
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        // SAFETY: this pool is popped once on the thread that created it.
        unsafe { objc_autoreleasePoolPop(self.0) }
    }
}

pub(crate) fn operate(
    target: &AttachedProcess,
    image: &Path,
    close: bool,
) -> Result<serde_json::Value, String> {
    target.verify_running().map_err(|error| error.to_string())?;
    let image: PathBuf = fs::canonicalize(image).map_err(|error| error.to_string())?;
    // SAFETY: each message uses the declared selector's exact Objective-C ABI
    // and any autoreleased values remain valid through the scoped pool.
    unsafe {
        let _pool = AutoreleasePool(objc_autoreleasePoolPush());
        let class = objc_getClass(c"NSRunningApplication".as_ptr());
        if class.is_null() {
            return Err("Native macOS application lifecycle is unavailable".into());
        }
        let send_pid: unsafe extern "C" fn(Object, Selector, libc::pid_t) -> Object =
            std::mem::transmute(objc_msgSend as *const ());
        let app = send_pid(
            class,
            sel_registerName(c"runningApplicationWithProcessIdentifier:".as_ptr()),
            target.pid() as libc::pid_t,
        );
        if app.is_null() {
            return Err(
                "The verified process is absent from the macOS application registry".into(),
            );
        }
        let send_object: unsafe extern "C" fn(Object, Selector) -> Object =
            std::mem::transmute(objc_msgSend as *const ());
        let url = send_object(app, sel_registerName(c"executableURL".as_ptr()));
        if url.is_null() {
            return Err("macOS did not report the running application executable".into());
        }
        let send_path: unsafe extern "C" fn(Object, Selector) -> *const c_char =
            std::mem::transmute(objc_msgSend as *const ());
        let path = send_path(url, sel_registerName(c"fileSystemRepresentation".as_ptr()));
        if path.is_null()
            || fs::canonicalize(Path::new(std::ffi::OsStr::from_bytes(
                CStr::from_ptr(path).to_bytes(),
            )))
            .ok()
            .as_ref()
                != Some(&image)
        {
            return Err(
                "The native application executable does not match the verified process".into(),
            );
        }
        target.verify_running().map_err(|error| error.to_string())?;
        let requested = if close {
            let send: unsafe extern "C" fn(Object, Selector) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            send(app, sel_registerName(c"terminate".as_ptr())) != 0
        } else {
            let send: unsafe extern "C" fn(Object, Selector, usize) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            send(app, sel_registerName(c"activateWithOptions:".as_ptr()), 0) != 0
        };
        if !requested {
            return Err("macOS did not accept the native application request".into());
        }
        Ok(serde_json::json!({"pid":target.pid(),"requested":true,"force":false}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn macos_cancel_leaves_original_instance_and_preference_untouched() {
        let closes = Cell::new(0);
        let saves = Cell::new(0);
        let outcome = resolve_confirmed(
            false,
            || Ok(Consent::Cancel),
            || {
                closes.set(closes.get() + 1);
                Ok(())
            },
            || {
                saves.set(saves.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(closes.get(), 0);
        assert_eq!(saves.get(), 0);
    }

    #[test]
    fn macos_remembered_permission_can_be_revoked_without_touching_another_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(PREFERENCE);
        let other = root.path().join("settings.json");
        fs::write(&other, b"fixture").unwrap();
        assert!(!read_preference(&path).unwrap());
        remember_permission(&path).unwrap();
        assert!(read_preference(&path).unwrap());
        revoke_permission(&path).unwrap();
        assert!(!read_preference(&path).unwrap());
        assert_eq!(fs::read(other).unwrap(), b"fixture");
    }

    #[test]
    fn macos_restart_permission_rejects_redirected_or_malformed_preferences() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(PREFERENCE);
        let other = root.path().join("settings.json");
        fs::write(&other, br#"{"schema":1,"autoRestart":true}"#).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        assert!(read_preference(&path).is_err());
        revoke_permission(&path).unwrap();
        assert!(other.is_file());
        fs::write(
            &path,
            br#"{"schema":1,"autoRestart":true,"autoRestart":false}"#,
        )
        .unwrap();
        assert!(read_preference(&path).is_err());
    }

    #[test]
    fn macos_never_remembers_permission_when_graceful_close_failed() {
        let saved = Cell::new(false);
        assert!(
            resolve_confirmed(
                false,
                || Ok(Consent::Remember),
                || Err("timed out".into()),
                || {
                    saved.set(true);
                    Ok(())
                },
            )
            .is_err()
        );
        assert!(!saved.get());
    }
}
