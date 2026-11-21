//! AppImage desktop integration: an AppImage installs nothing, so desktops
//! that find apps through `.desktop` files show a generic icon, can't pin
//! Keywhere to the task bar, and don't offer it for `.psafe3` files. On start
//! an AppImage adds its own menu entry, icon and MIME type under
//! `$XDG_DATA_HOME` (once; the user can undo it), and keeps the entry's path
//! current if the AppImage is moved.
//!
//! Only entries carrying our marker are ever changed or removed. Nothing is
//! written when a system package already provides `keywhere.desktop`.

use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = "X-Keywhere-AppImage=true";
const TEMPLATE: &str = include_str!("../../../../packaging/linux/keywhere.desktop");
const ICON: &[u8] = include_bytes!("../../../../assets/keywhere-key.svg");
const MIME: &str = include_str!("../../../../packaging/linux/keywhere-mime.xml");

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// First run: the entry was added (show a notice with Undo).
    Created,
    /// The AppImage moved; the entry now points at the new path.
    Updated,
    Unchanged,
}

struct Paths {
    desktop: PathBuf,
    icon: PathBuf,
    mime: PathBuf,
}

fn paths(data_home: &Path) -> Paths {
    Paths {
        desktop: data_home.join("applications/keywhere.desktop"),
        icon: data_home.join("icons/hicolor/scalable/apps/keywhere.svg"),
        mime: data_home.join("mime/packages/keywhere.xml"),
    }
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

fn system_entry_exists() -> bool {
    let dirs =
        // Unset or empty means the spec's default.
        std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| d.split(':').any(|p| !p.is_empty()))
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.split(':')
        .filter(|d| !d.is_empty())
        .any(|d| Path::new(d).join("applications/keywhere.desktop").exists())
}

/// Desktop Entry `Exec` quoting for a path.
fn quote_exec(path: &str) -> String {
    let mut q = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q
}

