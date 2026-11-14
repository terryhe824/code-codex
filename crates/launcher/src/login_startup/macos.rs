use super::Status;
use std::ffi::{CStr, OsStr};
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const LABEL: &str = "org.code-codex.login-startup";
const MAX_ENTRY_BYTES: u64 = 32768;

pub(crate) fn managed_launcher(root: &Path) -> io::Result<PathBuf> {
    workspace_service::application_paths::validate_install_root(root)?;
    if !root
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case("app"))
    {
        return Err(io::Error::other(
            "Login startup requires a managed app bundle",
        ));
    }
    let folder = root.join("Contents/MacOS");
    workspace_service::application_paths::validate_directory_path(&folder)?;
    let path = folder.join("Code-Codex");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::other(
            "Managed launcher is not an executable regular file",
        ));
    }
    Ok(path)
}

pub(crate) fn installed_launcher() -> io::Result<Option<PathBuf>> {
    let Some(root) = workspace_service::application_paths::install_root()? else {
        return Ok(None);
    };
    let launcher = managed_launcher(&root)?;
    let running = fs::canonicalize(std::env::current_exe()?)?;
    if running.parent() != Some(fs::canonicalize(root.join("Contents/MacOS"))?.as_path()) {
        return Ok(None);
    }
    Ok(Some(launcher))
}

struct Launchctl {
    domain: String,
}

impl Launchctl {
    fn run(&self, verb: &str, domain: &str) -> io::Result<std::process::Output> {
        Command::new("/bin/launchctl")
            .args([verb, domain])
            .stdin(Stdio::null())
            .output()
    }

    fn change(&self, verb: &str) -> io::Result<()> {
        let target = format!("{}/{LABEL}", self.domain);
        if self.run(verb, &target)?.status.success() {
            return Ok(());
        }
        Err(io::Error::other("macOS did not change startup permission"))
    }
}

trait Service {
    fn disabled(&self) -> io::Result<bool>;
    fn enable(&self) -> io::Result<()>;
    fn disable(&self) -> io::Result<()>;
}

impl Service for Launchctl {
    fn disabled(&self) -> io::Result<bool> {
        let output = self.run("print-disabled", &self.domain)?;
        if !output.status.success() || output.stdout.len() > 1024 * 1024 {
            return Err(io::Error::other("macOS login launch domain is unavailable"));
        }
        let text = std::str::from_utf8(&output.stdout).map_err(io::Error::other)?;
        parse_disabled(text)
    }

    fn enable(&self) -> io::Result<()> {
        self.change("enable")
    }
    fn disable(&self) -> io::Result<()> {
        self.change("disable")
    }
}

fn parse_disabled(text: &str) -> io::Result<bool> {
    const PREFIX: &str = "disabled services = {";
    let mut lines = text.trim().lines();
    if lines.next().map(str::trim) != Some(PREFIX) || lines.next_back().map(str::trim) != Some("}")
    {
        return Err(io::Error::other("Unknown macOS login registration state"));
    }
    let expected = format!("\"{LABEL}\"");
    let mut disabled = None;
    for line in lines {
        if let Some((label, state)) = line.split_once("=>")
            && label.trim() == expected
        {
            if disabled.is_some() {
                return Err(io::Error::other("Ambiguous macOS login registration state"));
            }
            disabled = match state.trim() {
                "disabled" | "true" => Some(true),
                "enabled" | "false" => Some(false),
                _ => return Err(io::Error::other("Unknown macOS login permission status")),
            };
        }
    }
    Ok(disabled.unwrap_or(false))
}

struct Registration<'a, S> {
    path: PathBuf,
    launcher: Option<PathBuf>,
    service: &'a S,
}

impl<'a, S: Service> Registration<'a, S> {
    fn new(home: &Path, launcher: Option<PathBuf>, service: &'a S) -> io::Result<Self> {
        let directory = home.join("Library/LaunchAgents");
        workspace_service::application_paths::validate_directory_path(&directory)?;
        Ok(Self {
            path: directory.join(format!("{LABEL}.plist")),
            launcher,
            service,
        })
    }

    fn document(&self) -> io::Result<Option<String>> {
        self.launcher.as_deref().map(plist).transpose()
    }

