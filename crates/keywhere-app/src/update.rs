//! In-app updates from GitHub Releases.
//!
//! Trust model, in order:
//! 1. `latest.json` and its minisign signature `latest.json.minisig` are
//!    fetched over HTTPS from this repository's latest release.
//! 2. The signature must verify with the public key compiled into this
//!    binary (`packaging/keywhere-update.pub`). Its private half lives only in
//!    the release workflow's secrets. Without a valid signature nothing in the
//!    manifest is used.
//! 3. The manifest names, per platform, a download URL that must be inside
//!    this repository's release downloads, its exact size and its SHA-256.
//! 4. The download is size-capped and hashed as it streams. Only a matching
//!    file replaces the running AppImage (atomic rename in the same folder),
//!    and only after the new file has reported the expected version.
//! 5. Only a version strictly newer than the running one is offered, so an
//!    old (validly signed) manifest can't downgrade anyone.
//!
//! Privacy: a check is two GET requests to github.com with a
//! `Keywhere/<version>` user agent. Nothing about vaults is sent.
//!
//! Installs that a package manager owns (Flatpak, distro packages, `cargo`
//! builds) are never modified: they get a notice and a link instead.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const RELEASES_PAGE: &str = "https://github.com/vcostin/keywhere/releases/latest";
const MANIFEST_URL: &str =
    "https://github.com/vcostin/keywhere/releases/latest/download/latest.json";
const SIGNATURE_URL: &str =
    "https://github.com/vcostin/keywhere/releases/latest/download/latest.json.minisig";
/// Every download URL in a manifest must start with this.
const DOWNLOAD_PREFIX: &str = "https://github.com/vcostin/keywhere/releases/download/";
const PUBLIC_KEY: &str = include_str!("../../../packaging/keywhere-update.pub");

const MANIFEST_MAX: u64 = 64 * 1024;
const SIGNATURE_MAX: u64 = 4 * 1024;
const BINARY_MAX: u64 = 400 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// Couldn't reach GitHub (offline, DNS, timeout, HTTP error).
    Network(String),
    /// Signature, manifest or download didn't verify. Never retried silently.
    Verify(&'static str),
    /// Couldn't write or run the new version.
    Install(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) | UpdateError::Install(e) => f.write_str(e),
            UpdateError::Verify(e) => f.write_str(e),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes_url: Option<String>,
    platforms: BTreeMap<String, Asset>,
}

/// A verified release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: semver::Version,
    /// Release page (always on github.com/vcostin/keywhere).
    pub notes_url: String,
    /// `None`: no build for this platform in that release.
    pub asset: Option<Asset>,
}

/// How this copy of Keywhere can be updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// A writable AppImage: download, verify and swap it in place.
    AppImage(PathBuf),
    /// Owned by something else (Flatpak, package manager, source build):
    /// only point to the release page.
    Manual,
}

pub fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

/// Manifest key for this build, e.g. `linux-x86_64`.
pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn install_method() -> Install {
    #[cfg(target_os = "linux")]
    {
        // Set by the AppImage runtime to the image's own path. Inside a
        // Flatpak sandbox it is never honoured.
        if std::env::var_os("FLATPAK_ID").is_none() {
            if let Some(p) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
                if appimage_replaceable(&p) {
                    return Install::AppImage(p);
                }
            }
        }
    }
    Install::Manual
}

/// `$APPIMAGE` is only an environment variable, so prove it before
/// overwriting anything: the file must be a type-2 AppImage owned by us, this
/// process must be running from that image's mount (or extracted copy), and
/// the folder must be writable by us and by nobody else (otherwise another
/// user could swap the verified temp file before it is renamed into place).
#[cfg(target_os = "linux")]
fn appimage_replaceable(p: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::getuid() };
    let (Ok(meta), Some(dir)) = (fs::symlink_metadata(p), p.parent()) else {
        return false;
    };
    if !p.is_absolute() || !meta.file_type().is_file() || meta.uid() != uid {
        return false;
    }
    let Ok(dmeta) = fs::metadata(dir) else {
        return false;
    };
    if dmeta.uid() != uid || dmeta.mode() & 0o022 != 0 {
        return false;
    }
    if !is_type2_appimage(p) || !running_from_appimage() {
        return false;
    }
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(any(target_os = "linux", test))]
/// ELF with the AppImage type-2 magic `AI\x02` at offset 8.
fn is_type2_appimage(p: &Path) -> bool {
    let mut head = [0u8; 11];
    fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head.starts_with(b"\x7fELF")
        && &head[8..11] == b"AI\x02"
}

