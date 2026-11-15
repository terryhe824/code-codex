//! Local macOS installation lifecycle. Online release trust is configured separately.
use semver::Version;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const PRODUCT_ID: &str = "io.github.rice-dog.code-codex.macos";
const INSTALL_NAME: &str = "Code-Codex.app";
const CORE: &str = "Contents/MacOS/code-codex-core";
const LAUNCHER: &str = "Contents/MacOS/Code-Codex";
const PURGE_CONFIRMATION: &str = "DELETE CODE-CODEX DATA";
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BUNDLE_ENTRIES: usize = 4096;

#[derive(Deserialize)]
struct BundleMetadata {
    #[serde(rename = "CFBundleIdentifier")]
    identifier: String,
    #[serde(rename = "CFBundleExecutable")]
    executable: String,
    #[serde(rename = "CFBundlePackageType")]
    package_type: String,
    #[serde(rename = "CFBundleShortVersionString")]
    version: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn data_paths(home: &Path) -> [PathBuf; 3] {
    ["Application Support", "Logs", "Caches"]
        .map(|role| home.join("Library").join(role).join("Code-Codex"))
}

fn user_home() -> io::Result<PathBuf> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = [0; 32768];
    let mut result = std::ptr::null_mut();
    // SAFETY: passwd, result and buffer are writable for the duration of the call.
    let status = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    if result.is_null() {
        return Err(invalid("The current user home is unavailable"));
    }
    // SAFETY: a successful result points to passwd initialized by getpwuid_r.
    let home = unsafe { (*result).pw_dir };
    if home.is_null() {
        return Err(invalid("The current user home is unavailable"));
    }
    // SAFETY: passwd's home field is a NUL-terminated string in buffer.
    let home = unsafe { std::ffi::CStr::from_ptr(home) };
    fs::canonicalize(Path::new(OsStr::from_bytes(home.to_bytes())))
}

fn destination_path(explicit: Option<&Path>, use_running_bundle: bool) -> io::Result<PathBuf> {
    if let Some(path) = explicit {
        return std::path::absolute(path);
    }
    if use_running_bundle && let Some(root) = workspace_service::application_paths::install_root()?
    {
        return Ok(root);
    }
    Ok(user_home()?.join("Applications").join(INSTALL_NAME))
}

fn validate_bundle_path(bundle: &Path) -> io::Result<()> {
    if bundle.extension().and_then(OsStr::to_str) != Some("app")
        || bundle
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid(
            "Expected an absolute, non-redirected .app bundle path",
        ));
    }
    workspace_service::application_paths::validate_directory_path(bundle)
}

fn local_parent(destination: &Path, create: bool) -> io::Result<&Path> {
    validate_bundle_path(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("Missing installation directory"))?;
    if create {
        fs::create_dir_all(parent)?;
    }
    workspace_service::application_paths::validate_directory_path(parent)?;
    let metadata = fs::symlink_metadata(parent)?;
    // SAFETY: geteuid has no preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
        return Err(invalid(
            "The installation directory must be writable only by the current user",
        ));
    }
    let path = CString::new(parent.as_os_str().as_bytes())?;
    let mut status = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: path is NUL-terminated and statfs receives one writable record.
    if unsafe { libc::statfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful statfs populated the record.
    if unsafe { status.assume_init() }.f_flags & libc::MNT_LOCAL as u32 == 0 {
        return Err(invalid(
            "The installation directory must be on a local volume",
        ));
    }
    Ok(parent)
}

fn previous_path(destination: &Path) -> io::Result<PathBuf> {
    let name = destination
        .file_stem()
        .ok_or_else(|| invalid("Missing app name"))?;
    let mut previous = OsString::from(".");
    previous.push(name);
    previous.push(".previous.app");
    Ok(destination.with_file_name(previous))
}

fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn regular_file(path: &Path) -> io::Result<File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(invalid("Bundle contains a redirected or non-regular file"));
    }
    Ok(file)
}