    fn read(&self) -> io::Result<Option<Vec<u8>>> {
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.len() > MAX_ENTRY_BYTES
            || metadata.uid() != uid()
        {
            return Err(io::Error::other("Invalid Code-Codex login entry"));
        }
        let mut contents = Vec::new();
        file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut contents)?;
        if contents.len() as u64 > MAX_ENTRY_BYTES {
            return Err(io::Error::other(
                "Code-Codex login entry exceeds its size limit",
            ));
        }
        Ok(Some(contents))
    }

    fn status(&self) -> io::Result<Status> {
        let document = self.document()?;
        match (self.read()?, document) {
            (None, expected) => Ok(Status {
                supported: expected.is_some(),
                enabled: false,
            }),
            (Some(data), Some(expected)) if data == expected.as_bytes() => Ok(Status {
                supported: true,
                enabled: !self.service.disabled()?,
            }),
            _ => Err(io::Error::other(
                "Login entry is stale; disable it before registering this installation",
            )),
        }
    }

    fn set(&self, enabled: bool) -> io::Result<Status> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("Missing LaunchAgents directory"))?;
        let lock = directory
            .parent()
            .ok_or_else(|| io::Error::other("Missing Library directory"))?
            .join("Application Support/Code-Codex/login-registration.lock");
        let _guard = crate::startup_conflict::macos_user_lock(&lock)?;
        if !enabled {
            if fs::symlink_metadata(&self.path)
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            {
                return Ok(Status {
                    supported: self.launcher.is_some(),
                    enabled: false,
                });
            }
            self.service.disable()?;
            if !self.service.disabled()? {
                return Err(io::Error::other(
                    "macOS login startup permission could not be revoked",
                ));
            }
            // An already-running login launch is left alone. The job is disabled
            // and its only on-disk registration is removed before the next login.
            match fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            return Ok(Status {
                supported: self.launcher.is_some(),
                enabled: false,
            });
        }

        let expected = self
            .document()?
            .ok_or_else(|| io::Error::other("Login startup requires this managed installation"))?;
        let existing = self.read()?;
        if existing
            .as_deref()
            .is_some_and(|old| old != expected.as_bytes())
        {
            return Err(io::Error::other(
                "Existing login entry differs; disable it before enabling this installation",
            ));
        }
        let staged = if existing.is_none() {
            workspace_service::application_paths::validate_directory_path(directory)?;
            fs::create_dir_all(directory)?;
            let mut staged = tempfile::NamedTempFile::new_in(directory)?;
            staged.write_all(expected.as_bytes())?;
            staged.as_file().sync_all()?;
            Some(staged)
        } else {
            None
        };
        self.service.enable()?;
        if self.service.disabled()? {
            return Err(io::Error::other("macOS did not enable the login entry"));
        }
        if let Some(staged) = staged {
            staged
                .persist_noclobber(&self.path)
                .map_err(|error| error.error)?;
        }
        let status = self.status()?;
        if !status.enabled {
            return Err(io::Error::other(
                "macOS login startup registration could not be verified",
            ));
        }
        Ok(status)
    }
}

fn plist(launcher: &Path) -> io::Result<String> {
    let path = launcher
        .to_str()
        .filter(|text| launcher.is_absolute() && !text.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("Invalid login launcher path"))?;
    let path = path
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{path}</string><string>run</string><string>--login-startup</string></array>
<key>RunAtLoad</key><true/>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>AbandonProcessGroup</key><true/>
</dict></plist>
"#
    ))
}

fn uid() -> libc::uid_t {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn user_home() -> io::Result<PathBuf> {
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = [0; 32768];
    let mut result = std::ptr::null_mut();
    // SAFETY: passwd and buffer are exclusive outputs of the declared sizes.
    let error = unsafe {
        libc::getpwuid_r(
            uid(),
            record.as_mut_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error));
    }
    if result.is_null() {
        return Err(io::Error::other("Unable to locate the current user's home"));
    }
    // SAFETY: a successful non-null result points to the initialized passwd.
    let home = unsafe { (*result).pw_dir };
    if home.is_null() {
        return Err(io::Error::other("The current user has no home directory"));
    }
    // SAFETY: passwd.pw_dir is a NUL-terminated string owned by buffer.
    let path = unsafe { CStr::from_ptr(home) }.to_bytes();
    fs::canonicalize(Path::new(OsStr::from_bytes(path)))
}

