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
//!    file replaces the running copy, and only after the new binary has
//!    reported the expected version:
//!    * Linux AppImage: atomic rename over the AppImage, in the same folder;
//!    * Windows portable exe: the running exe is renamed aside (Windows
//!      allows that, not overwriting) and the new one takes its name; the
//!      old file is deleted at the next start;
//!    * macOS: a zipped `Keywhere.app`, unpacked with `ditto` next to the
//!      installed bundle, which is then swapped by two renames.
//! 5. Only a version strictly newer than the running one is offered, so an
//!    old (validly signed) manifest can't downgrade anyone.
//!
//! Privacy: a check is two GET requests to github.com with a
//! `Keywhere/<version>` user agent. Nothing about vaults is sent.
//!
//! Installs that a package manager owns (Flatpak, distro packages, `cargo`
//! builds), copies in folders we can't write (Program Files, a mounted
//! disk image, a translocated quarantined app) are never modified: they get
//! a notice and a link instead.

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
    /// Windows: the running portable exe, in a folder we can write.
    Exe(PathBuf),
    /// macOS: the running `….app` bundle, in a folder we can write.
    AppBundle(PathBuf),
    /// Owned by something else (Flatpak, package manager, source build) or
    /// not writable: only point to the release page.
    Manual,
}

impl Install {
    pub fn can_replace(&self) -> bool {
        !matches!(self, Install::Manual)
    }

    /// What gets replaced.
    fn target(&self) -> Option<&Path> {
        match self {
            Install::AppImage(p) | Install::Exe(p) | Install::AppBundle(p) => Some(p),
            Install::Manual => None,
        }
    }

    /// The program to start after the swap.
    pub fn launch_path(&self) -> Option<PathBuf> {
        match self {
            Install::AppImage(p) | Install::Exe(p) => Some(p.clone()),
            Install::AppBundle(b) => Some(b.join("Contents/MacOS/keywhere")),
            Install::Manual => None,
        }
    }
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
    #[cfg(windows)]
    if let Some(p) = portable_exe() {
        return Install::Exe(p);
    }
    #[cfg(target_os = "macos")]
    if let Some(b) = app_bundle() {
        return Install::AppBundle(b);
    }
    Install::Manual
}

/// Can this process create (and remove) a file in `dir`? Anyone else who
/// can write there could already replace Keywhere, so the swap adds no risk.
#[cfg(any(windows, target_os = "macos", test))]
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".keywhere-write-test-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            fs::remove_file(&probe).is_ok()
        }
        Err(_) => false,
    }
}

/// Windows: the running `.exe`, if its folder is ours to write (not
/// Program Files).
#[cfg(windows)]
fn portable_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if !exe
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    dir_writable(exe.parent()?).then_some(exe)
}

/// macOS: the running bundle, if its folder is ours to write. A disk image
/// is read-only, and so is the random copy macOS runs a quarantined app
/// from (App Translocation).
#[cfg(target_os = "macos")]
fn app_bundle() -> Option<PathBuf> {
    let exe = fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    bundle_of(&exe).filter(|b| b.parent().is_some_and(dir_writable))
}

/// `…/X.app/Contents/MacOS/<exe>` → `…/X.app`; `None` for anything else or
/// a translocated copy.
#[cfg(any(target_os = "macos", test))]
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || bundle.extension()? != "app"
        || bundle
            .components()
            .any(|c| c.as_os_str() == "AppTranslocation")
    {
        return None;
    }
    Some(bundle.to_path_buf())
}

/// Leftovers of a finished update that couldn't be removed while the old
/// version ran (Windows keeps a running exe's file). Best effort.
pub fn cleanup_leftovers() {
    if let Some(target) = install_method().target() {
        cleanup_next_to(target);
    }
}

fn cleanup_next_to(target: &Path) {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return;
    };
    let name = name.to_string_lossy();
    let prefixes = [
        format!(".{name}.old-"),
        // An update interrupted mid-download or mid-unpack.
        format!(".{name}.update-"),
        ".Keywhere-update-".to_string(),
    ];
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if prefixes.iter().any(|p| n.starts_with(p.as_str())) {
            let p = e.path();
            let _ = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
        }
    }
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

/// The AppImage this process runs from, if it is one (no write checks;
/// see [`install_method`] for replacing it).
#[cfg(target_os = "linux")]
pub fn running_appimage() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("APPIMAGE")?);
    (p.is_absolute() && is_type2_appimage(&p) && running_from_appimage()).then_some(p)
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

