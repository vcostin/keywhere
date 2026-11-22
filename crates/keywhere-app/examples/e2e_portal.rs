//! Stand-in for xdg-desktop-portal's FileChooser, for tools/e2e/run.sh.
//!
//!   e2e_portal <answer file>
//!
//! Owns `org.freedesktop.portal.Desktop` on the session bus (the e2e test
//! runs on a private one) and answers every OpenFile / SaveFile request with
//! the path written in `<answer file>` at that moment, or "cancelled" when the
//! file is empty or missing. Each request is logged to stderr. Other methods
//! get an error reply, so callers don't wait for a timeout.

#[cfg(target_os = "linux")]
fn main() {
    use std::collections::HashMap;
    use zbus::message::Type;
    use zbus::zvariant::{ObjectPath, Value};

    let answer = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: e2e_portal <answer file>");
    let conn = zbus::blocking::Connection::session().expect("session bus");
    conn.request_name("org.freedesktop.portal.Desktop")
        .expect("portal name");
    eprintln!("e2e_portal: ready");
    let mut n = 0u32;
    for msg in zbus::blocking::MessageIterator::from(&conn) {
        let Ok(msg) = msg else { continue };
        let hdr = msg.header();
        if hdr.message_type() != Type::MethodCall {
            continue;
        }
        let iface = hdr.interface().map(|i| i.to_string()).unwrap_or_default();
        let member = hdr.member().map(|m| m.to_string()).unwrap_or_default();
        if iface != "org.freedesktop.portal.FileChooser"
            || !matches!(member.as_str(), "OpenFile" | "SaveFile")
        {
            let _ = conn.reply_error(
                &hdr,
                "org.freedesktop.DBus.Error.UnknownMethod",
                &"not provided by e2e_portal",
            );
            continue;
        }
        let sender = hdr
            .sender()
            .map(|s| s.trim_start_matches(':').replace('.', "_"))
            .unwrap_or_default();
        n += 1;
        let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/e2e{n}");
        let handle = ObjectPath::try_from(handle.as_str()).expect("object path");
        conn.reply(&hdr, &handle).expect("reply");

        let chosen = std::fs::read_to_string(&answer).unwrap_or_default();
        let chosen = chosen.trim();
        let mut results: HashMap<&str, Value> = HashMap::new();
        let code = if chosen.is_empty() {
            1u32
        } else {
            let uri = url::Url::from_file_path(chosen).expect("absolute path");
            results.insert("uris", Value::from(vec![uri.to_string()]));
            0
        };
        eprintln!(
            "e2e_portal: {member} -> {}",
            if code == 0 { chosen } else { "cancelled" }
        );
        conn.emit_signal(
            hdr.sender().map(|s| s.as_str()),
            &handle,
            "org.freedesktop.portal.Request",
            "Response",
            &(code, results),
        )
        .expect("response signal");
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("e2e_portal is Linux-only");
    std::process::exit(2);
}
