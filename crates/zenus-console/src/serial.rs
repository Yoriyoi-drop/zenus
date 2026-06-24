use core::fmt;
use zenus_sync::spinlock::SpinLock;

pub struct SerialPort {
    port: u16,
}

/// Global serial I/O mutex with IRQ disable to prevent deadlock when
/// interrupt handlers (page fault, GPF, etc.) also write to serial.
/// SpinLock::lock() disables interrupts while held.
static SERIAL_LOCK: SpinLock<()> = SpinLock::new(());

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

    fn tx_ready(&self) -> bool {
        self.read_byte(self.port + 5) & 0x20 != 0
    }

    fn wait_tx_ready(&self) {
        while !self.tx_ready() {
            core::hint::spin_loop();
        }
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
        self.wait_tx_ready();
        self.write_byte(byte);
    }

    fn write_batch_raw(&self, bytes: &[u8]) {
        for &b in bytes {
            self.write_byte(b);
        }
        self.wait_tx_ready();
    }

    pub fn write_str(&self, s: &str) {
        let _guard = SERIAL_LOCK.lock();
        let mut buf = [0u8; 16];
        let mut pos = 0usize;
        for byte in s.bytes() {
            if byte == 0x0A {
                if pos + 2 > 16 {
                    self.write_batch_raw(&buf[..pos]);
                    pos = 0;
                }
                buf[pos] = 0x0D; pos += 1;
                buf[pos] = 0x0A; pos += 1;
            } else {
                if pos >= 16 {
                    self.write_batch_raw(&buf[..pos]);
                    pos = 0;
                }
                buf[pos] = byte; pos += 1;
            }
        }
        if pos > 0 {
            self.write_batch_raw(&buf[..pos]);
        }
    }

    /// Like write_str but uses lock_no_irq() so interrupts stay enabled
    /// during serial I/O. Use from non-ISR context (e.g. shell).
    pub fn write_str_noirq(&self, s: &str) {
        let _guard = SERIAL_LOCK.lock_no_irq();
        let mut buf = [0u8; 16];
        let mut pos = 0usize;
        for byte in s.bytes() {
            if byte == 0x0A {
                if pos + 2 > 16 {
                    self.write_batch_raw(&buf[..pos]);
                    pos = 0;
                }
                buf[pos] = 0x0D; pos += 1;
                buf[pos] = 0x0A; pos += 1;
            } else {
                if pos >= 16 {
                    self.write_batch_raw(&buf[..pos]);
                    pos = 0;
                }
                buf[pos] = byte; pos += 1;
            }
        }
        if pos > 0 {
            self.write_batch_raw(&buf[..pos]);
        }
    }

    pub fn write_i64(&self, val: i64) {
        if val < 0 {
            self.write_byte_serial(b'-');
            self.write_u64(val.wrapping_neg() as u64);
        } else {
            self.write_u64(val as u64);
        }
    }

    pub fn write_u64(&self, val: u64) {
        let mut buf = [0u8; 20];
        let mut i = 0;
        let mut v = val;
        let _guard = SERIAL_LOCK.lock();
        if v == 0 {
            self.write_byte_serial(b'0');
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
    }

    pub fn write_u64_noirq(&self, val: u64) {
        let mut buf = [0u8; 20];
        let mut i = 0;
        let mut v = val;
        let _guard = SERIAL_LOCK.lock_no_irq();
        if v == 0 {
            self.write_byte_serial(b'0');
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
    }

    pub fn write_bytes(&self, buf: &[u8]) {
        let _guard = SERIAL_LOCK.lock();
        for chunk in buf.chunks(16) {
            for &byte in chunk {
                match byte {
                    0x0A => {
                        self.write_byte(0x0D);
                        self.write_byte(0x0A);
                    }
                    _ => self.write_byte(byte),
                }
            }
            self.wait_tx_ready();
        }
    }

    pub fn write_hex(&self, val: u64) {
        let hex = b"0123456789ABCDEF";
        let _guard = SERIAL_LOCK.lock();
        self.write_byte_serial(b'0');
        self.write_byte_serial(b'x');
        for i in (0..16).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as usize;
            self.write_byte_serial(hex[nibble]);
        }
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Use &self to disambiguate from this trait method.
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