/// Our executable lives inside the AppImage runtime's FUSE mount
/// (`/tmp/.mount_*`) or its extract-and-run folder, i.e. under `$APPDIR`.
#[cfg(target_os = "linux")]
fn running_from_appimage() -> bool {
    let (Some(appdir), Ok(exe)) = (std::env::var_os("APPDIR"), fs::read_link("/proc/self/exe"))
    else {
        return false;
    };
    let appdir = PathBuf::from(appdir);
    let name = appdir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    (name.starts_with(".mount_") || name.starts_with("appimage_extracted_"))
        && exe.starts_with(&appdir)
}

// ---------- verification (pure, tested) ----------

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn safe_download_url(url: &str) -> bool {
    url.starts_with(DOWNLOAD_PREFIX)
        && url.is_ascii()
        && !url.contains("..")
        && !url.contains(['?', '#', '\\', '@', '%'])
        && !url
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
}

/// Verify `json` against `signature` with `public_key`, then validate it and
/// return the release if it is newer than `current`.
fn verify(
    json: &[u8],
    signature: &str,
    public_key: &str,
    current: &semver::Version,
    platform: &str,
) -> Result<Option<Release>, UpdateError> {
    let bad = UpdateError::Verify;
    let pk = minisign_verify::PublicKey::decode(public_key)
        .map_err(|_| bad("The built-in update key is unreadable."))?;
    let sig = minisign_verify::Signature::decode(signature)
        .map_err(|_| bad("The update signature is malformed."))?;
    pk.verify(json, &sig, false)
        .map_err(|_| bad("The update information is not signed by Keywhere."))?;

    let m: Manifest =
        serde_json::from_slice(json).map_err(|_| bad("The update information is malformed."))?;
    let version = semver::Version::parse(&m.version)
        .map_err(|_| bad("The update information has no valid version."))?;
    if version <= *current {
        return Ok(None);
    }
    let tag_page = format!("https://github.com/vcostin/keywhere/releases/tag/v{version}");
    let notes_url = match m.notes_url {
        Some(u) if u == tag_page => u,
        Some(_) => return Err(bad("The update information links outside Keywhere.")),
        None => tag_page,
    };
    let asset = match m.platforms.get(platform) {
        None => None,
        Some(a) => {
            if !safe_download_url(&a.url) || !a.url.contains(&format!("/v{version}/")) {
                return Err(bad("The update download is not a Keywhere release file."));
            }
            if !is_sha256_hex(&a.sha256) || a.size == 0 || a.size > BINARY_MAX {
                return Err(bad("The update information is malformed."));
            }
            Some(a.clone())
        }
    };
    Ok(Some(Release {
        version,
        notes_url,
        asset,
    }))
}

// ---------- network ----------

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(5)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("Keywhere/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn get_small(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, UpdateError> {
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    resp.body_mut()
        .with_config()
        .limit(max)
        .read_to_vec()
        .map_err(|e| UpdateError::Network(e.to_string()))
}

/// Blocking: ask GitHub for the latest release. `Ok(None)` = up to date.
pub fn check() -> Result<Option<Release>, UpdateError> {
    let agent = agent();
    let json = get_small(&agent, MANIFEST_URL, MANIFEST_MAX)?;
    let sig = get_small(&agent, SIGNATURE_URL, SIGNATURE_MAX)?;
    let sig = String::from_utf8(sig)
        .map_err(|_| UpdateError::Verify("The update signature is malformed."))?;
    verify(&json, &sig, PUBLIC_KEY, &current_version(), &platform_key())
}

