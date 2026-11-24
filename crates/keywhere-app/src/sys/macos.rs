//! macOS: lock when the screen locks, the displays sleep, the user switches
//! to another account (fast user switching), or the Mac goes to sleep.
//!
//! The AppKit observers are registered once, on the main thread, and live
//! for the whole process; AppKit delivers them on the main thread.
//!
//! Sleep goes through IOKit instead of `NSWorkspaceWillSleepNotification`,
//! which is only a notice: the Mac may sleep before the locked view has been
//! drawn, leaving a frame with secrets in the window server's buffers. IOKit
//! waits for `IOAllowPowerChange` (up to 30 s), so we ask the app to lock,
//! wait for its ack, give it time to draw the locked view and flush the old
//! frames, and only then let the Mac sleep — the same handshake as the
//! logind delay inhibitor on Linux. The wait is bounded: if the app doesn't
//! ack within 3 s (a hung UI), the Mac sleeps anyway.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSString,
};

use super::{EventTx, SysEvent};

/// Posted to the distributed center when the screen locks.
const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";

pub fn watch_session(tx: EventTx) {
    let workspace = NSWorkspace::sharedWorkspace();
    let center = workspace.notificationCenter();
    // SAFETY: AppKit's exported notification-name constants.
    let names: [(&NSNotificationName, SysEvent); 2] = unsafe {
        [
            (NSWorkspaceScreensDidSleepNotification, SysEvent::Lock),
            (
                NSWorkspaceSessionDidResignActiveNotification,
                SysEvent::Lock,
            ),
        ]
    };
    for (name, ev) in names {
        observe(&center, name, ev, &tx);
    }
    if !power::watch_sleep(tx.clone()) {
        eprintln!("keywhere: no IOKit power notifications; locking on will-sleep without delay");
        // SAFETY: AppKit's exported notification-name constant.
        let will_sleep = unsafe { NSWorkspaceWillSleepNotification };
        observe(&center, will_sleep, SysEvent::LockForSleep, &tx);
    }
    let locked = NSString::from_str(SCREEN_LOCKED);
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    observe(&distributed, &locked, SysEvent::Lock, &tx);
}

fn observe(center: &NSNotificationCenter, name: &NSNotificationName, ev: SysEvent, tx: &EventTx) {
    let tx = tx.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let _ = tx.unbounded_send(ev.clone());
    });
    // SAFETY: no object filter, default (posting-thread) queue; the block
    // only sends on a thread-safe channel.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    // Observe for the life of the process.
    std::mem::forget(token);
}

/// IOKit system power notifications (`IORegisterForSystemPower`), served on
/// a thread of their own with its own run loop.
mod power {
    use super::*;

    type IoConnect = u32;
    type IoObject = u32;
    type NotificationPort = *mut c_void;
    type RunLoop = *mut c_void;
    type RunLoopSource = *mut c_void;
    type CfString = *const c_void;
    type Callback =
        extern "C" fn(refcon: *mut c_void, service: IoObject, kind: u32, arg: *mut c_void);

    // iokit_common_msg(n) = sys_iokit | sub_iokit_common | n = 0xE0000000 | n
    const CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
    const SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut NotificationPort,
            callback: Callback,
            notifier: *mut IoObject,
        ) -> IoConnect;
        fn IONotificationPortGetRunLoopSource(port: NotificationPort) -> RunLoopSource;
        fn IOAllowPowerChange(root: IoConnect, id: isize) -> i32;
        fn IODeregisterForSystemPower(notifier: *mut IoObject) -> i32;
        fn IONotificationPortDestroy(port: NotificationPort);
        fn IOServiceClose(connect: IoConnect) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: CfString;
        fn CFRunLoopGetCurrent() -> RunLoop;
        fn CFRunLoopAddSource(rl: RunLoop, source: RunLoopSource, mode: CfString);
        fn CFRunLoopRun();
    }

    /// The root power domain connection, set once registration succeeded.
    static ROOT: AtomicU32 = AtomicU32::new(0);

    /// Start the power thread; false if IOKit refused the registration.
    pub fn watch_sleep(tx: EventTx) -> bool {
        let (ok_tx, ok_rx) = std::sync::mpsc::channel();
        let spawned = thread::Builder::new()
            .name("keywhere-power".into())
            .spawn(move || {
                // The sender lives for the whole process, owned by the callback.
                let refcon = Box::into_raw(Box::new(tx)).cast::<c_void>();
                let mut port: NotificationPort = std::ptr::null_mut();
                let mut notifier: IoObject = 0;
                // SAFETY: out-pointers are valid locals; the callback matches
                // IOServiceInterestCallback and refcon outlives the thread.
                let root =
                    unsafe { IORegisterForSystemPower(refcon, &mut port, on_power, &mut notifier) };
                // SAFETY: port is checked before use.
                let source = if port.is_null() {
                    std::ptr::null_mut()
                } else {
                    unsafe { IONotificationPortGetRunLoopSource(port) }
                };
                if root == 0 || source.is_null() {
                    // SAFETY: undo whatever half of the registration took;
                    // no run loop ever served it, so refcon (from
                    // Box::into_raw above) is no longer reachable by IOKit.
                    unsafe {
                        if root != 0 {
                            IODeregisterForSystemPower(&mut notifier);
                            IOServiceClose(root);
                        }
                        if !port.is_null() {
                            IONotificationPortDestroy(port);
                        }
                        drop(Box::from_raw(refcon.cast::<EventTx>()));
                    }
                    let _ = ok_tx.send(false);
                    return;
                }
                ROOT.store(root, Ordering::SeqCst);
                // SAFETY: source is our notification port's run-loop source;
                // the run loop belongs to this thread, which never returns.
                unsafe { CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode) };
                let _ = ok_tx.send(true);
                // SAFETY: runs this thread's run loop for the process lifetime.
                unsafe { CFRunLoopRun() };
            });
        spawned.is_ok() && ok_rx.recv().unwrap_or(false)
    }

    extern "C" fn on_power(refcon: *mut c_void, _service: IoObject, kind: u32, arg: *mut c_void) {
        let root = ROOT.load(Ordering::SeqCst);
        match kind {
            // Idle sleep is being considered: don't veto it. The lock comes
            // with the will-sleep message that follows.
            CAN_SYSTEM_SLEEP => {
                // SAFETY: root is our registered connection; arg is the
                // notification id IOKit passed for this message.
                unsafe { IOAllowPowerChange(root, arg as isize) };
            }
            SYSTEM_WILL_SLEEP => {
                // SAFETY: refcon is the leaked EventTx from watch_sleep.
                let tx = unsafe { &*refcon.cast::<EventTx>() };
                let before = super::super::lock_ack();
                if tx.unbounded_send(SysEvent::LockForSleep).is_ok() {
                    // Wait for the app to lock and ask for a locked frame.
                    // Its ack comes before that frame is drawn, and its
                    // frame flush (the old layouts freed) runs ~240 ms
                    // after, so give both time, as on Windows.
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while super::super::lock_ack() == before && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(20));
                    }
                    thread::sleep(Duration::from_millis(300));
                }
                // SAFETY: as above.
                unsafe { IOAllowPowerChange(root, arg as isize) };
            }
            _ => {}
        }
    }
}
