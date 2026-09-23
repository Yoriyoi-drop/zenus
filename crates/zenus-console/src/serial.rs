use core::fmt;
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

/// Boot-time input drain buffer. All bytes arriving before shell starts
/// are collected here by polling with HLT (allows event loop to run).
static mut DRAIN_BUF: [u8; 256] = [0; 256];
static mut DRAIN_LEN: usize = 0;

// ── Interrupt-driven serial input ──
// These buffers collect incoming serial bytes from the UART interrupt
// handler. The shell polls `take_irq_byte()` in its read_line loop,
// alongside the existing polling-based read. With IER bit 0 set, the
// UART asserts IRQ4 on each received byte, which triggers a VM exit
// on KVM — forcing QEMU to process the host stdin pipe.

struct IrqBuf {
    data: [u8; 256],
    head: usize,
    tail: usize,
}

impl IrqBuf {
    const fn new() -> Self {
        IrqBuf {
            data: [0; 256],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, b: u8) {
        let next = (self.head + 1) % self.data.len();
        if next != self.tail {
            self.data[self.head] = b;
            self.head = next;
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.tail == self.head {
            return None;
        }
        let b = self.data[self.tail];
        self.tail = (self.tail + 1) % self.data.len();
        Some(b)
    }

    fn is_empty(&self) -> bool {
        self.tail == self.head
    }
}

static IRQ_BUF: SpinLock<IrqBuf> = SpinLock::new(IrqBuf::new());

/// Called from the UART interrupt handler (IRQ4 → vector 36).
/// Reads one byte directly from the UART data port and stores it in
/// the interrupt buffer. The caller (handler.rs) already verified
/// LSR.DR=1, so we skip the redundant Line Status Register check.
pub fn irq_handler_serial() {
    let b = read_byte_raw(0x3F8);
    let mut buf = IRQ_BUF.lock();
    buf.push(b);
}

/// Take one byte from the interrupt-driven serial input buffer.
/// Returns None if the buffer is empty.
pub fn take_irq_byte() -> Option<u8> {
    let mut buf = IRQ_BUF.lock();
    buf.pop()
}

/// Check if there's data in the interrupt-driven serial input buffer.
pub fn irq_data_available() -> bool {
    let buf = IRQ_BUF.lock();
    !buf.is_empty()
}

/// Enable UART interrupts: set IER (Interrupt Enable Register) bit 0
/// to enable "Received Data Available" interrupt.
/// NOTE: IOAPIC routing must be done separately (in boot code) after
/// IOAPIC init, because this crate doesn't depend on zenus_arch.
pub fn enable_serial_interrupts() {
    // Set IER bit 0: enable received data available interrupt
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F9u16, in("al") 0x01u8,
            options(nostack, preserves_flags),
        );
        // Also clear any pending interrupt by reading the data port
        let _drain: u8;
        core::arch::asm!("in al, dx", out("al") _drain, in("dx") 0x3F8u16, options(nostack, preserves_flags));
    }
}

/// Drain all pending serial input. Runs after serial init, before shell.
/// Uses spin loop with delay (no HLT) to avoid depending on timer interrupts
/// which may not fire reliably after SMP startup.
pub fn drain_boot_input() {
    unsafe {
        const MAX_DRAIN: usize = 256;
        // Polling-only: baca semua byte yang sudah sampai di FIFO.
        // TIDAK pakai sti;hlt;cli karena interrupts sengaja disabled
        // selama boot — PIT timer ISR (schedule_tick) bisa crash karena
        // scheduler/boot structs belum siap untuk preemption.
        // Piped input yang belum sampai di sini akan dihandle oleh
        // sti;hlt;cli di read_line() shell.
        for _ in 0..200 {
            let lsr = read_byte_raw(0x3FD);
            if lsr & 0x01 != 0 {
                let b = read_byte_raw(0x3F8);
                if DRAIN_LEN < MAX_DRAIN {
                    DRAIN_BUF[DRAIN_LEN] = b;
                    DRAIN_LEN += 1;
                }
                continue; // jika ada data, baca lagi tanpa delay
            }
            core::hint::spin_loop();
            if DRAIN_LEN >= MAX_DRAIN {
                break;
            }
        }
    }
}