/// Blocking: download `asset`, verify it, and replace the installed copy
/// with it. `progress(done, total)` is called as bytes arrive.
pub fn download_and_replace(
    release: &Release,
    install: &Install,
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
    replace_from(reader, release, install, progress)
}

/// Stream `reader` into a private temp file next to the installed copy,
/// checking size and SHA-256 against the signed manifest, check the new
/// binary reports the release's version, then put it in place (see the
/// module docs per platform). On any failure the installed copy is
/// untouched and the temp files are removed.
fn replace_from(
    mut reader: impl Read,
    release: &Release,
    install: &Install,
    progress: impl Fn(u64, u64),
) -> Result<(), UpdateError> {
    let asset = release
        .asset
        .as_ref()
        .ok_or_else(|| UpdateError::Install("No build for this system.".into()))?;
    let target = install
        .target()
        .ok_or_else(|| UpdateError::Install("This copy of Keywhere can't update itself.".into()))?;
    let dir = target
        .parent()
        .ok_or_else(|| UpdateError::Install("Keywhere's folder is unknown.".into()))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Keywhere".into());
    // Windows only runs files ending in .exe; the macOS download is a zip.
    let ext = match install {
        Install::Exe(_) => ".exe",
        Install::AppBundle(_) => ".zip",
        _ => "",
    };
    let tmp = dir.join(format!(".{name}.update-{}{ext}", std::process::id()));
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
        let io = |e: std::io::Error| UpdateError::Install(e.to_string());
        match install {
            Install::AppBundle(_) => install_bundle(&tmp, target, &release.version),
            Install::Exe(_) => {
                make_executable(&tmp).map_err(io)?;
                check_reports_version(&tmp, &release.version)?;
                let old = swap_into(target, &tmp).map_err(io)?;
                // Still in use by this process; cleanup_leftovers() takes it.
                let _ = fs::remove_file(old);
                Ok(())
            }
            _ => {
                make_executable(&tmp).map_err(io)?;
                check_reports_version(&tmp, &release.version)?;
                fs::rename(&tmp, target).map_err(io)
            }
        }
    })();
    // On success only the macOS zip is still there (the others were moved).
    let _ = fs::remove_file(&tmp);
    result
}

/// Put `new` at `target` by moving `target` aside first (a running exe or
/// app bundle can be renamed, not overwritten). Restores `target` if the
/// second rename fails. Returns where the old copy went.
fn swap_into(target: &Path, new: &Path) -> std::io::Result<PathBuf> {
    let name = target
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_string_lossy()
        .into_owned();
    let old = target.with_file_name(format!(".{name}.old-{}", std::process::id()));
    fs::rename(target, &old)?;
    if let Err(e) = fs::rename(new, target) {
        let _ = fs::rename(&old, target);
        return Err(e);
    }
    Ok(old)
}

