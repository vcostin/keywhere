//! Per-user config files (last vault path, settings). Never secrets.

use std::fs;
use std::path::{Path, PathBuf};

fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("keywhere"))
}

pub fn last_vault() -> Option<PathBuf> {
    let p = fs::read_to_string(config_dir()?.join("last-vault")).ok()?;
    let p = PathBuf::from(p.trim());
    p.exists().then_some(p)
}

pub fn remember_vault(path: &Path) {
    write_file("last-vault", &path.to_string_lossy());
}

/// The config dir, created owner-only (`0700`) if missing; an existing one is
/// tightened too (it names the vault path and says Keywhere is in use).
fn ensure_dir() -> Option<PathBuf> {
    let dir = config_dir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let _ = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir);
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

/// Read a small text file from the config dir.
pub fn read_file(name: &str) -> Option<String> {
    fs::read_to_string(config_dir()?.join(name)).ok()
}

/// Write a small text file to the config dir (best effort), owner-only,
/// replacing it atomically.
pub fn write_file(name: &str, text: &str) {
    let Some(dir) = ensure_dir() else { return };
    let tmp = dir.join(format!(".{name}.tmp"));
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let ok = opts.open(&tmp).and_then(|mut f| {
        use std::io::Write;
        f.write_all(text.as_bytes())
    });
    if ok.is_ok() {
        let _ = fs::rename(&tmp, dir.join(name));
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

/// Pane widths (sidebar, entry list) in logical pixels, as last resized.
pub fn pane_widths() -> Option<(f32, f32)> {
    let text = read_file("layout")?;
    let get = |key: &str| -> Option<f32> {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
    };
    Some((get("sidebar")?, get("list")?))
}

pub fn remember_pane_widths(sidebar: f32, list: f32) {
    write_file(
        "layout",
        &format!("sidebar = {}\nlist = {}\n", sidebar.round(), list.round()),
    );
}

/// Was this vault path last seen as Password Safe v3? Used to refuse a
/// silent v3 → v2 downgrade (someone replacing the file with an old v2 copy,
/// which has weak key stretching and no tamper detection).
pub fn last_seen_v3(path: &Path) -> bool {
    let key = format_key(path);
    read_file("formats").is_some_and(|t| {
        t.lines()
            .any(|l| l.strip_prefix("v3\t") == Some(key.as_str()))
    })
}

/// Record the format a vault path was opened/saved in.
pub fn remember_format(path: &Path, v3: bool) {
    let key = format_key(path);
    let old = read_file("formats").unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| l.split_once('\t').map(|(_, p)| p) != Some(key.as_str()))
        .map(str::to_string)
        .collect();
    if v3 {
        lines.push(format!("v3\t{key}"));
    }
    // Bounded: keep the most recent 64 paths.
    let skip = lines.len().saturating_sub(64);
    write_file("formats", &(lines[skip..].join("\n") + "\n"));
}

fn format_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace(['\n', '\t'], " ")
}

/// `~/…` shortening for display.
pub fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}
