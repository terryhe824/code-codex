use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::{
    ChannelPreference, CodexInstallation, DiscoveryError, DiscoverySource, channel_matches,
    channel_rank, contained_file, version_key,
};

const STABLE_BUNDLE_ID: &str = "com.openai.codex";
const BETA_BUNDLE_ID: &str = "com.openai.codex.beta";
const CLI_BUNDLE: &str = "Contents/Resources/codex-cli/CodexCLI.app";
const CLI_ENTRY: &str = "Contents/MacOS/codex";
const MAX_PLIST_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct BundleMetadata {
    #[serde(rename = "CFBundleIdentifier")]
    identifier: String,
    #[serde(rename = "CFBundleExecutable")]
    executable: String,
    #[serde(rename = "CFBundleShortVersionString")]
    version: String,
    #[serde(rename = "CFBundlePackageType")]
    package_type: String,
}

pub(super) fn discover(preference: ChannelPreference) -> Result<CodexInstallation, DiscoveryError> {
    let mut candidates = vec![PathBuf::from("/Applications")];
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("Applications"));
    }
    let mut applications: Vec<_> = candidates
        .iter()
        .filter_map(|directory| fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_bundle_path(path))
        .collect();

    // Spotlight also finds renamed or moved bundles outside Applications.
    // A query hit supplies a path only. The metadata and signature are checked below.
    let query = format!(
        r#"kMDItemCFBundleIdentifier == "{STABLE_BUNDLE_ID}" || kMDItemCFBundleIdentifier == "{BETA_BUNDLE_ID}""#
    );
    let spotlight = Command::new("/usr/bin/mdfind")
        .args(["-0", &query])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Some(output) = spotlight.ok().filter(|output| output.status.success()) {
        applications.extend(
            output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
                .map(|entry| PathBuf::from(OsStr::from_bytes(entry))),
        );
    }
    discover_in_candidates(applications, preference, verify_official_bundle)
}

pub(super) fn is_bundle_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

pub(super) fn discover_bundle(
    bundle: &Path,
    preference: ChannelPreference,
) -> Result<CodexInstallation, DiscoveryError> {
    discover_in_candidates([bundle.to_path_buf()], preference, verify_official_bundle)
}

fn discover_in_candidates(
    candidates: impl IntoIterator<Item = PathBuf>,
    preference: ChannelPreference,
    verify: impl Fn(&Path, &str) -> bool,
) -> Result<CodexInstallation, DiscoveryError> {
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter(|path| path.is_dir() && is_bundle_path(path))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();

    let mut first_error = DiscoveryError::CodexNotFound;
    let mut installations = Vec::new();
    for bundle in candidates {
        let Some(metadata) = bundle_metadata(&bundle) else {
            continue;
        };
        let channel = match metadata.identifier.as_str() {
            STABLE_BUNDLE_ID => "stable",
            BETA_BUNDLE_ID => "beta",
            _ => continue,
        };
        if !channel_matches(channel, preference) {
            continue;
        }
        match bundle_installation(&bundle, metadata, channel, &verify) {
            Ok(installation) => installations.push(installation),
            Err(error) if matches!(first_error, DiscoveryError::CodexNotFound) => {
                first_error = error;
            }
            Err(_) => {}
        }
    }
    installations
        .into_iter()
        .min_by(|left, right| {
            channel_rank(&left.channel, preference)
                .cmp(&channel_rank(&right.channel, preference))
                .then_with(|| version_key(&right.version).cmp(&version_key(&left.version)))
        })
        .ok_or(first_error)
}

fn bundle_metadata(bundle: &Path) -> Option<BundleMetadata> {
    let plist = contained_file(bundle, &bundle.join("Contents/Info.plist"))?;
    if fs::metadata(&plist).ok()?.len() > MAX_PLIST_BYTES {
        return None;
    }
    // plutil handles XML and binary property lists without loading bundle code.
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(plist)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() as u64 > MAX_PLIST_BYTES {
        return None;
    }
    let metadata: BundleMetadata = serde_json::from_slice(&output.stdout).ok()?;
    (metadata.package_type == "APPL" && !metadata.version.trim().is_empty()).then_some(metadata)
}