fn bundle_entries(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        if files.len() >= MAX_BUNDLE_ENTRIES || relative.components().count() > 16 {
            return Err(invalid("Bundle exceeds its directory limits"));
        }
        let kind = entry.file_type()?;
        files.push(relative.to_path_buf());
        if kind.is_dir() {
            bundle_entries(root, &path, files)?;
        } else if !kind.is_file() {
            return Err(invalid("Bundle contains a symlink or unsupported file"));
        }
    }
    Ok(())
}

pub(crate) fn fingerprint(root: &Path) -> io::Result<String> {
    validate_bundle_path(root)?;
    let mut entries = Vec::new();
    bundle_entries(root, root, &mut entries)?;
    entries.sort_unstable();
    let mut size = 0_u64;
    let mut digest = Sha256::new();
    digest.update(b"Code-Codex macOS bundle v1\0");
    for entry in entries {
        let name = entry.as_os_str().as_bytes();
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name);
        let path = root.join(entry);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            digest.update(b"D");
            continue;
        }
        let file = regular_file(&path)?;
        let metadata = file.metadata()?;
        let length = metadata.len();
        size = size
            .checked_add(length)
            .filter(|size| *size <= MAX_BUNDLE_BYTES)
            .ok_or_else(|| invalid("Bundle exceeds its size limit"))?;
        digest.update(b"F");
        digest.update((metadata.mode() & 0o111).to_be_bytes());
        digest.update(length.to_be_bytes());
        let actual = io::copy(&mut file.take(length + 1), &mut digest)?;
        if actual != length {
            return Err(invalid("Bundle changed while its digest was calculated"));
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_arm64(path: &Path) -> io::Result<()> {
    let mut file = regular_file(path)?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header)?;
    if header[..8] != [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]
        || header[12..16] != [2, 0, 0, 0]
        || file.metadata()?.permissions().mode() & 0o111 == 0
    {
        return Err(invalid(
            "Code-Codex requires native ARM64 Mach-O executables",
        ));
    }
    Ok(())
}

fn verify_bundle(root: &Path, allow_ad_hoc: bool) -> io::Result<BundleMetadata> {
    validate_bundle_path(root)?;
    workspace_service::application_paths::validate_install_root(root)?;
    workspace_service::application_paths::validate_directory_path(&root.join("Contents/MacOS"))?;
    let info = root.join("Contents/Info.plist");
    if regular_file(&info)?.metadata()?.len() > 32768 {
        return Err(invalid("Bundle metadata exceeds its size limit"));
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(&info)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err(invalid("Code-Codex bundle metadata is invalid"));
    }
    let metadata: BundleMetadata = serde_json::from_slice(&output.stdout)?;
    let version = Version::parse(&metadata.version).map_err(io::Error::other)?;
    if metadata.identifier != PRODUCT_ID
        || metadata.package_type != "APPL"
        || metadata.executable != "Code-Codex"
        || !version.pre.is_empty()
        || !version.build.is_empty()
    {
        return Err(invalid(
            "Bundle identity, version, or main executable is invalid",
        ));
    }
    verify_arm64(&root.join(LAUNCHER))?;
    verify_arm64(&root.join(CORE))?;
    if !allow_ad_hoc {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The trusted macOS release signing identity is not configured. Use --allow-ad-hoc only with a trusted local development build.",
        ));
    }
    let requirement = format!("-R=identifier \"{PRODUCT_ID}\"");
    let checked = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(requirement)
        .arg(root)
        .stdin(Stdio::null())
        .output()?;
    if !checked.status.success() {
        return Err(invalid("The bundle signature failed verification"));
    }
    for binary in [root.to_path_buf(), root.join(CORE)] {
        let output = Command::new("/usr/bin/codesign")
            .args(["-d", "--verbose=2"])
            .arg(binary)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success()
            || !String::from_utf8_lossy(&output.stderr)
                .lines()
                .any(|line| line == "Signature=adhoc")
        {
            return Err(invalid(
                "Local development mode requires ad-hoc signed Code-Codex components",
            ));
        }
    }
    Ok(metadata)
}

fn rename(from: &Path, to: &Path, mode: libc::c_uint) -> io::Result<()> {
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both absolute paths are NUL-terminated for Darwin renamex_np.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), mode) } == 0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
}

