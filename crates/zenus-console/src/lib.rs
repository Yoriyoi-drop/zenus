#![no_std]
#![allow(static_mut_refs)]

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod display;
pub mod error;
pub mod fb;
pub mod log;
pub mod serial;
pub mod syslog;
pub mod vga;

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// Only pure-logic paths: formatting buffers, syslog ring accounting, and the
/// error-code catalog. Anything touching UART ports, VGA memory, or the
/// framebuffer stays out — those fault in ring 3.
#[cfg(test)]
mod host_tests {
    use crate::error::{self, ErrorModule};
    use crate::log::{LogBuf, LogLevel};
    use core::fmt::Write;

    #[test]
    fn logbuf_formats_and_reports_truncation() {
        let mut buf = LogBuf::new();
        let _ = write!(&mut buf, "cpu={} mem={}", 4, 2048);
        assert_eq!(buf.as_str(), "cpu=4 mem=2048");
        assert!(!buf.was_truncated());

        // 512-byte buffer: a 600-byte write must keep the head and flag it.
        let mut big = LogBuf::new();
        let long = "x".repeat(600);
        let _ = big.write_str(&long);
        assert_eq!(big.as_str().len(), 512);
        assert!(big.was_truncated());
        assert!(big.as_str().bytes().all(|b| b == b'x'));
    }

    #[test]
    fn log_levels_have_distinct_prefixes() {
        let levels = [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Notice,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Critical,
            LogLevel::Fatal,
            LogLevel::Panic,
        ];
        for (i, a) in levels.iter().enumerate() {
            assert!(!a.prefix().is_empty());
            for b in levels.iter().skip(i + 1) {
                assert_ne!(a.prefix(), b.prefix(), "prefix collision");
            }
        }
        assert_eq!(LogLevel::Info.prefix(), "INFO ");
    }

    #[test]
    fn error_catalog_is_consistent() {
        let catalog = error::catalog();
        assert!(!catalog.is_empty());

        // No duplicate code strings.
        for (i, a) in catalog.iter().enumerate() {
            assert!(!a.code.is_empty());
            assert!(!a.desc.is_empty());
            // "ZN-<PREFIX>-NNNN", and the PREFIX matches the module.
            let rest = a.code.strip_prefix("ZN-").expect("code starts with ZN-");
            let dash = rest.find('-').expect("code has second dash");
            let (prefix, suffix) = (&rest[..dash], &rest[dash + 1..]);
            assert_eq!(prefix, a.module.prefix(), "code/module mismatch: {}", a.code);
            assert_eq!(suffix.len(), 4, "bad code shape: {}", a.code);
            assert!(suffix.bytes().all(|b| b.is_ascii_digit()), "bad code shape: {}", a.code);
            for b in catalog.iter().skip(i + 1) {
                assert_ne!(a.code, b.code, "duplicate code {}", a.code);
            }
        }
    }

    #[test]
    fn error_modules_have_distinct_prefixes_and_names() {
        let modules = [
            ErrorModule::Kernel,
            ErrorModule::Memory,
            ErrorModule::FileSystem,
            ErrorModule::Process,
            ErrorModule::Driver,
            ErrorModule::Network,
            ErrorModule::Security,
        ];
        for (i, a) in modules.iter().enumerate() {
            for b in modules.iter().skip(i + 1) {
                assert_ne!(a.prefix(), b.prefix());
                assert_ne!(a.name(), b.name());
            }
        }
    }

    #[test]
    fn error_counters_are_per_module_not_per_digits() {
        // Regression test: the old counter hashed only the trailing digits, so
        // ZN-KRN-0001 and ZN-FS-0001 shared a slot. Use private high-numbered
        // codes so no other test touches these slots.
        error::record_error(LogLevel::Error, Some("ZN-KRN-9021"), "test", "a", "t.rs", 1);
        error::record_error(LogLevel::Error, Some("ZN-KRN-9021"), "test", "b", "t.rs", 2);
        error::record_error(LogLevel::Error, Some("ZN-FS-9021"), "test", "c", "t.rs", 3);
        assert_eq!(error::get_error_count("ZN-KRN-9021"), 2);
        assert_eq!(error::get_error_count("ZN-FS-9021"), 1);

        // Malformed codes land in the unknown slot without panicking.
        error::record_error(LogLevel::Error, Some("bogus"), "test", "d", "t.rs", 4);
        assert_eq!(error::get_error_count("bogus"), 1);
    }

    #[test]
    fn syslog_ring_counts_and_orders_entries() {
        use crate::syslog;
        syslog::syslog_init();

        let before = syslog::syslog_get_count();
        syslog::syslog_write(LogLevel::Info, "test", "first");
        syslog::syslog_write(LogLevel::Warn, "test", "second");
        assert_eq!(syslog::syslog_get_count(), before + 2);

        let last = syslog::syslog_get(before + 1).expect("newest readable");
        assert_eq!(crate::syslog::syslog_msg_str(&last), "second");
        assert!(syslog::syslog_get(before + 2).is_none(), "past the end");
    }
}