fn bundle_installation(
    bundle: &Path,
    metadata: BundleMetadata,
    channel: &str,
    verify: &impl Fn(&Path, &str) -> bool,
) -> Result<CodexInstallation, DiscoveryError> {
    let name = Path::new(&metadata.executable);
    let executable = match (name.components().next(), name.components().count()) {
        (Some(Component::Normal(_)), 1) if name.file_name() == Some(name.as_os_str()) => {
            let directory = bundle.join("Contents/MacOS");
            executable_in(bundle, &directory.join(name))
                .filter(|executable| executable.parent() == Some(directory.as_path()))
        }
        _ => None,
    }
    .ok_or_else(|| {
        DiscoveryError::InvalidExecutable.context(
            "discover macOS application",
            "CFBundleExecutable has no usable file under Contents/MacOS",
        )
    })?;
    let archive = bundle.join("Contents/Resources/app.asar");
    if contained_file(bundle, &archive).as_deref() != Some(archive.as_path()) {
        return Err(DiscoveryError::InvalidExecutable.context(
            "discover macOS application",
            "Contents/Resources/app.asar is missing or redirected",
        ));
    }
    if !verify(bundle, &metadata.identifier) {
        return Err(DiscoveryError::InvalidExecutable.context(
            "discover macOS application",
            "the bundle did not satisfy the OpenAI Developer ID signing requirement",
        ));
    }

    Ok(CodexInstallation {
        executable,
        version: metadata.version,
        channel: channel.to_owned(),
        source: DiscoverySource::MacApplication,
        package_name: Some(metadata.identifier),
        package_full_name: None,
        app_user_model_id: None,
        app_server: embedded_cli(bundle, verify),
    })
}

fn executable_in(bundle: &Path, path: &Path) -> Option<PathBuf> {
    contained_file(bundle, path)
        .filter(|file| fs::metadata(file).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0))
}

fn embedded_cli(bundle: &Path, verify: &impl Fn(&Path, &str) -> bool) -> Option<PathBuf> {
    // codex-cli/bin/codex delegates to this signed executable. Resolve it
    // directly instead of executing a shell wrapper during discovery.
    let cli_bundle = bundle.join(CLI_BUNDLE);
    let expected = cli_bundle.join(CLI_ENTRY);
    let actual = executable_in(bundle, &expected)?;
    (actual == expected && verify(&cli_bundle, "codex")).then_some(actual)
}

pub(super) fn verify_app_server(
    bundle: &Path,
    identifier: &str,
    executable: &Path,
) -> Result<(), DiscoveryError> {
    if !matches!(identifier, STABLE_BUNDLE_ID | BETA_BUNDLE_ID)
        || !verify_official_bundle(bundle, identifier)
        || embedded_cli(bundle, &verify_official_bundle).as_deref() != Some(executable)
    {
        return Err(DiscoveryError::AppServerNotFound.context(
            "verify macOS App Server",
            "the embedded CLI could not be verified in the signed application",
        ));
    }
    Ok(())
}