pub(super) fn status() -> io::Result<Status> {
    let service = Launchctl {
        domain: format!("gui/{}", uid()),
    };
    Registration::new(&user_home()?, installed_launcher()?, &service)?.status()
}

pub(super) fn set(enabled: bool) -> io::Result<Status> {
    let service = Launchctl {
        domain: format!("gui/{}", uid()),
    };
    let launcher = if enabled {
        installed_launcher()?
    } else {
        installed_launcher().ok().flatten()
    };
    Registration::new(&user_home()?, launcher, &service)?.set(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct FixtureService {
        disabled: Cell<bool>,
        enables: Cell<u32>,
    }

    impl Service for FixtureService {
        fn disabled(&self) -> io::Result<bool> {
            Ok(self.disabled.get())
        }
        fn enable(&self) -> io::Result<()> {
            self.enables.set(self.enables.get() + 1);
            self.disabled.set(false);
            Ok(())
        }
        fn disable(&self) -> io::Result<()> {
            self.disabled.set(true);
            Ok(())
        }
    }

    #[test]
    fn macos_status_reports_os_revocation_without_restoring_permission() {
        let home = tempfile::tempdir().unwrap();
        let launcher = home
            .path()
            .join("Code & Codex 中文.app/Contents/MacOS/Code-Codex");
        let service = FixtureService::default();
        let entry = Registration::new(home.path(), Some(launcher), &service).unwrap();
        assert!(!entry.status().unwrap().enabled);
        assert!(!home.path().join("Library").exists());
        assert!(entry.set(true).unwrap().enabled);
        assert_eq!(service.enables.get(), 1);
        service.disabled.set(true);
        assert!(!entry.status().unwrap().enabled);
        assert_eq!(service.enables.get(), 1);
        assert!(entry.path.is_file());
        assert!(!entry.set(false).unwrap().enabled);
        assert!(!entry.path.exists());
        assert!(service.disabled.get());
    }

    #[test]
    fn macos_login_plist_preserves_spaces_unicode_and_xml_characters_as_one_argument() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root
            .path()
            .join("Code & Codex <中文>.app/Contents/MacOS/Code-Codex");
        let entry = root.path().join("fixture.plist");
        fs::write(&entry, plist(&launcher).unwrap()).unwrap();
        let output = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--"])
            .arg(entry)
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["ProgramArguments"],
            serde_json::json!([launcher.to_str().unwrap(), "run", "--login-startup"])
        );
        assert_eq!(value["Label"], LABEL);
        assert_eq!(value["RunAtLoad"], true);
    }

    #[test]
    fn macos_login_state_parser_scopes_permission_to_the_exact_service() {
        let state = |body: &str| format!("disabled services = {{\n{body}\n}}");
        assert!(!parse_disabled(&state(r#""org.code-codex.other" => disabled"#)).unwrap());
        assert!(parse_disabled(&state(&format!(r#""{LABEL}" => disabled"#))).unwrap());
        assert!(!parse_disabled(&state(&format!(r#""{LABEL}" => enabled"#))).unwrap());
        assert!(parse_disabled(&state(&format!(r#""{LABEL}" => unknown"#))).is_err());
    }

    #[test]
    fn macos_revoke_removes_only_the_product_entry_even_if_installation_moved() {
        let home = tempfile::tempdir().unwrap();
        let service = FixtureService::default();
        let entry =
            Registration::new(home.path(), Some(home.path().join("old")), &service).unwrap();
        entry.set(true).unwrap();
        let other_entry = entry.path.with_file_name("com.openai.codex.plist");
        fs::write(&other_entry, b"untouched").unwrap();
        let moved =
            Registration::new(home.path(), Some(home.path().join("new")), &service).unwrap();
        assert!(moved.status().is_err());
        assert!(moved.set(true).is_err());
        moved.set(false).unwrap();
        assert_eq!(fs::read(other_entry).unwrap(), b"untouched");
        assert!(!moved.path.exists());
    }
}
