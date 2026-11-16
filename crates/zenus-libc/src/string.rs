//! Minimal string and memory primitives for userspace.
//!
//! These follow the C contracts: pointers must be valid for the whole range,
//! `strlen`/`strcmp`/`strncmp` expect NUL-terminated bytes, and `memcpy`
//! requires non-overlapping ranges (use `memmove` when they may overlap).
//! They are `unsafe fn` because the caller owns those preconditions — the
//! kernel never validates user pointers on behalf of a userspace program.

/// Copy `n` bytes from `src` to `dst`. The ranges must not overlap.
pub unsafe fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: caller guarantees both ranges are valid and non-overlapping.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, n) };
    dst
}

/// Copy `n` bytes from `src` to `dst`, tolerating overlap.
pub unsafe fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: caller guarantees both ranges are valid for `n` bytes. `copy`
    // picks the direction that preserves the source when they overlap.
    unsafe { core::ptr::copy(src, dst, n) };
    dst
}

/// Fill `n` bytes at `dst` with the low byte of `c`.
pub unsafe fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8 {
    // SAFETY: caller guarantees `dst` is valid for `n` bytes.
    unsafe { core::ptr::write_bytes(dst, c as u8, n) };
    dst
}

/// Compare `n` bytes. Returns negative, zero or positive as `a` is less than,
/// equal to or greater than `b`.
pub unsafe fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: caller guarantees both ranges are valid for `n` bytes.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return x as i32 - y as i32;
        }
    }
    0
}

/// Length of the NUL-terminated string at `s`, excluding the terminator.
pub unsafe fn strlen(s: *const u8) -> usize {
    let mut len = 0;
    // SAFETY: caller guarantees a NUL terminator is reachable from `s`.
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

/// Compare two NUL-terminated strings. Same sign convention as `memcmp`.
pub unsafe fn strcmp(a: *const u8, b: *const u8) -> i32 {
    let mut i = 0;
    loop {
        // SAFETY: caller guarantees both strings are NUL-terminated.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return x as i32 - y as i32;
        }
        if x == 0 {
            return 0;
        }
        i += 1;
    }
}

/// Compare at most `n` bytes of two NUL-terminated strings. Stops early at a
/// NUL on either side, like C's `strncmp`.
pub unsafe fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: caller guarantees both strings are NUL-terminated.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return x as i32 - y as i32;
        }
        if x == 0 {
            return 0;
        }
    }
    0
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn strlen_counts_bytes_until_nul() {
        assert_eq!(unsafe { strlen(b"zenus\0".as_ptr()) }, 5);
        assert_eq!(unsafe { strlen(b"\0".as_ptr()) }, 0);
    }

    #[test]
    fn strcmp_orders_and_detects_equality() {
        assert_eq!(unsafe { strcmp(b"abc\0".as_ptr(), b"abc\0".as_ptr()) }, 0);
        assert!(unsafe { strcmp(b"abc\0".as_ptr(), b"abd\0".as_ptr()) } < 0);
        assert!(unsafe { strcmp(b"abd\0".as_ptr(), b"abc\0".as_ptr()) } > 0);
        // A prefix sorts before the longer string.
        assert!(unsafe { strcmp(b"ab\0".as_ptr(), b"abc\0".as_ptr()) } < 0);
    }

    #[test]
    fn strncmp_stops_at_n_or_nul() {
        assert_eq!(
            unsafe { strncmp(b"abcdef\0".as_ptr(), b"abcxyz\0".as_ptr(), 3) },
            0
        );
        assert!(unsafe { strncmp(b"abcdef\0".as_ptr(), b"abcxyz\0".as_ptr(), 4) } != 0);
        // A NUL on either side ends the comparison before `n`.
        assert_eq!(unsafe { strncmp(b"ab\0cd".as_ptr(), b"ab\0ef".as_ptr(), 5) }, 0);
    }

    #[test]
    fn memcpy_copies_exact_bytes_and_memcmp_compares_them() {
        let src = [1u8, 2, 3, 4, 5];
        let mut dst = [0u8; 5];
        assert_eq!(unsafe { memcpy(dst.as_mut_ptr(), src.as_ptr(), src.len()) }, dst.as_mut_ptr());
        assert_eq!(dst, src);
        assert_eq!(unsafe { memcmp(dst.as_ptr(), src.as_ptr(), 5) }, 0);
        assert!(unsafe { memcmp(b"a\0".as_ptr(), b"b\0".as_ptr(), 2) } < 0);
    }

    #[test]
    fn memmove_handles_overlap_in_both_directions() {
        let mut buf = *b"abcdefgh";
        // Source above destination: a naive forward copy would corrupt it.
        assert_eq!(unsafe { memmove(buf.as_mut_ptr(), buf[2..].as_ptr(), 4) }, buf.as_mut_ptr());
        assert_eq!(&buf, b"cdefefgh");

        let mut buf = *b"abcdefgh";
        // Source below destination: a naive backward copy would corrupt it.
        unsafe { memmove(buf[2..].as_mut_ptr(), buf.as_ptr(), 4) };
        assert_eq!(&buf, b"ababcdgh");
    }

    #[test]
    fn memset_fills_only_the_requested_range() {
        let mut buf = [0u8; 6];
        assert_eq!(unsafe { memset(buf.as_mut_ptr(), 0x41, 4) }, buf.as_mut_ptr());
        assert_eq!(buf, [0x41, 0x41, 0x41, 0x41, 0, 0]);
        // Only the low byte of `c` is used.
        unsafe { memset(buf[4..].as_mut_ptr(), 0x1FF, 2) };
        assert_eq!(buf, [0x41, 0x41, 0x41, 0x41, 0xFF, 0xFF]);
    }
}
