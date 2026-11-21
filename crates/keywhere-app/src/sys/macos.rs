//! macOS: lock when the screen locks, the displays sleep, the user switches
//! to another account (fast user switching), or the Mac goes to sleep.
//!
//! The observers are registered once, on the main thread, and live for the
//! whole process; AppKit delivers them on the main thread.

use std::ptr::NonNull;

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
    let names: [(&NSNotificationName, SysEvent); 3] = unsafe {
        [
            (NSWorkspaceWillSleepNotification, SysEvent::LockForSleep),
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
