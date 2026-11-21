//! Keywhere desktop app entry point.

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("--version" | "-V")) {
        println!("Keywhere {}", keywhere::VERSION_LABEL);
        return;
    }
    if arg.as_deref() == Some("--after-update") {
        // Started by the updater: let the old process exit first.
        if let Some(pid) = std::env::args().nth(2) {
            keywhere::update::wait_for_exit(&pid);
        }
        keywhere::run(keywhere::config::last_vault());
        return;
    }
    let path = std::env::args_os()
        .nth(1)
        .filter(|a| a != "--show")
        .map(std::path::PathBuf::from)
        .or_else(keywhere::config::last_vault);
    keywhere::run(path);
}
