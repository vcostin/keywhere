//! Build watermark: git commit (+ "-dirty"), build time. Shown in the UI and
//! by `keywhere --version` so it's always clear which commit a binary is.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let hash = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // Uncommitted changes in tracked files => mark the build.
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let commit = if dirty { format!("{hash}-dirty") } else { hash };
    let date = Command::new("date")
        .args(["-u", "+%Y-%m-%d %H:%M UTC"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=KEYWHERE_COMMIT={commit}");
    println!("cargo:rustc-env=KEYWHERE_BUILD_DATE={date}");
    // Rebuild the watermark when HEAD moves or the index changes.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{r}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    // Re-check the dirty flag whenever sources change, not only on commits.
    for dir in ["src", "../keywhere-core/src", "../../assets"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    windows_resources();
}

/// Windows: the app icon as resource 1 (GPUI loads that for the window,
/// task bar and Alt+Tab; Explorer shows it for the exe) and version info
/// (Properties → Details).
fn windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest_dir.join("../../packaging/windows/keywhere.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut n: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    n.resize(3, 0);
    // .rc strings treat backslashes as escapes; rc.exe accepts '/'.
    let ico = ico.display().to_string().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Keywhere"
      VALUE "FileVersion", "{version}"
      VALUE "ProductName", "Keywhere"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "keywhere.exe"
      VALUE "LegalCopyright", "MIT License"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        a = n[0],
        b = n[1],
        c = n[2],
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("keywhere.rc");
    std::fs::write(&out, rc).expect("write keywhere.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("compile Windows resources");
}