/// The entry for an AppImage at `appimage`, or `None` if the path can't be
/// written into a desktop file safely.
fn entry_for(appimage: &Path) -> Option<String> {
    let p = appimage.to_str()?;
    if p.contains(['\n', '\r', '%']) {
        return None;
    }
    let exec = quote_exec(p);
    let mut out = String::new();
    for line in TEMPLATE.lines() {
        if let Some(rest) = line.strip_prefix("Exec=keywhere") {
            out.push_str(&format!("Exec={exec}{rest}\n"));
            if rest == " %f" {
                // Main entry: hide it if the AppImage is gone, and mark it ours.
                out.push_str(&format!("TryExec={p}\n{MARKER}\n"));
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Some(out)
}

/// A new private temp file next to `path`: random name, never follows a
/// symlink or reuses an existing file.
fn write_temp(path: &Path, data: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(dir)?;
    let mut rnd = [0u8; 8];
    getrandom::getrandom(&mut rnd).map_err(std::io::Error::other)?;
    let name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        u64::from_le_bytes(rnd)
    );
    let tmp = dir.join(name);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    if let Err(e) = f.write_all(data) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

/// Replace `path` (a rename never writes through a symlink at `path`).
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = write_temp(path, data)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Create `path` only if nothing exists there: an exclusive create that
/// never follows a symlink, so an entry that appeared meanwhile (or a planted
/// link) is never written through or replaced. Works on any filesystem.
fn create_new(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if let Err(e) = f.write_all(data).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Add `path` unless a file (ours or anyone's) is already there.
fn add_if_missing(path: &Path, data: &[u8]) -> std::io::Result<()> {
    match create_new(path, data) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        r => r,
    }
}

fn refresh_caches(data_home: &Path) {
    use std::process::{Command, Stdio};
    for (cmd, arg) in [
        ("update-desktop-database", data_home.join("applications")),
        ("update-mime-database", data_home.join("mime")),
    ] {
        let _ = Command::new(cmd)
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn integrate_into(data_home: &Path, appimage: &Path) -> std::io::Result<Outcome> {
    let p = paths(data_home);
    let Some(entry) = entry_for(appimage) else {
        return Ok(Outcome::Unchanged);
    };
    match fs::read_to_string(&p.desktop) {
        Ok(existing) if !existing.lines().any(|l| l == MARKER) => Ok(Outcome::Unchanged),
        Ok(existing) if existing == entry => Ok(Outcome::Unchanged),
        Ok(_) => {
            write_atomic(&p.desktop, entry.as_bytes())?;
            Ok(Outcome::Updated)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The entry first: if someone else's appeared meanwhile, write
            // nothing at all. Icon and MIME type only where none exist yet.
            match create_new(&p.desktop, entry.as_bytes()) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Ok(Outcome::Unchanged);
                }
                Err(e) => return Err(e),
            }
            add_if_missing(&p.icon, ICON)?;
            add_if_missing(&p.mime, MIME.as_bytes())?;
            Ok(Outcome::Created)
        }
        Err(e) => Err(e),
    }
}

/// Run once at startup (blocking file work; call off the UI thread).
pub fn integrate() -> Outcome {
    if !crate::settings::get().menu_entry || system_entry_exists() {
        return Outcome::Unchanged;
    }
    let (Some(appimage), Some(home)) = (crate::update::running_appimage(), data_home()) else {
        return Outcome::Unchanged;
    };
    match integrate_into(&home, &appimage) {
        Ok(o) => {
            if o != Outcome::Unchanged {
                refresh_caches(&home);
            }
            o
        }
        Err(e) => {
            eprintln!("keywhere: couldn't add the app menu entry: {e}");
            Outcome::Unchanged
        }
    }
}

/// Undo: remove what [`integrate`] added (only if it is ours) and don't add
/// it again.
pub fn remove() {
    crate::settings::update(|s| s.menu_entry = false);
    let Some(home) = data_home() else { return };
    let p = paths(&home);
    let ours = fs::read_to_string(&p.desktop).is_ok_and(|e| e.lines().any(|l| l == MARKER));
    if ours {
        let _ = fs::remove_file(&p.desktop);
        // The icon and MIME type only if they are still exactly ours.
        if fs::read(&p.icon).is_ok_and(|b| b == ICON) {
            let _ = fs::remove_file(&p.icon);
        }
        if fs::read(&p.mime).is_ok_and(|b| b == MIME.as_bytes()) {
            let _ = fs::remove_file(&p.mime);
        }
        refresh_caches(&home);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kw-integ-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_then_follows_a_moved_appimage() {
        let home = tmp("move");
        let a = Path::new("/opt/apps/My Keywhere.AppImage");
        assert_eq!(integrate_into(&home, a).unwrap(), Outcome::Created);
        let e = fs::read_to_string(paths(&home).desktop).unwrap();
        assert!(e.contains("Exec=\"/opt/apps/My Keywhere.AppImage\" %f\n"));
        assert!(e.contains("Exec=\"/opt/apps/My Keywhere.AppImage\" --show\n"));
        assert!(e.contains("TryExec=/opt/apps/My Keywhere.AppImage\n"));
        assert!(e.contains(MARKER));
        assert!(paths(&home).icon.exists() && paths(&home).mime.exists());
        assert_eq!(integrate_into(&home, a).unwrap(), Outcome::Unchanged);
        let b = Path::new("/home/u/Keywhere.AppImage");
        assert_eq!(integrate_into(&home, b).unwrap(), Outcome::Updated);
        assert!(fs::read_to_string(paths(&home).desktop)
            .unwrap()
            .contains("TryExec=/home/u/Keywhere.AppImage\n"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn leaves_entries_it_did_not_create() {
        let home = tmp("foreign");
        let p = paths(&home);
        write_atomic(&p.desktop, b"[Desktop Entry]\nExec=/somewhere/else\n").unwrap();
        assert_eq!(
            integrate_into(&home, Path::new("/a/Keywhere.AppImage")).unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(
            fs::read_to_string(&p.desktop).unwrap(),
            "[Desktop Entry]\nExec=/somewhere/else\n"
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn temp_files_never_follow_symlinks() {
        let home = tmp("symlink");
        let p = paths(&home);
        let victim = home.join("victim");
        fs::write(&victim, "keep").unwrap();
        fs::create_dir_all(p.desktop.parent().unwrap()).unwrap();
        // A symlink at the final path is replaced, not written through.
        std::os::unix::fs::symlink(&victim, &p.desktop).unwrap();
        write_atomic(&p.desktop, b"new").unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(fs::read_to_string(&p.desktop).unwrap(), "new");
        // create_new refuses to replace anything.
        assert_eq!(
            create_new(&p.desktop, b"other").unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&p.desktop).unwrap(), "new");
        let leftovers = fs::read_dir(p.desktop.parent().unwrap())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn keeps_existing_icon_and_writes_nothing_on_a_lost_race() {
        let home = tmp("race");
        let p = paths(&home);
        // Someone else's icon stays as it is.
        write_atomic(&p.icon, b"<svg>theirs</svg>").unwrap();
        assert_eq!(
            integrate_into(&home, Path::new("/a/Keywhere.AppImage")).unwrap(),
            Outcome::Created
        );
        assert_eq!(fs::read(&p.icon).unwrap(), b"<svg>theirs</svg>");
        assert!(p.mime.exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn exec_quoting_and_unsafe_paths() {
        assert_eq!(quote_exec("/a/b$c\"d"), "\"/a/b\\$c\\\"d\"");
        assert!(entry_for(Path::new("/a/100%.AppImage")).is_none());
        assert!(entry_for(Path::new("/a/x\ny.AppImage")).is_none());
    }
}