/// Blocking: download `asset`, verify it, and atomically replace `target`
/// with it. `progress(done, total)` is called as bytes arrive.
pub fn download_and_replace(
    release: &Release,
    target: &Path,
    progress: impl Fn(u64, u64),
) -> Result<(), UpdateError> {
    let asset = release
        .asset
        .as_ref()
        .ok_or_else(|| UpdateError::Install("No build for this system.".into()))?;
    let mut resp = agent()
        .get(&asset.url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let reader = resp.body_mut().with_config().limit(asset.size + 1).reader();
    replace_from(reader, release, target, progress)
}

/// Stream `reader` into a private temp file next to `target`, checking size
/// and SHA-256 against the signed manifest, check the new file reports the
/// release's version, then rename it over `target`. On any failure `target`
/// is untouched and the temp file is removed.
fn replace_from(
    mut reader: impl Read,
    release: &Release,
    target: &Path,
    progress: impl Fn(u64, u64),
) -> Result<(), UpdateError> {
    let asset = release
        .asset
        .as_ref()
        .ok_or_else(|| UpdateError::Install("No build for this system.".into()))?;
    let dir = target
        .parent()
        .ok_or_else(|| UpdateError::Install("The AppImage has no folder.".into()))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Keywhere.AppImage".into());
    let tmp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let result = (|| {
        let mut file = create_private(&tmp).map_err(|e| UpdateError::Install(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut done: u64 = 0;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| UpdateError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > asset.size {
                return Err(UpdateError::Verify(
                    "The download is larger than announced.",
                ));
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| UpdateError::Install(e.to_string()))?;
            progress(done, asset.size);
        }
        if done != asset.size || hex(&hasher.finalize()) != asset.sha256 {
            return Err(UpdateError::Verify(
                "The download doesn't match the signed checksum.",
            ));
        }
        file.sync_all()
            .map_err(|e| UpdateError::Install(e.to_string()))?;
        drop(file);
        make_executable(&tmp).map_err(|e| UpdateError::Install(e.to_string()))?;
        check_reports_version(&tmp, &release.version)?;
        fs::rename(&tmp, target).map_err(|e| UpdateError::Install(e.to_string()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn create_private(p: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    // A leftover from an earlier crashed attempt is ours to replace.
    let _ = fs::remove_file(p);
    o.open(p)
}

fn make_executable(p: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Run `<new> --version` and require "Keywhere <version> …", so a file that
/// doesn't start (missing FUSE, wrong architecture) never replaces a working
/// one.
fn check_reports_version(p: &Path, version: &semver::Version) -> Result<(), UpdateError> {
    let fail = |e: String| UpdateError::Install(format!("The new version didn't start: {e}"));
    let mut child = std::process::Command::new(p)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| fail(e.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail("timed out".into()));
            }
        }
    }
    let mut out = String::new();
    if let Some(s) = child.stdout.take() {
        let _ = s.take(4096).read_to_string(&mut out);
    }
    let want = format!("Keywhere {version} ");
    if out.starts_with(&want) {
        Ok(())
    } else {
        Err(fail(out.lines().next().unwrap_or("no output").to_string()))
    }
}

/// Start the updated AppImage once this process has exited.
pub fn relaunch(target: &Path) -> std::io::Result<()> {
    std::process::Command::new(target)
        .arg("--after-update")
        .arg(std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// `keywhere --after-update <pid>`: wait (up to 30 s) for the old process to
/// exit so the new one can become the single instance. Only waits on a
/// process of ours (same uid); returns false if it is still running.
#[cfg(not(unix))]
pub fn wait_for_exit(_pid: &str) -> bool {
    true
}

/// See the non-unix variant.
#[cfg(unix)]
pub fn wait_for_exit(pid: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(pid) = pid.parse::<u32>() else {
        return true;
    };
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let ours = |p: &Path| fs::metadata(p).is_ok_and(|m| m.uid() == unsafe { libc::getuid() });
    let deadline = Instant::now() + Duration::from_secs(30);
    while ours(&proc) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

// ---------- UI state ----------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading {
        version: semver::Version,
        percent: u8,
    },
    Failed(UpdateError),
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);
/// The user chose "Later" for this version: don't show the banner again
/// this session.
static DISMISSED: Mutex<Option<semver::Version>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_status(s: Status) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// Forget a shown error (toast "Dismiss").
pub fn clear() {
    set_status(Status::Idle);
}

pub fn set_failed(e: UpdateError) {
    set_status(Status::Failed(e));
}

fn busy() -> bool {
    matches!(status(), Status::Checking | Status::Downloading { .. })
}

/// The release to advertise in a banner, unless dismissed.
pub fn banner() -> Option<Release> {
    match status() {
        Status::Available(r)
            if DISMISSED.lock().unwrap_or_else(|e| e.into_inner()).as_ref() != Some(&r.version) =>
        {
            Some(r)
        }
        _ => None,
    }
}

pub fn dismiss() {
    if let Status::Available(r) = status() {
        *DISMISSED.lock().unwrap_or_else(|e| e.into_inner()) = Some(r.version);
    }
}

/// Check in the background. An automatic check stays silent on failure.
pub fn start_check(manual: bool, cx: &mut gpui::App) {
    if busy() {
        return;
    }
    if manual {
        *DISMISSED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    set_status(Status::Checking);
    cx.refresh_windows();
    let task = cx.background_executor().spawn(async { check() });
    cx.spawn(async move |cx| {
        let r = task.await;
        set_status(match r {
            Ok(None) => Status::UpToDate,
            Ok(Some(rel)) => Status::Available(rel),
            Err(_) if !manual => Status::Idle,
            Err(e) => Status::Failed(e),
        });
        let _ = cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

/// Download and swap in the available release, then call `on_ready` (which
/// locks and relaunches) on the main thread.
pub fn start_install(cx: &mut gpui::App, on_ready: impl FnOnce(&mut gpui::App, PathBuf) + 'static) {
    let (Status::Available(rel), Install::AppImage(target)) = (status(), install_method()) else {
        return;
    };
    set_status(Status::Downloading {
        version: rel.version.clone(),
        percent: 0,
    });
    cx.refresh_windows();
    let target2 = target.clone();
    let task = cx.background_executor().spawn(async move {
        download_and_replace(&rel, &target2, |done, total| {
            let percent = (done.saturating_mul(100) / total.max(1)).min(100) as u8;
            let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
            if let Status::Downloading { percent: p, .. } = &mut *s {
                *p = percent;
            }
        })
        .map(|()| rel)
    });
    // Repaint the progress while downloading.
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        if !matches!(status(), Status::Downloading { .. }) {
            break;
        }
        if cx.update(|cx| cx.refresh_windows()).is_err() {
            break;
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let r = task.await;
        let _ = cx.update(|cx| match r {
            Ok(_) => on_ready(cx, target),
            Err(e) => {
                set_status(Status::Failed(e));
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// DEV ONLY: pretend a release is available (screenshots).
#[cfg(feature = "devtools")]
pub fn dev_fake_available() {
    set_status(Status::Available(Release {
        version: semver::Version::new(0, 2, 0),
        notes_url: "https://github.com/vcostin/keywhere/releases/tag/v0.2.0".into(),
        asset: None,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUB: &str = include_str!("../testdata/update/test.pub");
    const TEST_JSON: &[u8] = include_bytes!("../testdata/update/latest.json");
    const TEST_SIG: &str = include_str!("../testdata/update/latest.json.minisig");

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn release_key_is_valid() {
        minisign_verify::PublicKey::decode(PUBLIC_KEY).expect("release public key");
    }

    #[test]
    fn signed_manifest_offers_newer_version() {
        let r = verify(TEST_JSON, TEST_SIG, TEST_PUB, &v("0.1.0"), "linux-x86_64")
            .unwrap()
            .unwrap();
        assert_eq!(r.version, v("9.9.9"));
        assert_eq!(
            r.notes_url,
            "https://github.com/vcostin/keywhere/releases/tag/v9.9.9"
        );
        let a = r.asset.unwrap();
        assert_eq!(a.size, 12345);
        assert!(a.url.ends_with("Keywhere-9.9.9-x86_64.AppImage"));
        // Same release, other platform: notice without a download.
        let r = verify(TEST_JSON, TEST_SIG, TEST_PUB, &v("0.1.0"), "windows-x86_64")
            .unwrap()
            .unwrap();
        assert!(r.asset.is_none());
    }

    #[test]
    fn same_or_older_is_not_offered() {
        for cur in ["9.9.9", "10.0.0"] {
            assert_eq!(
                verify(TEST_JSON, TEST_SIG, TEST_PUB, &v(cur), "linux-x86_64"),
                Ok(None)
            );
        }
    }

    #[test]
    fn tampering_or_wrong_key_is_rejected() {
        let mut j = TEST_JSON.to_vec();
        let i = j.iter().position(|&b| b == b'9').unwrap();
        j[i] = b'8';
        assert!(matches!(
            verify(&j, TEST_SIG, TEST_PUB, &v("0.1.0"), "linux-x86_64"),
            Err(UpdateError::Verify(_))
        ));
        // Signed by the test key, checked with the release key.
        assert!(matches!(
            verify(TEST_JSON, TEST_SIG, PUBLIC_KEY, &v("0.1.0"), "linux-x86_64"),
            Err(UpdateError::Verify(_))
        ));
        assert!(matches!(
            verify(TEST_JSON, "garbage", TEST_PUB, &v("0.1.0"), "linux-x86_64"),
            Err(UpdateError::Verify(_))
        ));
    }

    #[test]
    fn download_urls_are_pinned() {
        let ok = "https://github.com/vcostin/keywhere/releases/download/v1.2.3/Keywhere-1.2.3-x86_64.AppImage";
        assert!(safe_download_url(ok));
        for bad in [
            "http://github.com/vcostin/keywhere/releases/download/v1.2.3/x",
            "https://github.com/vcostin/keywhere.evil/releases/download/v1/x",
            "https://github.com/evil/keywhere/releases/download/v1/x",
            "https://github.com/vcostin/keywhere/releases/download/../../evil/x",
            "https://github.com/vcostin/keywhere/releases/download/v1/x?y=1",
            "https://github.com/vcostin/keywhere/releases/download/v1/%2e%2e/x",
            "https://github.com/vcostin/keywhere/releases/download/v1/a b",
        ] {
            assert!(!safe_download_url(bad), "{bad}");
        }
        assert!(is_sha256_hex(&"ab".repeat(32)));
        assert!(!is_sha256_hex(&"AB".repeat(32)));
        assert!(!is_sha256_hex("abc"));
    }

    #[test]
    fn replace_only_with_verified_file() {
        let dir = std::env::temp_dir().join(format!("kw-repl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Keywhere.AppImage");
        fs::write(&target, "old").unwrap();
        let new = b"#!/bin/sh\necho 'Keywhere 2.0.0 \xc2\xb7 abc'\n".to_vec();
        let rel = |sha: String, size: u64| Release {
            version: v("2.0.0"),
            notes_url: String::new(),
            asset: Some(Asset {
                url: String::new(),
                sha256: sha,
                size,
            }),
        };
        let good = hex(&Sha256::digest(&new));
        let leftovers = || {
            fs::read_dir(&dir)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains(".update-")
                })
                .count()
        };
        // Wrong checksum, truncated, oversized: target untouched, no temp file.
        for (data, r) in [
            (new.clone(), rel("0".repeat(64), new.len() as u64)),
            (new[..10].to_vec(), rel(good.clone(), new.len() as u64)),
            (
                [new.clone(), b"x".to_vec()].concat(),
                rel(good.clone(), new.len() as u64),
            ),
        ] {
            assert!(matches!(
                replace_from(&data[..], &r, &target, |_, _| {}),
                Err(UpdateError::Verify(_))
            ));
            assert_eq!(fs::read(&target).unwrap(), b"old");
            assert_eq!(leftovers(), 0);
        }
        // Verified but reports another version: not installed.
        let wrong = Release {
            version: v("2.0.1"),
            ..rel(good.clone(), new.len() as u64)
        };
        assert!(matches!(
            replace_from(&new[..], &wrong, &target, |_, _| {}),
            Err(UpdateError::Install(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), b"old");
        // The real thing.
        replace_from(&new[..], &rel(good, new.len() as u64), &target, |_, _| {}).unwrap();
        assert_eq!(fs::read(&target).unwrap(), new);
        assert_eq!(leftovers(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn appimage_magic() {
        let dir = std::env::temp_dir().join(format!("kw-magic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x");
        fs::write(&f, b"\x7fELF\x02\x01\x01\x00AI\x02rest").unwrap();
        assert!(is_type2_appimage(&f));
        fs::write(&f, b"\x7fELF\x02\x01\x01\x00\x00\x00\x00rest").unwrap();
        assert!(!is_type2_appimage(&f));
        fs::write(&f, b"short").unwrap();
        assert!(!is_type2_appimage(&f));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_check_of_new_file() {
        let dir = std::env::temp_dir().join(format!("kw-upd-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake");
        fs::write(&script, "#!/bin/sh\necho 'Keywhere 1.2.3 · abc · now'\n").unwrap();
        make_executable(&script).unwrap();
        assert!(check_reports_version(&script, &v("1.2.3")).is_ok());
        assert!(check_reports_version(&script, &v("1.2.4")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
