//! Keywhere desktop app entry point.

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("--version" | "-V")) {
        println!("Keywhere {}", keywhere::VERSION_LABEL);
        return;
    }
    // File-dialog helper (see sys/picker.rs): no GUI, no vault, no hardening.
    #[cfg(target_os = "linux")]
    if arg.as_deref() == Some(keywhere::sys::picker::PICK_ARG) {
        let rest: Vec<String> = std::env::args().skip(2).collect();
        keywhere::sys::picker::run_child(&rest);
    }
    if arg.as_deref() == Some("--after-update") {
        // Started by the updater: let the old process exit first.
        let exited = std::env::args()
            .nth(2)
            .is_none_or(|pid| keywhere::update::wait_for_exit(&pid));
        if !exited {
            eprintln!("Keywhere was updated, but the old version is still running. Quit it and start Keywhere again.");
            std::process::exit(1);
        }
        keywhere::run_after_update(keywhere::config::last_vault());
        return;
    }
    // A file given on the command line (file manager, "Open with").
    let file = std::env::args_os()
        .nth(1)
        .filter(|a| a != "--show")
        .map(std::path::PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p));
    keywhere::run(file);
}