/// macOS: unpack the verified zip next to the installed bundle (`ditto`
/// keeps the bundle's metadata and signature), check the new binary's
/// version, then swap the bundles and delete the old one.
fn install_bundle(zip: &Path, bundle: &Path, version: &semver::Version) -> Result<(), UpdateError> {
    let err = |e: String| UpdateError::Install(e);
    let dir = bundle
        .parent()
        .ok_or_else(|| err("Keywhere's folder is unknown.".into()))?;
    let staging = dir.join(format!(".Keywhere-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    // Private, and fails if someone recreated the name in between.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&staging).map_err(|e| err(e.to_string()))?;
    let result = (|| {
        let ok = std::process::Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(zip)
            .arg(&staging)
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|e| err(e.to_string()))?
            .success();
        if !ok {
            return Err(err("Couldn't unpack the update.".into()));
        }
        let new_app = staging.join("Keywhere.app");
        let new_bin = new_app.join("Contents/MacOS/keywhere");
        // Defence in depth (the zip already matched the signed checksum): a
        // real directory whose symlinks all stay inside it.
        if !bundle_is_contained(&new_app) {
            return Err(UpdateError::Verify(
                "The update contains links outside Keywhere.app.",
            ));
        }
        if !new_bin.is_file() {
            return Err(UpdateError::Verify("The update has no Keywhere.app."));
        }
        check_reports_version(&new_bin, version)?;
        let old = swap_into(bundle, &new_app).map_err(|e| err(e.to_string()))?;
        let _ = fs::remove_dir_all(old);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

/// `app` is a directory (not a symlink), and every symlink inside it is
/// relative and resolves inside it (bundles use relative links in
/// frameworks; nothing may point elsewhere).
fn bundle_is_contained(app: &Path) -> bool {
    let real_dir = fs::symlink_metadata(app).is_ok_and(|m| m.is_dir());
    let Ok(root) = fs::canonicalize(app) else {
        return false;
    };
    if !real_dir {
        return false;
    }
    let mut stack = vec![app.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            let p = entry.path();
            let Ok(m) = fs::symlink_metadata(&p) else {
                return false;
            };
            if m.file_type().is_symlink() {
                let relative = fs::read_link(&p).is_ok_and(|t| t.is_relative());
                let inside = fs::canonicalize(&p).is_ok_and(|c| c.starts_with(&root));
                if !relative || !inside {
                    return false;
                }
            } else if m.is_dir() {
                stack.push(p);
            }
        }
    }
    true
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
    #[cfg(not(unix))]
    let _ = p;
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
    // ETXTBSY: a process forked elsewhere in this program while the file was
    // still open for writing briefly holds that fd; retry for a moment.
    let mut tries = 0;
    let mut child = loop {
        match std::process::Command::new(p)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(fail(e.to_string())),
        }
    };
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
/// exit so the new one can become the single instance and take the vault
/// lock. Only waits on a process of ours; returns false if it is still
/// running.
#[cfg(windows)]
pub fn wait_for_exit(pid: &str) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return true;
    };
    // SAFETY: plain handle query; a null handle means gone (or not ours).
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return true;
    }
    // SAFETY: `h` is a valid process handle owned here.
    let r = unsafe { WaitForSingleObject(h, 30_000) };
    // SAFETY: as above.
    unsafe { CloseHandle(h) };
    r == WAIT_OBJECT_0
}

/// See the Windows variant.
#[cfg(not(any(unix, windows)))]
pub fn wait_for_exit(_pid: &str) -> bool {
    true
}