fn replace_current(
    staged: &Path,
    current: &Path,
    previous: &Path,
    has_previous: bool,
) -> io::Result<()> {
    let mode = if has_previous {
        libc::RENAME_SWAP
    } else {
        libc::RENAME_EXCL
    };
    // Put the verified candidate at the backup path first. One atomic swap
    // publishes it and saves the current version together.
    rename(staged, previous, mode)?;
    if let Err(switch_error) = rename(previous, current, libc::RENAME_SWAP) {
        rename(previous, staged, mode).map_err(|restore_error| {
            io::Error::other(format!(
                "Could not switch the installation ({switch_error}) or restore its staging state ({restore_error})"
            ))
        })?;
        return Err(switch_error);
    }
    Ok(())
}

fn check_idle(bundle: &Path) -> io::Result<()> {
    for name in [LAUNCHER, CORE] {
        let path = bundle.join(name);
        if crate::process_guard::is_executable_running_except(&path, std::process::id())
            .map_err(io::Error::other)?
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Quit the running Code-Codex application before changing this installation",
            ));
        }
    }
    Ok(())
}

fn lock_installation(parent: &Path) -> io::Result<File> {
    crate::startup_conflict::macos_user_lock(&parent.join(".code-codex-installation.lock"))
}

pub(crate) fn install(
    source: &Path,
    destination: Option<&Path>,
    expected_sha256: Option<&str>,
    allow_ad_hoc: bool,
) -> io::Result<Value> {
    let source = std::path::absolute(source)?;
    let destination = destination_path(destination, false)?;
    let source_digest = fingerprint(&source)?;
    if expected_sha256.is_some_and(|expected| expected != source_digest) {
        return Err(invalid("The Code-Codex bundle SHA-256 does not match"));
    }
    let metadata = verify_bundle(&source, allow_ad_hoc)?;
    let parent = local_parent(&destination, true)?;
    let _lock = lock_installation(parent)?;
    let previous = previous_path(&destination)?;
    let has_current = exists(&destination)?;
    let has_previous = exists(&previous)?;
    if has_current {
        verify_bundle(&destination, allow_ad_hoc)?;
        check_idle(&destination)?;
    }
    if has_previous {
        verify_bundle(&previous, allow_ad_hoc)?;
        check_idle(&previous)?;
    }
    if source == destination
        || source.starts_with(&destination)
        || destination.starts_with(&source)
        || source == previous
        || source.starts_with(&previous)
    {
        return Err(invalid(
            "Use a separate source bundle for local installation",
        ));
    }
    let staging = tempfile::Builder::new()
        .prefix(".code-codex-install-")
        .tempdir_in(parent)?;
    let staged = staging.path().join(INSTALL_NAME);
    let copied = Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(&staged)
        .stdin(Stdio::null())
        .output()?;
    if !copied.status.success() {
        return Err(io::Error::other("Could not stage the Code-Codex bundle"));
    }
    if fingerprint(&staged)? != source_digest {
        return Err(invalid(
            "The staged bundle differs from the verified source",
        ));
    }
    verify_bundle(&staged, allow_ad_hoc)?;
    if has_current {
        if let Err(error) = replace_current(&staged, &destination, &previous, has_previous) {
            if has_previous && !fingerprint(&staged).is_ok_and(|digest| digest == source_digest) {
                let recovery = staging.keep();
                return Err(io::Error::other(format!(
                    "{error}. The prior bundle remains at {}",
                    recovery.display()
                )));
            }
            return Err(error);
        }
    } else {
        rename(&staged, &destination, libc::RENAME_EXCL)?;
    }
    Ok(
        json!({"installed":true,"version":metadata.version,"previousAvailable":has_current||has_previous}),
    )
}

pub(crate) fn rollback(destination: Option<&Path>, allow_ad_hoc: bool) -> io::Result<Value> {
    let destination = destination_path(destination, true)?;
    let parent = local_parent(&destination, false)?;
    let _lock = lock_installation(parent)?;
    let previous = previous_path(&destination)?;
    verify_bundle(&destination, allow_ad_hoc)?;
    let prior = verify_bundle(&previous, allow_ad_hoc)?;
    check_idle(&destination)?;
    check_idle(&previous)?;
    rename(&destination, &previous, libc::RENAME_SWAP)?;
    Ok(json!({"rolledBack":true,"version":prior.version,"previousAvailable":true}))
}

