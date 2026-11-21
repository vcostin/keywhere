//! "Open another vault…" file dialog on Linux, through a helper process.
//!
//! Release builds mark Keywhere non-dumpable (`PR_SET_DUMPABLE=0`) so other
//! processes can't read its memory. A side effect: xdg-desktop-portal can't
//! open `/proc/<pid>/root` to identify the caller and refuses every request
//! ("Portal operation not allowed"). Instead of weakening that, the dialog is
//! requested by a short-lived `keywhere --pick-vault` child: a fresh process
//! (exec resets dumpable), holding no secrets, which prints the chosen path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const PICK_ARG: &str = "--pick-vault";

/// Parent side (blocking; run it off the UI thread). `Ok(None)` = cancelled.
pub fn pick_vault(title: &str, accept: &str) -> Result<Option<PathBuf>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = Command::new(exe)
        .args([PICK_ARG, title, accept])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => {
            let path = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
            let path = path.trim_end_matches('\n');
            Ok((!path.is_empty()).then(|| PathBuf::from(path)))
        }
        Some(1) => Ok(None),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Child side: `keywhere --pick-vault <title> <accept label>`. Prints the
/// chosen path and exits 0, exits 1 on cancel, 2 (message on stderr) on error.
pub fn run_child(args: &[String]) -> ! {
    let title = args.first().map(String::as_str).unwrap_or("Open vault");
    let accept = args.get(1).map(String::as_str).unwrap_or("Open");
    match open_file(title, accept) {
        Ok(Some(p)) => {
            println!("{}", p.display());
            std::process::exit(0)
        }
        Ok(None) => std::process::exit(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2)
        }
    }
}

/// org.freedesktop.portal.FileChooser.OpenFile, waiting for the Response
/// signal on the request object.
fn open_file(title: &str, accept: &str) -> Result<Option<PathBuf>, String> {
    let err = |e: zbus::Error| e.to_string();
    let conn = zbus::blocking::Connection::session().map_err(err)?;
    let token = format!("keywhere{}", std::process::id());
    let sender = conn
        .unique_name()
        .ok_or("no D-Bus name")?
        .trim_start_matches(':')
        .replace('.', "_");
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    // Subscribe before calling, so a fast reply isn't missed.
    let responses = zbus::blocking::MessageIterator::for_match_rule(
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .map_err(err)?
            .member("Response")
            .map_err(err)?
            .path(request_path.as_str())
            .map_err(err)?
            .build(),
        &conn,
        None,
    )
    .map_err(err)?;

    let filters: Vec<(&str, Vec<(u32, &str)>)> = vec![
        (
            "Password Safe (*.psafe3, *.dat)",
            vec![(0, "*.psafe3"), (0, "*.dat")],
        ),
        ("All files", vec![(0, "*")]),
    ];
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("accept_label", Value::from(accept));
    options.insert("modal", Value::from(true));
    options.insert("multiple", Value::from(false));
    options.insert("filters", Value::from(filters));
    let reply = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.FileChooser"),
            "OpenFile",
            &("", title, options),
        )
        .map_err(err)?;
    let handle: OwnedObjectPath = reply.body().deserialize().map_err(err)?;
    if handle.as_str() != request_path {
        // Old portals pick their own path; follow it.
        return wait_on(&conn, handle.as_str());
    }
    let mut responses = responses;
    match responses.next() {
        Some(msg) => response(&msg.map_err(err)?),
        None => Err("the file dialog closed unexpectedly".into()),
    }
}

fn wait_on(conn: &zbus::blocking::Connection, path: &str) -> Result<Option<PathBuf>, String> {
    let err = |e: zbus::Error| e.to_string();
    let it = zbus::blocking::MessageIterator::for_match_rule(
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .map_err(err)?
            .member("Response")
            .map_err(err)?
            .path(path)
            .map_err(err)?
            .build(),
        conn,
        None,
    )
    .map_err(err)?;
    let mut it = it;
    match it.next() {
        Some(msg) => response(&msg.map_err(err)?),
        None => Err("the file dialog closed unexpectedly".into()),
    }
}

fn response(msg: &zbus::Message) -> Result<Option<PathBuf>, String> {
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        msg.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => {}
        1 => return Ok(None), // cancelled
        _ => return Err("the file dialog failed".into()),
    }
    let uris: Vec<String> = results
        .get("uris")
        .cloned()
        .ok_or("no file chosen")?
        .try_into()
        .map_err(|e: zbus::zvariant::Error| e.to_string())?;
    let uri = uris.first().ok_or("no file chosen")?;
    url::Url::parse(uri)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .map(Some)
        .ok_or_else(|| format!("not a local file: {uri}"))
}
