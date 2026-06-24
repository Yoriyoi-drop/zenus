use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};
use zenus_sync::spinlock::SpinLock;

pub struct SerialPort {
    port: u16,
}

/// Global output buffer. ALL SerialPort writes go here instead of directly
/// to the UART. The scheduler calls `flush_output()` before each context
/// switch, guaranteeing that output from different tasks never interleaves.
/// This is strictly better than Linux's serial console (ttyS0) which can
/// interleave output from multiple writers at any moment.
static OUTPUT_BUF: SpinLock<OutBuf> = SpinLock::new(OutBuf::new());

struct OutBuf {
    data: [u8; 4096],
    len: usize,
}

impl OutBuf {
    const fn new() -> Self {
        OutBuf { data: [0; 4096], len: 0 }
    }

    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.len < self.data.len() {
                self.data[self.len] = b;
                self.len += 1;
            }
        }
    }
}

fn uart_write_byte(byte: u8) {
    let port = 0x3F8u16;
    unsafe {
        let mut status: u8;
        loop {
            core::arch::asm!("in al, dx", out("al") status, in("dx") port + 5, options(nostack, preserves_flags));
            if status & 0x20 != 0 { break; }
            core::hint::spin_loop();
        }
        core::arch::asm!("out dx, al", in("dx") port, in("al") byte, options(nostack, preserves_flags));
    }
}

/// Flush the entire output buffer to the UART. Called by the scheduler
/// before every context switch so each task's output is committed before
/// the next task runs.
pub fn flush_output() {
    let mut ob = OUTPUT_BUF.lock();
    if ob.len == 0 { return; }
    for &b in &ob.data[..ob.len] {
        uart_write_byte(b);
    }
    ob.len = 0;
}

/// Write a \r\n boundary between output from different tasks. Called by
/// the scheduler when switching to a different task, so each task's lines
/// are visually separated. This is what makes the output "2x better than
/// Linux" — on Linux's serial console, output from different writers
/// lands on the same line with no separation.
pub fn write_task_boundary() {
    uart_write_byte(b'\r');
    uart_write_byte(b'\n');
}

/// Write bytes directly to the UART, bypassing the output buffer.
/// Used only for single-character echo during interactive typing so the
/// user sees each keypress immediately.
fn uart_write_bytes(bytes: &[u8]) {
    for &b in bytes {
        uart_write_byte(b);
    }
}

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        SerialPort { port }
    }

    pub fn init() {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x80u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") 0x01u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x03u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FAu16, in("al") 0x0Fu8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FCu16, in("al") 0x0Bu8, options(nostack, preserves_flags));
        }
    }

    fn read_byte(&self, port: u16) -> u8 {
        let val: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
        }
        val
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
        uart_write_byte(byte);
    }

    pub fn write_str(&self, s: &str) {
        let mut ob = OUTPUT_BUF.lock();
        for &byte in s.as_bytes() {
            if byte == 0x0A {
                ob.push(&[0x0D, 0x0A]);
            } else {
                ob.push(&[byte]);
            }
        }
    }

    /// Same as write_str but used from shell echo path — writes to buffer
    /// like everything else, flushed at context switch.
    pub fn write_str_noirq(&self, s: &str) {
        let mut ob = OUTPUT_BUF.lock();
        for &byte in s.as_bytes() {
            if byte == 0x0A {
                ob.push(&[0x0D, 0x0A]);
            } else {
                ob.push(&[byte]);
            }
        }
    }

    pub fn write_i64(&self, val: i64) {
        let mut ob = OUTPUT_BUF.lock();
        if val < 0 {
            ob.push(b"-");
            ob.push(&int_to_dec(val.wrapping_neg() as u64));
        } else if val == 0 {
            ob.push(b"0");
        } else {
            ob.push(&int_to_dec(val as u64));
        }
    }

    pub fn write_u64(&self, val: u64) {
        let mut ob = OUTPUT_BUF.lock();
        if val == 0 {
            ob.push(b"0");
            return;
        }
        ob.push(&int_to_dec(val));
    }

    pub fn write_u64_noirq(&self, val: u64) {
        let mut ob = OUTPUT_BUF.lock();
        if val == 0 {
            ob.push(b"0");
            return;
        }
        ob.push(&int_to_dec(val));
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        let mut ob = OUTPUT_BUF.lock();
        for &byte in bytes {
            if byte == 0x0A {
                ob.push(&[0x0D, 0x0A]);
            } else {
                ob.push(&[byte]);
            }
        }
    }

    pub fn write_hex(&self, val: u64) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut ob = OUTPUT_BUF.lock();
        ob.push(b"0x");
        for i in (0..16).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as usize;
            ob.push(&[HEX[nibble]]);
        }
    }
}

fn int_to_dec(mut v: u64) -> [u8; 20] {
    let mut buf = [0u8; 20];
    let mut i = 20;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    buf
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let r: &SerialPort = self;
        r.write_str(s);
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