/// See the non-unix variant.
#[cfg(unix)]
pub fn wait_for_exit(pid: &str) -> bool {
    let Ok(pid) = pid.parse::<u32>() else {
        return true;
    };
    // Linux: /proc/<pid> owned by us. macOS (no /proc): kill(pid, 0)
    // succeeds only for a live process we may signal, i.e. our own.
    #[cfg(target_os = "linux")]
    let alive = || {
        use std::os::unix::fs::MetadataExt;
        let proc = PathBuf::from(format!("/proc/{pid}"));
        fs::metadata(&proc).is_ok_and(|m| m.uid() == unsafe { libc::getuid() })
    };
    #[cfg(not(target_os = "linux"))]
    let alive = || {
        // SAFETY: signal 0 only checks existence and permission.
        i32::try_from(pid).is_ok_and(|p| unsafe { libc::kill(p, 0) } == 0)
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while alive() {
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
    let install = install_method();
    let (Status::Available(rel), Some(launch)) = (status(), install.launch_path()) else {
        return;
    };
    set_status(Status::Downloading {
        version: rel.version.clone(),
        percent: 0,
    });
    cx.refresh_windows();
    let task = cx.background_executor().spawn(async move {
        download_and_replace(&rel, &install, |done, total| {
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
            Ok(_) => on_ready(cx, launch),
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

    // Uses a shell script as the "new version".
    #[cfg(unix)]
    #[test]
    fn replace_only_with_verified_file() {
        let dir = std::env::temp_dir().join(format!("kw-repl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Keywhere.AppImage");
        let install = Install::AppImage(target.clone());
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
                replace_from(&data[..], &r, &install, |_, _| {}),
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
            replace_from(&new[..], &wrong, &install, |_, _| {}),
            Err(UpdateError::Install(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), b"old");
        // The real thing.
        replace_from(&new[..], &rel(good, new.len() as u64), &install, |_, _| {}).unwrap();
        assert_eq!(fs::read(&target).unwrap(), new);
        assert_eq!(leftovers(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    // The Windows path (exe renamed aside, new one in its place), run with a
    // shell script so it works on any Unix CI.
    #[cfg(unix)]
    #[test]
    fn exe_is_swapped_and_old_copy_cleaned() {
        let dir = std::env::temp_dir().join(format!("kw-exe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Keywhere.exe");
        fs::write(&target, "old").unwrap();
        let new = b"#!/bin/sh\necho 'Keywhere 2.0.0 \xc2\xb7 abc'\n".to_vec();
        let r = Release {
            version: v("2.0.0"),
            notes_url: String::new(),
            asset: Some(Asset {
                url: String::new(),
                sha256: hex(&Sha256::digest(&new)),
                size: new.len() as u64,
            }),
        };
        replace_from(&new[..], &r, &Install::Exe(target.clone()), |_, _| {}).unwrap();
        assert_eq!(fs::read(&target).unwrap(), new);
        // Only the new exe is left (Unix can delete the old one at once).
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Keywhere.exe".to_string()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn swap_restores_target_when_new_is_missing() {
        let dir = std::env::temp_dir().join(format!("kw-swap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Keywhere.exe");
        fs::write(&target, "old").unwrap();
        assert!(swap_into(&target, &dir.join("missing")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        // Directories (app bundles) swap the same way.
        let app = dir.join("Keywhere.app");
        let new_app = dir.join("staging").join("Keywhere.app");
        fs::create_dir_all(app.join("Contents")).unwrap();
        fs::write(app.join("Contents/v"), "1").unwrap();
        fs::create_dir_all(new_app.join("Contents")).unwrap();
        fs::write(new_app.join("Contents/v"), "2").unwrap();
        let old = swap_into(&app, &new_app).unwrap();
        assert_eq!(fs::read(app.join("Contents/v")).unwrap(), b"2");
        assert_eq!(fs::read(old.join("Contents/v")).unwrap(), b"1");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn leftovers_next_to_target_are_removed() {
        let dir = std::env::temp_dir().join(format!("kw-left-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Keywhere.exe");
        fs::write(&target, "new").unwrap();
        fs::write(dir.join(".Keywhere.exe.old-123"), "old").unwrap();
        fs::write(dir.join(".Keywhere.exe.update-9.exe"), "partial").unwrap();
        fs::create_dir_all(dir.join(".Keywhere-update-9/Keywhere.app")).unwrap();
        fs::create_dir_all(dir.join(".Keywhere.exe.old-456/Contents")).unwrap();
        fs::write(dir.join("vault.psafe3"), "keep").unwrap();
        cleanup_next_to(&target);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["Keywhere.exe", "vault.psafe3"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn bundle_symlinks_must_stay_inside() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("kw-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let app = dir.join("Keywhere.app");
        fs::create_dir_all(app.join("Contents/Frameworks/X.framework/Versions/A")).unwrap();
        // A framework-style relative link inside the bundle is fine.
        symlink(
            "Versions/A",
            app.join("Contents/Frameworks/X.framework/Current"),
        )
        .unwrap();
        assert!(bundle_is_contained(&app));
        // Absolute, or relative but escaping: refused.
        symlink("/etc", app.join("Contents/abs")).unwrap();
        assert!(!bundle_is_contained(&app));
        fs::remove_file(app.join("Contents/abs")).unwrap();
        symlink("../../..", app.join("Contents/up")).unwrap();
        assert!(!bundle_is_contained(&app));
        fs::remove_file(app.join("Contents/up")).unwrap();
        // The bundle itself being a symlink: refused.
        let link = dir.join("Link.app");
        symlink(&app, &link).unwrap();
        assert!(!bundle_is_contained(&link));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn app_bundle_from_exe_path() {
        let b = |p: &str| bundle_of(Path::new(p));
        assert_eq!(
            b("/Applications/Keywhere.app/Contents/MacOS/keywhere"),
            Some(PathBuf::from("/Applications/Keywhere.app"))
        );
        assert_eq!(b("/usr/local/bin/keywhere"), None);
        assert_eq!(b("/x/Keywhere.app/Contents/Resources/keywhere"), None);
        assert_eq!(
            b("/private/var/folders/x/AppTranslocation/ABC/d/Keywhere.app/Contents/MacOS/keywhere"),
            None
        );
    }

    #[test]
    fn writable_dir_probe() {
        let dir = std::env::temp_dir();
        assert!(dir_writable(&dir));
        assert!(!dir_writable(&dir.join("does-not-exist-kw")));
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

    // Uses a shell script as the "new version".
    #[cfg(unix)]
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
