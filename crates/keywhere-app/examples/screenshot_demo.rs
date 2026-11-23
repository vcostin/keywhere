//! DEV ONLY: open a *demo* vault pre-unlocked so UI screenshots can be taken
//! in CI / a headless compositor. Never use with a real vault.
//! KEYWHERE_DEMO_PASSWORD=... cargo run --features devtools --example screenshot_demo -- demo.psafe3 [title-to-select]
//! cargo run --features devtools --example screenshot_demo -- --create-form new.psafe3

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--create-form") {
        let path = std::env::args().nth(2).expect("new vault path");
        keywhere::run_create_form(path.into());
        return;
    }
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("demo vault path"));
    let pw = zeroize::Zeroizing::new(
        std::env::var("KEYWHERE_DEMO_PASSWORD").expect("KEYWHERE_DEMO_PASSWORD"),
    );
    let vault = keywhere_core::Vault::open(&path, pw.as_bytes()).expect("open demo vault");
    keywhere::run_preunlocked(path, vault, std::env::args().nth(2));
}
