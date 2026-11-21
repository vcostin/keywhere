//! Windows: lock when the session locks (Win+L, screen saver with password),
//! when the user switches away (fast user switching, RDP disconnect), and
//! before the PC sleeps or hibernates.
//!
//! A hidden top-level window on its own thread receives
//! `WM_WTSSESSION_CHANGE` (after `WTSRegisterSessionNotification`) and the
//! `WM_POWERBROADCAST` broadcast. It is never shown.

use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, PBT_APMSUSPEND, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW,
    WS_OVERLAPPED, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
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
        _ => {}
    }
    // SAFETY: forwarding the unchanged message to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WTS_SESSION_LOGON, WTS_SESSION_UNLOCK};

    #[test]
    fn locks_on_lock_and_switch_away_only() {
        assert_eq!(session_event(WTS_SESSION_LOCK), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_CONSOLE_DISCONNECT), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_REMOTE_DISCONNECT), Some(SysEvent::Lock));
        assert_eq!(session_event(WTS_SESSION_UNLOCK), None);
        assert_eq!(session_event(WTS_SESSION_LOGON), None);
    }
}
