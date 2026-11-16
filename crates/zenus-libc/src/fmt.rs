//! Minimal, allocation-free formatted output — the core a printf can be built
//! on.
//!
//! C varargs are neither safe nor available in `no_std` Rust, so conversions
//! take a type-safe [`Arg`] slice instead. The caller owns the output buffer;
//! [`format`] never writes past it. This deliberately supports only the common
//! conversions (`%s`, `%d`, `%i`, `%u`, `%x`, `%X`, `%c`, `%%`) — width,
//! precision and padding can be layered on later without changing the API.

/// One conversion argument. A byte slice is used for strings because user
/// memory may not be UTF-8 and must not be assumed NUL-terminated here.
pub enum Arg<'a> {
    Str(&'a [u8]),
    Int(i64),
    Uint(u64),
    Hex(u64),
    UpperHex(u64),
    Char(u8),
}

struct Sink<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl Sink<'_> {
    fn push(&mut self, b: u8) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    fn push_slice(&mut self, s: &[u8]) {
        for &b in s {
            self.push(b);
        }
    }

    fn push_uint(&mut self, mut value: u64, radix: u8, upper: bool) {
        if value == 0 {
            self.push(b'0');
            return;
        }
        // 64 digits is enough for base 2, the smallest radix we ever use.
        let mut digits = [0u8; 64];
        let mut n = 0;
        while value > 0 {
            let d = (value % radix as u64) as u8;
            digits[n] = if d < 10 {
                b'0' + d
            } else if upper {
                b'A' + (d - 10)
            } else {
                b'a' + (d - 10)
            };
            n += 1;
            value /= radix as u64;
        }
        while n > 0 {
            n -= 1;
            self.push(digits[n]);
        }
    }
}

/// Format `fmt` into `buf`, consuming one [`Arg`] per conversion in order.
///
/// Returns the number of bytes actually written, clamped to `buf.len()`. A
/// conversion with no matching argument, or a specifier this module does not
/// implement, is emitted literally (`%s`) rather than dropped — a malformed
/// format string should be visible, not silently truncate the message.
pub fn format(buf: &mut [u8], fmt: &[u8], args: &[Arg<'_>]) -> usize {
    let mut sink = Sink { buf, len: 0 };
    let mut ai = 0;
    let mut i = 0;
    while i < fmt.len() {
        let b = fmt[i];
        if b != b'%' {
            sink.push(b);
            i += 1;
            continue;
        }
        if i + 1 < fmt.len() && fmt[i + 1] == b'%' {
            sink.push(b'%');
            i += 2;
            continue;
        }

        let spec = fmt.get(i + 1).copied();
        match args.get(ai) {
            Some(arg) => match (spec, arg) {
                (Some(b's'), Arg::Str(s)) => sink.push_slice(s),
                (Some(b'd' | b'i'), Arg::Int(v)) => {
                    if *v < 0 {
                        sink.push(b'-');
                    }
                    // unsigned_abs handles i64::MIN without overflowing.
                    sink.push_uint(v.unsigned_abs(), 10, false);
                }
                (Some(b'u'), Arg::Uint(v)) => sink.push_uint(*v, 10, false),
                (Some(b'x'), Arg::Hex(v)) => sink.push_uint(*v, 16, false),
                (Some(b'X'), Arg::UpperHex(v)) => sink.push_uint(*v, 16, true),
                (Some(b'c'), Arg::Char(c)) => sink.push(*c),
                _ => {
                    sink.push(b'%');
                    if let Some(s) = spec {
                        sink.push(s);
                    }
                }
            },
            None => {
                sink.push(b'%');
                if let Some(s) = spec {
                    sink.push(s);
                }
                sink.push(b'?');
            }
        }
        ai += 1;
        i += 2;
    }
    sink.len
}

#[cfg(test)]
mod host_tests {
    use super::*;

    fn check(fmt: &[u8], args: &[Arg<'_>], expected: &[u8]) {
        let mut buf = [0u8; 128];
        let n = format(&mut buf, fmt, args);
        assert_eq!(&buf[..n], expected);
    }

    #[test]
    fn literal_text_passes_through() {
        check(b"hello", &[], b"hello");
        check(b"", &[], b"");
    }

    #[test]
    fn percent_escape_does_not_consume_an_argument() {
        check(b"100%% done %d", &[Arg::Int(7)], b"100% done 7");
    }

    #[test]
    fn string_char_and_unsigned_conversions() {
        check(
            b"[%s][%c][%u]",
            &[Arg::Str(b"zenus"), Arg::Char(b'Z'), Arg::Uint(42)],
            b"[zenus][Z][42]",
        );
    }

    #[test]
    fn signed_conversion_handles_negative_and_min() {
        check(
            b"%d/%d",
            &[Arg::Int(-12), Arg::Int(i64::MIN)],
            b"-12/-9223372036854775808",
        );
    }

    #[test]
    fn hex_conversions_use_requested_case() {
        check(b"%x %X", &[Arg::Hex(0xABCD), Arg::UpperHex(0xABCD)], b"abcd ABCD");
    }

    #[test]
    fn buffer_overflow_is_clamped_without_panicking() {
        let mut buf = [0u8; 4];
        let n = format(&mut buf, b"abcdef", &[]);
        assert_eq!(n, 4);
        assert_eq!(&buf, b"abcd");
    }

    #[test]
    fn malformed_input_is_visible_not_dropped() {
        check(b"%q", &[], b"%q?");
        check(b"%d", &[Arg::Str(b"x")], b"%d");
    }
}