pub(crate) fn uninstall(
    destination: Option<&Path>,
    purge_confirmation: Option<&str>,
    allow_ad_hoc: bool,
) -> io::Result<Value> {
    let destination = destination_path(destination, true)?;
    uninstall_at(
        &destination,
        &user_home()?,
        purge_confirmation,
        allow_ad_hoc,
        || crate::login_startup::set(false).map(|_| ()),
        crate::accounts::purge_managed_credentials,
    )
}

fn uninstall_at(
    destination: &Path,
    home: &Path,
    purge_confirmation: Option<&str>,
    allow_ad_hoc: bool,
    revoke_startup: impl FnOnce() -> io::Result<()>,
    purge_vault: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<Value> {
    let purge = match purge_confirmation {
        None => false,
        Some(PURGE_CONFIRMATION) => true,
        Some(_) => {
            return Err(invalid(
                "Data removal requires the exact phrase DELETE CODE-CODEX DATA",
            ));
        }
    };
    let parent = local_parent(destination, false)?;
    let _lock = lock_installation(parent)?;
    let previous = previous_path(destination)?;
    verify_bundle(destination, allow_ad_hoc)?;
    check_idle(destination)?;
    let has_previous = exists(&previous)?;
    if has_previous {
        verify_bundle(&previous, allow_ad_hoc)?;
        check_idle(&previous)?;
    }
    let paths = data_paths(home);
    if purge {
        for path in &paths {
            workspace_service::application_paths::validate_directory_path(path)?;
            if destination.starts_with(path) || previous.starts_with(path) {
                return Err(invalid(
                    "An installation cannot be purged from inside the product data directory",
                ));
            }
        }
    }
    // Retire both verified bundles before any irreversible credential deletion.
    let removed = tempfile::Builder::new()
        .prefix(".code-codex-remove-")
        .tempdir_in(parent)?;
    let bundles = [
        (destination, removed.path().join(INSTALL_NAME)),
        (previous.as_path(), removed.path().join("previous.app")),
    ];
    let bundles = &bundles[..1 + usize::from(has_previous)];
    let mut staged = 0;
    let outcome = (|| {
        for (original, retired) in bundles {
            rename(original, retired, libc::RENAME_EXCL)?;
            staged += 1;
        }
        revoke_startup()?;
        if purge {
            purge_vault(&paths[0])?;
            for path in paths {
                if exists(&path)? {
                    workspace_service::application_paths::validate_directory_path(&path)?;
                    fs::remove_dir_all(path)?;
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut failures = Vec::new();
        for (original, retired) in bundles[..staged].iter().rev() {
            if let Err(restore_error) = rename(retired, original, libc::RENAME_EXCL) {
                failures.push(format!(
                    "could not restore {} ({restore_error}); bundle remains at {}",
                    original.display(),
                    retired.display()
                ));
            }
        }
        if failures.is_empty() {
            return Err(error);
        }
        let _ = removed.keep();
        return Err(io::Error::other(format!(
            "{error}; {}",
            failures.join("; ")
        )));
    }
    removed.close()?;
    Ok(json!({"uninstalled":true,"dataPurged":purge}))
}

pub(crate) fn uninstall_dialog(destination: &Path) -> io::Result<Value> {
    let current = workspace_service::application_paths::install_root()?
        .ok_or_else(|| invalid("The uninstall dialog requires an installed Code-Codex bundle"))?;
    validate_bundle_path(destination)?;
    if fs::canonicalize(destination)? != current {
        return Err(invalid(
            "The uninstall dialog does not own this application",
        ));
    }
    verify_bundle(&current, true)?;
    const SCRIPT: &str = r#"
on run
    try
        set result to display dialog "Remove Code-Codex? The official Codex app is preserved. After choosing an option, quit Code-Codex within 60 seconds to complete removal." with title "Uninstall Code-Codex" buttons {"Cancel", "Remove all data…", "Keep data"} default button "Cancel" cancel button "Cancel" giving up after 60
        if gave up of result then return "cancel"
        if button returned of result is "Keep data" then return "keep"
        set confirmation to display dialog "Type DELETE CODE-CODEX DATA to permanently delete Code-Codex settings, logs, caches and saved connections." with title "Remove Code-Codex data" default answer "" buttons {"Cancel", "Remove"} default button "Cancel" cancel button "Cancel" giving up after 60
        if gave up of confirmation then return "cancel"
        if text returned of confirmation is "DELETE CODE-CODEX DATA" then return "purge"
    on error number -128
        return "cancel"
    end try
    return "cancel"
end run
"#;
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", SCRIPT])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            "macOS could not open the Code-Codex uninstall dialog",
        ));
    }
    let confirm = match String::from_utf8_lossy(&output.stdout).trim() {
        "keep" => None,
        "purge" => Some(PURGE_CONFIRMATION),
        "cancel" => return Ok(json!({"uninstalled":false,"cancelled":true})),
        _ => return Err(invalid("Unrecognized uninstall confirmation")),
    };
    let outcome = (|| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            match check_idle(&current) {
                Ok(()) => break,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(350));
                }
                Err(error) => return Err(error),
            }
        }
        uninstall(Some(&current), confirm, true)
    })();
    if let Err(error) = &outcome {
        let _ = Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv\n display alert \"Code-Codex removal stopped\" message (item 1 of argv) giving up after 30\nend run", "--"])
            .arg(error.to_string())
            .stdin(Stdio::null()).output();
    }
    outcome
}

