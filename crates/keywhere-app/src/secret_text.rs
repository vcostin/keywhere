//! Displaying secrets with GPUI without leaving copies in freed memory.
//!
//! GPUI can only draw text from a `SharedString`, which is an `Arc<str>` and
//! is *not* zeroized on drop. We can't avoid handing GPUI an `Arc<str>`, but we
//! can own one reference to it ourselves: every secret display string is
//! registered here, and [`sweep`] (run periodically and on lock) zeroizes each
//! one in place as soon as we hold the *only* remaining reference, meaning
//! GPUI's element tree and line-layout cache have let go of it. Only then is
//! it freed.
//!
//! Zeroing a `str` to all-NUL bytes keeps it valid UTF-8.

use std::sync::{Arc, Mutex};

use gpui::SharedString;

static GRAVEYARD: Mutex<Vec<Arc<str>>> = Mutex::new(Vec::new());

/// Make a display string for secret text and register it for wiping.
pub fn display(secret: &str) -> SharedString {
    let arc: Arc<str> = Arc::from(secret);
    // A poisoned lock must not stop us from tracking secrets.
    GRAVEYARD.lock().unwrap_or_else(|e| e.into_inner()).push(arc.clone());
    SharedString::from(arc)
}

/// Zeroize and drop every registered string nobody else references any more.
/// Returns how many are still in use (held by GPUI).
pub fn sweep() -> usize {
    let mut g = GRAVEYARD.lock().unwrap_or_else(|e| e.into_inner());
    g.retain_mut(|arc| match Arc::get_mut(arc) {
        Some(s) => {
            // SAFETY: we are the unique owner (get_mut succeeded); writing NUL
            // bytes keeps the str valid UTF-8.
            unsafe { s.as_bytes_mut() }.fill(0);
            false
        }
        None => true,
    });
    g.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipes_once_unreferenced() {
        let s = display("hunter2-secret");
        let raw: Arc<str> = s.clone().into();
        sweep();
        assert!(GRAVEYARD.lock().unwrap().iter().any(|a| Arc::ptr_eq(a, &raw))); // still in use
        drop(s);
        drop(raw);
        // Our registry now holds the last reference; sweep zeroizes + frees it.
        sweep();
        assert!(GRAVEYARD.lock().unwrap().iter().all(|a| &**a != "hunter2-secret"));
    }

    #[test]
    fn zeroizes_in_place() {
        let mut a: Arc<str> = Arc::from("abc");
        unsafe { Arc::get_mut(&mut a).unwrap().as_bytes_mut() }.fill(0);
        assert_eq!(a.as_bytes(), &[0, 0, 0]);
    }
}
