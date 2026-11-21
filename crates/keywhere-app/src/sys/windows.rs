//! Windows: lock when the session locks (Win+L, screen saver with password),
//! when the user switches away (fast user switching, RDP disconnect), and
//! before the PC sleeps or hibernates, or when the display turns off.
//!
//! A hidden top-level window on its own thread receives
//! `WM_WTSSESSION_CHANGE` (after `WTSRegisterSessionNotification`) and the
//! `WM_POWERBROADCAST` broadcast. It is never shown.
//!
//! The display-off lock matters on Modern Standby (S0ix) laptops: they
//! often never send `PBT_APMSUSPEND`, but the console display always goes
//! off first (`GUID_CONSOLE_DISPLAY_STATE` = 0).

use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, DEVICE_NOTIFY_WINDOW_HANDLE, MSG, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_OVERLAPPED, WTS_CONSOLE_DISCONNECT,
    WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
};

use super::{EventTx, SysEvent};

static TX: OnceLock<EventTx> = OnceLock::new();

pub fn watch_session(tx: EventTx) {
    if TX.set(tx).is_err() {
        return;
    }
    let _ = thread::Builder::new()
        .name("keywhere-session".into())
        .spawn(|| {
            // SAFETY: plain Win32 window/message loop on this thread only.
            if let Err(e) = unsafe { run() } {
                eprintln!("keywhere: session watcher unavailable: {e}");
            }
        });
}

/// What a session-change code means for us.
fn session_event(code: u32) -> Option<SysEvent> {
    matches!(
        code,
        WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT
    )
    .then_some(SysEvent::Lock)
}

unsafe fn run() -> Result<(), &'static str> {
    let class: Vec<u16> = "KeywhereSessionWatcher\0".encode_utf16().collect();
    // SAFETY: null = this executable's module.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    // SAFETY: all-zero is a valid "unset" WNDCLASSW; required fields set below.
    let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    // SAFETY: `wc` and the class name outlive the call.
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err("RegisterClassW failed");
    }
    // A hidden top-level window: message-only windows don't get the
    // WM_POWERBROADCAST broadcast.
    // SAFETY: the registered class above; no parent, menu or create params.
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW failed");
    }
    // SAFETY: `hwnd` is our live window.
    if unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } == 0 {
        eprintln!("keywhere: no session-lock notifications (sleep lock still works)");
    }
    // SAFETY: `hwnd` is our live window; the GUID is a static constant.
    let display = unsafe {
        RegisterPowerSettingNotification(
            hwnd,
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if display == 0 {
        eprintln!("keywhere: no display-off notifications");
    }
    // SAFETY: standard message loop for windows owned by this thread.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => {
            if let (Some(ev), Some(tx)) = (session_event(wparam as u32), TX.get()) {
                let _ = tx.unbounded_send(ev);
            }
        }
        WM_POWERBROADCAST if wparam as u32 == PBT_APMSUSPEND => lock_for_sleep(),
        WM_POWERBROADCAST if wparam as u32 == PBT_POWERSETTINGCHANGE => {
            // SAFETY: for PBT_POWERSETTINGCHANGE, lParam points to a
            // POWERBROADCAST_SETTING valid for the duration of the message.
            let setting = unsafe { (lparam as *const POWERBROADCAST_SETTING).as_ref() };
            if let (Some(s), Some(tx)) = (setting, TX.get()) {
                // SAFETY: `Data` has `DataLength` bytes.
                let data =
                    unsafe { std::slice::from_raw_parts(s.Data.as_ptr(), s.DataLength as usize) };
                if display_turned_off(&s.PowerSetting, data) {
                    let _ = tx.unbounded_send(SysEvent::Lock);
                }
            }
        }
        _ => {}
    }
    // SAFETY: forwarding the unchanged message to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// `GUID_CONSOLE_DISPLAY_STATE` with a DWORD 0 (off; 1 = on, 2 = dimmed).
fn display_turned_off(setting: &windows_sys::core::GUID, data: &[u8]) -> bool {
    let g = GUID_CONSOLE_DISPLAY_STATE;
    let same = setting.data1 == g.data1
        && setting.data2 == g.data2
        && setting.data3 == g.data3
        && setting.data4 == g.data4;
    same && data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == 0
}

/// Windows gives PBT_APMSUSPEND handlers about two seconds: ask the app to
/// lock, and wait (on this thread only) until it confirms or time runs out.
fn lock_for_sleep() {
    let Some(tx) = TX.get() else { return };
    let before = super::lock_ack();
    if tx.unbounded_send(SysEvent::LockForSleep).is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    while super::lock_ack() == before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    // As on Linux: give the locked screen time to replace the last frames
    // (the app's frame flush runs after its ack).
    thread::sleep(Duration::from_millis(300));
}

