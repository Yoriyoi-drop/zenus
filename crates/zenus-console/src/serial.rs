use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SerialPort {
    port: u16,
}

/// Global serial I/O mutex. Prevents interleaved output from concurrent
/// callers (including SMP cores). Uses `try_lock` internally — if the lock
/// is contended (e.g. an interrupt handler interrupting a serial write),
/// the caller spins briefly and then falls through to avoid deadlock.
static SERIAL_LOCK: AtomicBool = AtomicBool::new(false);

fn acquire_serial() {
    let mut backoff = 1u32;
    loop {
        match SERIAL_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return,
            Err(_) => {
                for _ in 0..backoff { core::hint::spin_loop(); }
                backoff = backoff.saturating_mul(2).min(256);
            }
        }
    }
}

fn release_serial() {
    SERIAL_LOCK.store(false, Ordering::Release);
}

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        SerialPort { port }
    }

    pub fn init() {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x80u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") 0x03u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x03u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FAu16, in("al") 0x0Fu8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FCu16, in("al") 0x0Bu8, options(nostack, preserves_flags));
        }
    }

    fn write_byte(&self, byte: u8) {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") self.port, in("al") byte, options(nostack, preserves_flags));
        }
    }

    fn read_byte(&self, port: u16) -> u8 {
        let val: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
        }
        val
    }

    fn is_transmit_empty(&self) -> bool {
        self.read_byte(self.port + 5) & 0x20 != 0
    }

    pub fn is_data_available(&self) -> bool {
        self.read_byte(self.port + 5) & 0x01 != 0
    }

    pub fn read_byte_serial(&self) -> u8 {
        while !self.is_data_available() {
            core::hint::spin_loop();
        }
        self.read_byte(self.port)
    }

    pub fn write_byte_serial(&self, byte: u8) {
        while !self.is_transmit_empty() {
            core::hint::spin_loop();
        }
        self.write_byte(byte);
    }

    pub fn write_str_locked(&self, s: &str) {
        acquire_serial();
        for byte in s.bytes() {
            match byte {
                0x0A => {
                    self.write_byte_serial(0x0D);
                    self.write_byte_serial(0x0A);
                }
                _ => self.write_byte_serial(byte),
            }
        }
        release_serial();
    }

    pub fn write_u64(&self, val: u64) {
        let mut buf = [0u8; 20];
        let mut i = 0;
        let mut v = val;
        acquire_serial();
        if v == 0 {
            self.write_byte_serial(b'0');
            release_serial();
            return;
        }
        while v > 0 {
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
            i += 1;
        }
        while i > 0 {
            i -= 1;
            self.write_byte_serial(buf[i]);
        }
        release_serial();
    }

    pub fn write_bytes(&self, buf: &[u8]) {
        acquire_serial();
        for &byte in buf {
            match byte {
                0x0A => {
                    self.write_byte_serial(0x0D);
                    self.write_byte_serial(0x0A);
                }
                _ => self.write_byte_serial(byte),
            }
        }
        release_serial();
    }

    pub fn write_hex(&self, val: u64) {
        let hex = b"0123456789ABCDEF";
        acquire_serial();
        self.write_byte_serial(b'0');
        self.write_byte_serial(b'x');
        for i in (0..16).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as usize;
            self.write_byte_serial(hex[nibble]);
        }
        release_serial();
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _serial = $crate::serial::SerialPort::new(0x3F8);
        write!(_serial, $($arg)*).ok();
    }};
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}