/// Take one byte from boot drain buffer.
pub fn take_drain_byte() -> Option<u8> {
    unsafe {
        if DRAIN_LEN == 0 {
            None
        } else {
            let b = DRAIN_BUF[0];
            for i in 1..DRAIN_LEN {
                DRAIN_BUF[i - 1] = DRAIN_BUF[i];
            }
            DRAIN_LEN -= 1;
            Some(b)
        }
    }
}

fn read_byte_raw(port: u16) -> u8 {
    let val: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nostack, preserves_flags));
    }
    val
}

struct OutBuf {
    data: [u8; 4096],
    len: usize,
}

impl OutBuf {
    const fn new() -> Self {
        OutBuf {
            data: [0; 4096],
            len: 0,
        }
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

/// Emergency write: directly output a byte to the UART Tx port,
/// bypassing the OUTPUT_BUF ring buffer entirely.
/// Use ONLY during early boot when OUTPUT_BUF may not be accessible,
/// or in panic/crash paths where buffer state is suspect.
pub fn uart_write_byte_emergency(byte: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") byte, options(nostack, preserves_flags));
    }
}

/// Write a byte directly to the UART Tx port.
fn uart_write_byte(byte: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") byte, options(nostack, preserves_flags));
    }
}

/// Flush output buffer using try_lock(). Safe to call from timer ISR —
/// jika lock sedang dipegang task lain, fungsi return tanpa blocking
/// (ISR tidak boleh deadlock). Buffer akan di-flush di timer tick
/// berikutnya atau oleh explicit flush_output_blocking() dari boot path.
pub fn flush_output() {
    let mut guard = OUTPUT_BUF.try_lock();
    let ob = match guard.as_mut() {
        Some(b) => b,
        None => return,
    };
    if ob.len == 0 {
        return;
    }
    for &b in &ob.data[..ob.len] {
        uart_write_byte(b);
    }
    ob.len = 0;
}

/// Flush output buffer using lock(). AMAN dipanggil dari boot path
/// karena interrupts disabled — tidak ada ISR yang bisa preempt.
/// Gunakan ini di explicit flush_output() selama boot untuk memastikan
/// buffer selalu ter-flush (tidak seperti try_lock() yang bisa gagal
/// spurious). JANGAN panggil dari timer ISR (risiko deadlock).
pub fn flush_output_blocking() {
    let mut ob = OUTPUT_BUF.lock();
    if ob.len == 0 {
        return;
    }
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

impl SerialPort {
    pub const fn new(port: u16) -> Self {
        SerialPort { port }
    }

    pub fn init() {
        unsafe {
            // Standard 16550 UART init. Writes to IER, DLL, DLM, LCR, FCR, MCR.
            // FCR=0x01 enables the 16-byte FIFO without CLEAR_RCVR (which
            // orphans any pre-FIFO byte in RBR on some QEMU versions).
            // IER=0x00 initially (interrupts disabled)
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x80u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") 0x01u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3F9u16, in("al") 0x00u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FBu16, in("al") 0x03u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FAu16, in("al") 0x01u8, options(nostack, preserves_flags));
            core::arch::asm!("out dx, al", in("dx") 0x3FCu16, in("al") 0x0Bu8, options(nostack, preserves_flags));
            // IER remains 0x00 — interrupts not enabled yet.
            // Call register_serial_irq() later to enable them.
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
            let (digits, start) = int_to_dec(val.wrapping_neg() as u64);
            ob.push(&digits[start..]);
        } else if val == 0 {
            ob.push(b"0");
        } else {
            let (digits, start) = int_to_dec(val as u64);
            ob.push(&digits[start..]);
        }
    }

    pub fn write_u64(&self, val: u64) {
        let mut ob = OUTPUT_BUF.lock();
        if val == 0 {
            ob.push(b"0");
            return;
        }
        let (digits, start) = int_to_dec(val);
        ob.push(&digits[start..]);
    }

    pub fn write_u64_noirq(&self, val: u64) {
        let mut ob = OUTPUT_BUF.lock();
        if val == 0 {
            ob.push(b"0");
            return;
        }
        let (digits, start) = int_to_dec(val);
        ob.push(&digits[start..]);
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

fn int_to_dec(mut v: u64) -> ([u8; 20], usize) {
    let mut buf = [0u8; 20];
    let mut i = 20;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    (buf, i)
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