fn verify_official_bundle(bundle: &Path, identifier: &str) -> bool {
    // Require Apple's Developer ID chain and the OpenAI team from the
    // official bundle's designated requirement, in addition to the code ID.
    let requirement = format!(
        r#"identifier "{identifier}" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = "2DC432GLL2""#
    );
    Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--deep"])
        .arg(format!("-R={requirement}"))
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::discover_in_candidates;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn fixture_bundle(directory: &Path, name: &str, identifier: &str, version: &str) -> PathBuf {
        let bundle = directory.join(name);
        fs::create_dir_all(bundle.join("Contents")).expect("fixture Contents");
        fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
    <key>CFBundleIdentifier</key><string>{identifier}</string>
    <key>CFBundleExecutable</key><string>Desktop Runner</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#
            ),
        )
        .expect("fixture plist");
        fixture_file(&bundle, "Contents/MacOS/Desktop Runner", true);
        fixture_file(&bundle, "Contents/Resources/app.asar", false);
        bundle
    }

    fn fixture_file(bundle: &Path, relative: &str, executable: bool) -> PathBuf {
        let file = bundle.join(relative);
        fs::create_dir_all(file.parent().expect("fixture file parent")).expect("fixture parent");
        fs::write(&file, b"fixture").expect("fixture file");
        if executable {
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755))
                .expect("fixture executable permissions");
        }
        file
    }

    // The platform verifier requires an OpenAI signature. Test metadata and layout
    // independently, and use the real verifier for the unsigned-bundle case.
    fn verified_fixture(_path: &Path, _identifier: &str) -> bool {
        true
    }

    #[test]
    fn mac_discovery_uses_bundle_metadata_and_nested_cli_under_renamed_unicode_path() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(
            directory.path(),
            "任意 名称.app",
            "com.openai.codex",
            "26.1111.21308",
        );
        fixture_file(&bundle, "Contents/Resources/codex-cli/bin/codex", true);
        let cli = fixture_file(
            &bundle,
            "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            true,
        );
        let installation = discover_in_candidates(
            [bundle.clone()],
            ChannelPreference::Stable,
            verified_fixture,
        )
        .expect("discover renamed stable app");
        assert_eq!(
            installation.executable,
            fs::canonicalize(bundle.join("Contents/MacOS/Desktop Runner"))
                .expect("canonical desktop executable")
        );
        assert_eq!(
            installation.app_server,
            Some(fs::canonicalize(cli).expect("canonical CLI"))
        );
        assert_eq!(installation.version, "26.1111.21308");
        assert_eq!(
            installation.package_name.as_deref(),
            Some("com.openai.codex")
        );
    }

    #[test]
    fn mac_discovery_rejects_wrong_bundle_id_even_with_expected_app_name() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "ChatGPT.app", "com.other.codex", "1");
        assert!(matches!(
            discover_in_candidates([bundle], ChannelPreference::Any, verified_fixture),
            Err(DiscoveryError::CodexNotFound)
        ));
    }

    #[test]
    fn mac_discovery_rejects_missing_main_program_and_escaping_symlink() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "Codex.app", "com.openai.codex", "1");
        let executable = bundle.join("Contents/MacOS/Desktop Runner");
        fs::remove_file(&executable).expect("remove main executable");
        assert!(
            discover_in_candidates([bundle.clone()], ChannelPreference::Any, verified_fixture)
                .is_err()
        );
        let outside = fixture_file(directory.path(), "outside-executable", true);
        std::os::unix::fs::symlink(outside, executable).expect("escaping symlink");
        assert!(
            discover_in_candidates([bundle], ChannelPreference::Any, verified_fixture).is_err()
        );
    }

    #[test]
    fn mac_discovery_does_not_treat_embedded_cli_wrapper_as_the_app_server() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "Codex.app", "com.openai.codex", "1");
        fixture_file(&bundle, "Contents/Resources/codex-cli/bin/codex", true);
        let installation =
            discover_in_candidates([bundle], ChannelPreference::Any, verified_fixture)
                .expect("GUI");
        assert!(installation.app_server.is_none());
    }

    #[test]
    fn mac_discovery_rejects_bundle_without_resources() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "Codex.app", "com.openai.codex", "1");
        fs::remove_dir_all(bundle.join("Contents/Resources")).expect("remove Resources");
        assert!(
            discover_in_candidates([bundle], ChannelPreference::Any, verified_fixture).is_err()
        );
    }

    #[test]
    fn mac_discovery_prefers_stable_and_filters_channels_by_bundle_id() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let stable = fixture_bundle(
            directory.path(),
            "Beta in Name.app",
            "com.openai.codex",
            "1.2",
        );
        let beta = fixture_bundle(
            directory.path(),
            "Stable in Name.app",
            "com.openai.codex.beta",
            "9",
        );
        let newer_stable =
            fixture_bundle(directory.path(), "Newer.app", "com.openai.codex", "1.10");
        let candidates = [beta.clone(), stable, newer_stable];
        for (preference, channel, version) in [
            (ChannelPreference::Any, "stable", "1.10"),
            (ChannelPreference::Stable, "stable", "1.10"),
            (ChannelPreference::Beta, "beta", "9"),
        ] {
            let installation =
                discover_in_candidates(candidates.clone(), preference, verified_fixture)
                    .expect("matching channel");
            assert_eq!(installation.channel, channel);
            assert_eq!(installation.version, version);
        }
        assert!(
            discover_in_candidates([beta], ChannelPreference::Stable, verified_fixture).is_err()
        );
    }

    #[test]
    fn mac_discovery_rejects_matching_metadata_without_official_signature() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "Codex.app", "com.openai.codex", "1");
        assert!(
            discover_in_candidates(
                [bundle.clone()],
                ChannelPreference::Any,
                super::verify_official_bundle
            )
            .is_err()
        );
        assert!(discover_codex(Some(&bundle), None, ChannelPreference::Any).is_err());
    }

    #[test]
    fn mac_discovery_preserves_explicit_executable_overrides_as_custom() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let executable = fixture_file(directory.path(), "development executable", false);
        let installation = discover_codex(Some(&executable), Some("dev"), ChannelPreference::Beta)
            .expect("explicit executable override");
        assert_eq!(
            installation.executable,
            fs::canonicalize(executable).expect("canonical executable")
        );
        assert_eq!(installation.source, DiscoverySource::Explicit);
        assert_eq!(installation.channel, "custom");
        assert_eq!(installation.version, "dev");
        assert!(installation.app_server.is_none());
    }

    #[test]
    fn mac_discovery_does_not_trust_app_server_paths_from_an_unverified_bundle() {
        let directory = tempfile::TempDir::new().expect("temp dir");
        let bundle = fixture_bundle(directory.path(), "Codex.app", "com.openai.codex", "1");
        fixture_file(
            &bundle,
            "Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            true,
        );
        let installation =
            discover_in_candidates([bundle], ChannelPreference::Any, verified_fixture)
                .expect("GUI");
        assert!(discover_app_server_source(None, Some(&installation)).is_err());
    }
}
