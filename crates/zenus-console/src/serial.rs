use core::fmt;

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        SerialPort { port }
    }

    pub fn init() {
        let mut s = SerialPort { port: 0x3F8 };
        s.write_byte(0x00); // Disable interrupts
        s.write_byte(0x80); // Enable DLAB
        s.write_byte(0x03); // Divisor low (38400 baud)
        s.write_byte(0x00); // Divisor high
        s.write_byte(0x03); // 8N1
        s.write_byte(0x0F); // Enable FIFO, clear, 14-byte threshold
        s.write_byte(0x0B); // Enable IRQs, RTS/DSR set
    }

    fn write_byte(&mut self, byte: u8) {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") self.port, in("al") byte);
        }
    }

    fn read_byte(&self, port: u16) -> u8 {
        let val: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") val, in("dx") port);
        }
        val
    }

    fn is_transmit_empty(&self) -> bool {
        self.read_byte(self.port + 5) & 0x20 != 0
    }

    pub fn is_data_available(&self) -> bool {
        self.read_byte(self.port + 5) & 0x01 != 0
    }

    pub fn read_byte_serial(&mut self) -> u8 {
        while !self.is_data_available() {
            core::hint::spin_loop();
        }
        self.read_byte(self.port)
    }

    pub fn write_byte_serial(&mut self, byte: u8) {
        while !self.is_transmit_empty() {
            core::hint::spin_loop();
        }
        self.write_byte(byte);
    }

    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x0A => {
                    self.write_byte_serial(0x0D);
                    self.write_byte_serial(0x0A);
                }
                _ => self.write_byte_serial(byte),
            }
        }
    }

    pub fn write_u64(&mut self, val: u64) {
        let mut buf = [0u8; 20];
        let mut i = 0;
        let mut v = val;
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

    pub fn write_hex(&mut self, val: u64) {
        let hex = b"0123456789ABCDEF";
        self.write_str("0x");
        for i in (0..16).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as usize;
            self.write_byte_serial(hex[nibble]);
        }
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
        let mut _serial = $crate::serial::SerialPort::new(0x3F8);
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
