//! DEV ONLY: open a *demo* vault pre-unlocked so UI screenshots can be taken
//! in CI / a headless compositor. Never use with a real vault.
//! KEYWHERE_DEMO_PASSWORD=... cargo run --features devtools --example screenshot_demo -- demo.psafe3 [title-to-select]

fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("demo vault path"));
    let pw = zeroize::Zeroizing::new(
        std::env::var("KEYWHERE_DEMO_PASSWORD").expect("KEYWHERE_DEMO_PASSWORD"),
    );
    let vault = keywhere_core::Vault::open(&path, pw.as_bytes()).expect("open demo vault");
    keywhere::run_preunlocked(path, vault, std::env::args().nth(2));
}