/// Process hardening, the Windows counterpart of the Linux
/// `PR_SET_DUMPABLE=0` + no core dumps:
/// * no Windows Error Reporting for crashes (no crash dump with secrets);
/// * the process's access list lets other programs of the same user only
///   see that it runs, wait for it and end it, not read or write its memory,
///   inject threads or duplicate its handles. Administrators and the owner
///   (who could rewrite the list) are not stopped; this is defence in depth,
///   like KeePass's "protect process" option.
pub fn harden() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    };
    // SAFETY: only changes this process's error mode.
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    // SAFETY: see restrict_process_access.
    if let Err(e) = unsafe { restrict_process_access() } {
        eprintln!("keywhere: could not restrict process access: {e}");
    }
}

unsafe fn restrict_process_access() -> Result<(), &'static str> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT};
    use windows_sys::Win32::Security::{
        AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl, TokenUser,
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: pseudo-handle of this process; `token` receives a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("OpenProcessToken failed");
    }
    let mut len = 0u32;
    // SAFETY: size query; a null buffer with length 0 is allowed.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    // u64 elements: TOKEN_USER holds pointers and needs their alignment.
    let mut user = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `user` has at least `len` bytes.
    let ok =
        unsafe { GetTokenInformation(token, TokenUser, user.as_mut_ptr().cast(), len, &mut len) };
    // SAFETY: we own `token`.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        return Err("GetTokenInformation failed");
    }
    // SAFETY: filled in by GetTokenInformation(TokenUser); the SID points
    // into `user`, which stays alive until the end of this function.
    let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };

    // SAFETY: `sid` is a valid SID.
    let sid_len = unsafe { GetLengthSid(sid) } as usize;
    let acl_len = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>() + sid_len;
    // u32 elements: an ACL must be DWORD-aligned.
    let mut acl = vec![0u32; acl_len.div_ceil(4)];
    let acl_ptr = acl.as_mut_ptr().cast::<ACL>();
    // SAFETY: `acl` has `acl_len` bytes.
    if unsafe { InitializeAcl(acl_ptr, acl_len as u32, ACL_REVISION) } == 0 {
        return Err("InitializeAcl failed");
    }
    let allowed = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
    // SAFETY: initialised ACL with room for exactly this ACE.
    if unsafe { AddAccessAllowedAce(acl_ptr, ACL_REVISION, allowed, sid) } == 0 {
        return Err("AddAccessAllowedAce failed");
    }
    // SAFETY: valid ACL; owner and group unchanged (null).
    let r = unsafe {
        SetSecurityInfo(
            GetCurrentProcess(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl_ptr,
            std::ptr::null(),
        )
    };
    if r != ERROR_SUCCESS {
        return Err("SetSecurityInfo failed");
    }
    Ok(())
}

/// The per-user installer (NSIS) records the version under Settings → Apps.
/// After an in-app update that entry would still show the old one, so a
/// copy started from the recorded install folder refreshes it. Portable and
/// MSI copies have no such entry (or another folder) and are left alone.
pub fn refresh_installed_version() {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Keywhere");
    let mut buf = [0u16; 1024];
    let mut len = std::mem::size_of_val(&buf) as u32;
    // SAFETY: valid NUL-terminated strings; `buf`/`len` describe a writable buffer.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            wide("InstallLocation").as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if r != ERROR_SUCCESS {
        return;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    let location = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    let here = std::env::current_exe().ok();
    let same = here
        .as_deref()
        .and_then(std::path::Path::parent)
        .is_some_and(|d| same_dir(d, &location));
    if !same {
        return;
    }
    let version = wide(env!("CARGO_PKG_VERSION"));
    // SAFETY: as above; the data is a NUL-terminated UTF-16 string.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            wide("DisplayVersion").as_ptr(),
            REG_SZ,
            version.as_ptr().cast(),
            (version.len() * 2) as u32,
        );
    }
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WTS_SESSION_LOGON, WTS_SESSION_UNLOCK};

    #[test]
    fn display_off_only() {
        let g = GUID_CONSOLE_DISPLAY_STATE;
        assert!(display_turned_off(&g, &0u32.to_le_bytes()));
        assert!(!display_turned_off(&g, &1u32.to_le_bytes()));
        assert!(!display_turned_off(&g, &2u32.to_le_bytes()));
        assert!(!display_turned_off(&g, &[0]));
        let mut other = g;
        other.data1 ^= 1;
        assert!(!display_turned_off(&other, &0u32.to_le_bytes()));
    }

    #[test]
    fn locks_on_lock_and_switch_away_only() {
        assert_eq!(session_event(WTS_SESSION_LOCK), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_CONSOLE_DISCONNECT), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_REMOTE_DISCONNECT), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_SESSION_UNLOCK), None);
        assert_eq!(session_event(WTS_SESSION_LOGON), None);
    }
}
