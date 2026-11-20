//! Keywhere desktop app entry point.

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(keywhere::config::last_vault);
    keywhere::run(path);
}