pub(crate) fn uninstaller_command(bundle: &Path, core: &Path) -> io::Result<Command> {
    validate_installed_core(bundle, core)?;
    let mut command = Command::new(core);
    command
        .arg("mac-uninstall-dialog")
        .arg("--destination")
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

pub(crate) fn validate_installed_core(bundle: &Path, core: &Path) -> io::Result<()> {
    verify_bundle(bundle, true)?;
    if fs::canonicalize(core)? != bundle.join(CORE) {
        return Err(invalid(
            "The uninstall helper must belong to the installed Code-Codex bundle",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, version: &str, product: &str) -> PathBuf {
        let app = root.join(format!("样本 {version}.app"));
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        let source = root.join("synthetic-arm64");
        if !source.exists() {
            let code = root.join("fixture.c");
            fs::write(&code, "int main(void) { return 0; }").unwrap();
            assert!(
                Command::new("/usr/bin/clang")
                    .args(["-arch", "arm64", "-o"])
                    .arg(&source)
                    .arg(code)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        for name in ["Code-Codex", "code-codex-core"] {
            fs::copy(&source, app.join("Contents/MacOS").join(name)).unwrap();
        }
        fs::write(
            app.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{product}</string>
<key>CFBundleExecutable</key><string>Code-Codex</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>"#
            ),
        )
        .unwrap();
        fs::write(
            app.join("Contents/Resources/storage-layout.json"),
            br#"{"schema":1,"product":"Code-Codex"}"#,
        )
        .unwrap();
        assert!(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .output()
                .unwrap()
                .status
                .success()
        );
        app
    }

    #[test]
    fn mac_installation_validates_stages_replaces_rolls_back_and_preserves_data() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let first = fixture(temp.path(), "1.0.0", "io.github.rice-dog.code-codex.macos");
        let second = fixture(temp.path(), "1.1.0", "io.github.rice-dog.code-codex.macos");
        let destination = temp.path().join("应用 安装/Code-Codex.app");
        let home = temp.path().join("user home");
        let data = home.join("Library/Application Support/Code-Codex");
        let official = home.join("Library/Application Support/Codex");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&official).unwrap();
        fs::write(data.join("fixture"), "preserve").unwrap();
        fs::write(official.join("fixture"), "leave untouched").unwrap();
        let digest = fingerprint(&first).unwrap();
        assert!(install(&first, Some(&destination), Some(&"0".repeat(64)), true).is_err());
        assert!(!destination.exists());
        assert!(install(&first, Some(&destination), Some(&digest), false).is_err());
        assert_eq!(
            install(&first, Some(&destination), Some(&digest), true).unwrap()["version"],
            "1.0.0"
        );
        assert_eq!(
            install(&second, Some(&destination), None, true).unwrap()["version"],
            "1.1.0"
        );
        assert_eq!(
            rollback(Some(&destination), true).unwrap()["version"],
            "1.0.0"
        );
        uninstall_at(&destination, &home, None, true, || Ok(()), |_| Ok(())).unwrap();
        assert!(!destination.exists());
        assert_eq!(
            fs::read_to_string(data.join("fixture")).unwrap(),
            "preserve"
        );
        install(&first, Some(&destination), None, true).unwrap();
        assert!(
            uninstall_at(
                &destination,
                &home,
                Some("yes"),
                true,
                || Ok(()),
                |_| Ok(())
            )
            .is_err()
        );
        assert!(destination.exists());
        uninstall_at(
            &destination,
            &home,
            Some("DELETE CODE-CODEX DATA"),
            true,
            || Ok(()),
            |_| Ok(()),
        )
        .unwrap();
        assert!(!data.exists());
        assert_eq!(
            fs::read_to_string(official.join("fixture")).unwrap(),
            "leave untouched"
        );
    }

    #[test]
    fn mac_installation_preserves_credentials_when_login_revocation_fails() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let destination = fixture(temp.path(), "1.0.0", PRODUCT_ID);
        let home = temp.path().join("synthetic home");
        let credential = temp.path().join("synthetic credential");
        fs::write(&credential, "preserve until login revocation succeeds").unwrap();

        let error = uninstall_at(
            &destination,
            &home,
            Some(PURGE_CONFIRMATION),
            true,
            || Err(io::Error::other("Fixture login revocation failed")),
            |_| fs::remove_file(&credential),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "Fixture login revocation failed");
        assert!(destination.is_dir());
        assert!(credential.is_file(), "a failed removal deleted credentials");
    }

    #[test]
    fn mac_installation_preserves_credentials_when_either_bundle_cannot_be_retired() {
        let mut preserved = Vec::new();
        for immutable_current in [true, false] {
            let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
            let destination = fixture(temp.path(), "1.1.0", PRODUCT_ID);
            let backup = previous_path(&destination).unwrap();
            fs::rename(fixture(temp.path(), "1.0.0", PRODUCT_ID), &backup).unwrap();
            let immutable = if immutable_current {
                &destination
            } else {
                &backup
            };
            let home = temp.path().join("synthetic home");
            let credential = temp.path().join("synthetic credential");
            fs::write(&credential, "preserve until both bundles are staged").unwrap();
            let set_flag = |flag: &str| {
                assert!(
                    Command::new("/usr/bin/chflags")
                        .arg(flag)
                        .arg(immutable)
                        .status()
                        .unwrap()
                        .success()
                );
            };
            set_flag("uchg");
            let result = uninstall_at(
                &destination,
                &home,
                Some(PURGE_CONFIRMATION),
                true,
                || Ok(()),
                |_| fs::remove_file(&credential),
            );
            set_flag("nouchg");

            assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EPERM));
            preserved.push(credential.is_file());
            assert_eq!(verify_bundle(&destination, true).unwrap().version, "1.1.0");
            assert_eq!(verify_bundle(&backup, true).unwrap().version, "1.0.0");
        }
        assert_eq!(
            preserved,
            [true, true],
            "a failed retirement deleted credentials"
        );
    }

    #[test]
    fn mac_installation_restores_both_verified_bundles_when_vault_purge_fails() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let destination = fixture(temp.path(), "1.1.0", PRODUCT_ID);
        let backup = previous_path(&destination).unwrap();
        fs::rename(fixture(temp.path(), "1.0.0", PRODUCT_ID), &backup).unwrap();
        let home = temp.path().join("synthetic home");
        let data = data_paths(&home)[0].join("fixture.txt");
        fs::create_dir_all(data.parent().unwrap()).unwrap();
        fs::write(&data, "preserve").unwrap();

        let result = uninstall_at(
            &destination,
            &home,
            Some(PURGE_CONFIRMATION),
            true,
            || Ok(()),
            |_| {
                assert!(!destination.exists());
                assert!(!backup.exists());
                Err(io::Error::other("Fixture vault purge failed"))
            },
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            "Fixture vault purge failed"
        );
        assert_eq!(fs::read_to_string(data).unwrap(), "preserve");
        assert_eq!(verify_bundle(&destination, true).unwrap().version, "1.1.0");
        assert_eq!(verify_bundle(&backup, true).unwrap().version, "1.0.0");
    }

    #[test]
    fn mac_installation_keeps_signed_recovery_copy_if_restore_is_blocked() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let destination = fixture(temp.path(), "1.1.0", PRODUCT_ID);
        let backup = previous_path(&destination).unwrap();
        fs::rename(fixture(temp.path(), "1.0.0", PRODUCT_ID), &backup).unwrap();
        let home = temp.path().join("synthetic home");

        let result = uninstall_at(
            &destination,
            &home,
            Some(PURGE_CONFIRMATION),
            true,
            || Ok(()),
            |_| {
                fs::create_dir(&destination)?;
                Err(io::Error::other("Fixture vault purge failed"))
            },
        );

        let diagnostic = result.unwrap_err().to_string();
        let recovery = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".code-codex-remove-")
            })
            .expect("signed recovery directory");
        let staged = recovery.join(INSTALL_NAME);
        assert!(diagnostic.contains(&staged.display().to_string()));
        assert!(diagnostic.contains("Fixture vault purge failed"));
        assert_eq!(verify_bundle(&staged, true).unwrap().version, "1.1.0");
        assert_eq!(verify_bundle(&backup, true).unwrap().version, "1.0.0");
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    }

    #[test]
    fn mac_installation_refuses_wrong_products_architectures_and_redirected_targets() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let wrong = fixture(temp.path(), "1.0.0", "com.openai.codex");
        let valid = fixture(temp.path(), "1.1.0", "io.github.rice-dog.code-codex.macos");
        let destination = temp.path().join("Code-Codex.app");
        assert!(install(&wrong, Some(&destination), None, true).is_err());
        let helper = uninstaller_command(&valid, &valid.join(CORE)).unwrap();
        assert_eq!(helper.get_program(), valid.join(CORE));
        assert_eq!(
            helper.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("mac-uninstall-dialog"),
                OsStr::new("--destination"),
                valid.as_os_str()
            ]
        );
        assert!(uninstaller_command(&valid, &wrong.join(CORE)).is_err());
        std::os::unix::fs::symlink(temp.path().join("no application"), &destination).unwrap();
        assert!(install(&valid, Some(&destination), None, true).is_err());
        assert!(
            fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(&destination).unwrap();
        fs::write(valid.join("Contents/MacOS/code-codex-core"), b"MZ Windows").unwrap();
        assert!(install(&valid, Some(&destination), None, true).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn mac_installation_rolls_back_if_retaining_the_old_version_fails() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let staged = temp.path().join("staged");
        let current = temp.path().join("current");
        fs::write(&staged, "new").unwrap();
        fs::write(&current, "old").unwrap();
        assert!(
            replace_current(
                &staged,
                &current,
                &temp.path().join("missing/previous"),
                false
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(current).unwrap(), "old");
        assert_eq!(fs::read_to_string(staged).unwrap(), "new");
    }

    #[test]
    fn mac_installation_restores_previous_backup_when_the_atomic_switch_fails() {
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let staged = temp.path().join("staged");
        let missing_current = temp.path().join("missing/current");
        let previous = temp.path().join("previous");
        fs::write(&staged, "new").unwrap();
        fs::write(&previous, "prior backup").unwrap();
        assert!(replace_current(&staged, &missing_current, &previous, true).is_err());
        assert_eq!(fs::read_to_string(previous).unwrap(), "prior backup");
        assert_eq!(fs::read_to_string(staged).unwrap(), "new");
    }
}
