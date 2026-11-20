//! Process-wide allocator that zeroes memory when it is freed.
//!
//! Keywhere wipes the secrets it owns (`Zeroizing`, `secret_text`), but
//! libraries make their own short-lived copies: the clipboard backend
//! (arboard / wl-clipboard-rs), GPUI's text layout caches, `String`
//! reallocations, … When those are freed, the bytes would linger in freed
//! heap until reused. Measured: after copying a password and locking, one
//! copy survived in the clipboard backend's freed buffer.
//!
//! This allocator clears every block on `dealloc` (and the old block on
//! `realloc`) before handing it back to the system allocator, so freed
//! memory never retains secrets, whoever allocated it. The cost is a
//! `memset` of each freed block, which is negligible next to everything else
//! a GUI does.

use std::alloc::{GlobalAlloc, Layout, System};

pub struct ZeroizeOnFree;

unsafe impl GlobalAlloc for ZeroizeOnFree {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            // Volatile so the compiler can't elide a write to memory that is
            // about to be freed.
            wipe(ptr, layout.size());
            System.dealloc(ptr, layout)
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            // Never let the system allocator move a block and leave the old
            // contents behind: allocate, copy, wipe, free.
            let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
            let new = System.alloc(new_layout);
            if !new.is_null() {
                std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                wipe(ptr, layout.size());
                System.dealloc(ptr, layout);
            }
            new
        }
    }
}

#[inline]
unsafe fn wipe(ptr: *mut u8, len: usize) {
    // glibc's explicit_bzero is a memset the compiler must not elide.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::explicit_bzero(ptr.cast(), len);
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    unsafe {
        // `write_bytes` + a compiler fence: the zeroing is a visible side
        // effect that the optimizer must keep before the free.
        std::ptr::write_bytes(ptr, 0, len);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        std::hint::black_box(ptr);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn allocations_still_work() {
        let mut v: Vec<u8> = Vec::new();
        for i in 0..10_000u32 {
            v.push(i as u8);
        }
        let s: String = (0..1000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        assert_eq!(v.len(), 10_000);
        assert_eq!(s.len(), 1000);
    }
}
