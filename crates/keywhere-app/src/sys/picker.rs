//! "Open another vault…" / "Create new vault…" file dialogs on Linux,
//! through a helper process.
//!
//! Release builds mark Keywhere non-dumpable (`PR_SET_DUMPABLE=0`) so other
//! processes can't read its memory. A side effect: xdg-desktop-portal can't
//! open `/proc/<pid>/root` to identify the caller and refuses every request
//! ("Portal operation not allowed"). Instead of weakening that, the dialog is
//! requested by a short-lived `keywhere --pick-vault` (or `--pick-new-vault`)
//! child: a fresh process (exec resets dumpable), holding no secrets, which
//! prints the chosen path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const PICK_ARG: &str = "--pick-vault";
/// Save dialog for a new vault: `--pick-new-vault <title> <accept> <name> [<folder>]`.
pub const PICK_NEW_ARG: &str = "--pick-new-vault";

/// Parent side (blocking; run it off the UI thread). `Ok(None)` = cancelled.
pub fn pick_vault(title: &str, accept: &str) -> Result<Option<PathBuf>, String> {
    run_helper(&[PICK_ARG, title, accept])
}

/// Save dialog for a new vault file, suggesting `name` in `folder`.
/// `Ok(None)` = cancelled. The portal may already have asked about
/// replacing an existing file; the caller still refuses to replace one.
pub fn pick_new_vault(
    title: &str,
    accept: &str,
    name: &str,
    folder: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, String> {
    let folder = folder.and_then(|f| f.to_str()).unwrap_or("");
    run_helper(&[PICK_NEW_ARG, title, accept, name, folder])
}

fn run_helper(args: &[&str]) -> Result<Option<PathBuf>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    // The user may browse for a while; a helper that never answers is
    // killed eventually so the unlock screen isn't stuck waiting forever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30 * 60);
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the file dialog did not answer".into());
            }
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
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

/// Child side: `keywhere --pick-vault <title> <accept label>`, or
/// `keywhere --pick-new-vault <title> <accept label> <name> [<folder>]` (`save`).
/// Prints the chosen path and exits 0, exits 1 on cancel, 2 (message on
/// stderr) on error.
pub fn run_child(args: &[String], save: bool) -> ! {
    let arg = |i: usize, default: &'static str| args.get(i).map(String::as_str).unwrap_or(default);
    let r = if save {
        let folder = Some(arg(3, "")).filter(|f| !f.is_empty());
        choose(
            "SaveFile",
            arg(0, "Create new vault"),
            arg(1, "Create"),
            Some((arg(2, "Keywhere.psafe3"), folder)),
        )
    } else {
        choose("OpenFile", arg(0, "Open vault"), arg(1, "Open"), None)
    };
    match r {
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

/// org.freedesktop.portal.FileChooser.OpenFile or SaveFile (with `save`:
/// suggested file name and folder), waiting for the Response signal on the
/// request object.
fn choose(
    method: &str,
    title: &str,
    accept: &str,
    save: Option<(&str, Option<&str>)>,
) -> Result<Option<PathBuf>, String> {
    let err = |e: zbus::Error| e.to_string();
    let conn = zbus::blocking::Connection::session().map_err(err)?;
    let token = format!("keywhere{}", std::process::id());
    let sender = conn
        .unique_name()
        .ok_or("no D-Bus name")?
        .trim_start_matches(':')
        .replace('.', "_");
    // Subscribe to every Response for our requests *before* calling, so a
    // fast reply isn't missed even if an old portal picks its own path.
    let responses = zbus::blocking::MessageIterator::for_match_rule(
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .map_err(err)?
            .member("Response")
            .map_err(err)?
            .path_namespace(format!("/org/freedesktop/portal/desktop/request/{sender}"))
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
    options.insert("filters", Value::from(filters));
    match save {
        None => {
            options.insert("multiple", Value::from(false));
        }
        Some((name, folder)) => {
            options.insert("current_name", Value::from(name));
            if let Some(folder) = folder {
                // A NUL-terminated byte string ("ay"), as the portal spec says.
                let mut bytes = folder.as_bytes().to_vec();
                bytes.push(0);
                options.insert("current_folder", Value::from(bytes));
            }
        }
    }
    let reply = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.FileChooser"),
            method,
            &("", title, options),
        )
        .map_err(err)?;
    let handle: OwnedObjectPath = reply.body().deserialize().map_err(err)?;
    for msg in responses {
        let msg = msg.map_err(err)?;
        if msg.header().path().map(|p| p.as_str()) == Some(handle.as_str()) {
            return response(&msg);
        }
    }
    Err("the file dialog closed unexpectedly".into())
}

fn response(msg: &zbus::Message) -> Result<Option<PathBuf>, String> {
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        msg.body().deserialize().map_err(|e| e.to_string())?;
    parse_response(code, &results)
}

/// The portal's `Response(code, results)`: the chosen local file, `None` if
/// the user cancelled.
fn parse_response(
    code: u32,
    results: &HashMap<String, OwnedValue>,
) -> Result<Option<PathBuf>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn uris(list: &[&str]) -> HashMap<String, OwnedValue> {
        let v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        HashMap::from([(
            "uris".to_string(),
            OwnedValue::try_from(Value::from(v)).unwrap(),
        )])
    }

    #[test]
    fn chosen_file_is_decoded() {
        let r = parse_response(0, &uris(&["file:///home/u/My%20Vault.psafe3"]));
        assert_eq!(r, Ok(Some(PathBuf::from("/home/u/My Vault.psafe3"))));
    }

    #[test]
    fn cancel_is_not_an_error() {
        assert_eq!(parse_response(1, &HashMap::new()), Ok(None));
    }

    #[test]
    fn failures_and_odd_replies_are_errors() {
        assert!(parse_response(2, &uris(&["file:///a.psafe3"])).is_err());
        assert!(parse_response(0, &HashMap::new()).is_err());
        assert!(parse_response(0, &uris(&[])).is_err());
        assert!(parse_response(0, &uris(&["https://example.com/a.psafe3"])).is_err());
        let wrong_type = HashMap::from([(
            "uris".to_string(),
            OwnedValue::try_from(Value::from(7u32)).unwrap(),
        )]);
        assert!(parse_response(0, &wrong_type).is_err());
    }
}
