use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::instructions::port::Port;
use zenus_sync::spinlock::SpinLock;

const KB_DATA: u16 = 0x60;
const KB_CMD: u16 = 0x64;
const KB_STATUS: u16 = 0x64;

const STATUS_OUTPUT_FULL: u8 = 0x01;
const CMD_ENABLE: u8 = 0xAE;
const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;

struct KeyboardState {
    shift: bool,
    caps: bool,
    buf: [u8; 256],
    read_idx: usize,
    write_idx: usize,
    /// Set when 0xE0 prefix byte was received (extended scancode sequence).
    e0_prefix: bool,
    /// Pending multi-byte sequence to return from read_key() (e.g., ESC [ A for up arrow).
    pending: [u8; 4],
    pending_len: usize,
    pending_pos: usize,
}

static KEYBOARD: SpinLock<KeyboardState> = SpinLock::new(KeyboardState {
    shift: false,
    caps: false,
    buf: [0; 256],
    read_idx: 0,
    write_idx: 0,
    e0_prefix: false,
    pending: [0; 4],
    pending_len: 0,
    pending_pos: 0,
});

static KEY_PRESSED: AtomicBool = AtomicBool::new(false);

const SCANCODE_SET1: [u8; 128] = [
    0, 0x1B, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 0x08, 0x09,
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's',
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v',
    b'b', b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const SCANCODE_SHIFT: [u8; 128] = [
    0, 0x1B, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 0x08, 0x09,
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0, b'A', b'S',
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|', b'Z', b'X', b'C', b'V',
    b'B', b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

pub fn init() {
    let mut status = Port::<u8>::new(KB_STATUS);
    let mut data = Port::<u8>::new(KB_DATA);

    unsafe {
        while (status.read() & STATUS_OUTPUT_FULL) != 0 {
            data.read();
        }

        let mut cmd = Port::<u8>::new(KB_CMD);
        cmd.write(CMD_READ_CONFIG);
        let mut config = data.read();
        config |= 0x01;
        cmd.write(CMD_WRITE_CONFIG);
        data.write(config);

        cmd.write(CMD_ENABLE);

        while (status.read() & STATUS_OUTPUT_FULL) != 0 {
            data.read();
        }
    }

    // Route IRQ1 through IOAPIC for APIC mode
    if crate::interrupts::ioapic::is_initialized() {
        let vector = 33u8;
        let apic_id = crate::interrupts::apic::current_apic_id() as u8;
        if crate::interrupts::ioapic::route_irq(1, vector, apic_id) {
            zenus_console::kinfo!("IOAPIC keyboard IRQ1 -> vector 33");
        } else {
            zenus_console::kwarn!("IOAPIC keyboard IRQ1 -> FAILED");
        }
    }

    zenus_console::kinfo!("PS/2 Keyboard initialized");
}

/// Extended scancode (Set 1) → escape sequence mapping for arrow/navigation keys.
/// Format: byte sequence stored in pending buffer (e.g., [0x1B, 0x5B, 0x41] for up arrow).
fn extended_to_escape(scancode: u8) -> ([u8; 4], usize) {
    match scancode & 0x7F {
        0x48 => ([0x1B, 0x5B, 0x41, 0], 3), // Up    → ESC [ A
        0x50 => ([0x1B, 0x5B, 0x42, 0], 3), // Down  → ESC [ B
        0x4D => ([0x1B, 0x5B, 0x43, 0], 3), // Right → ESC [ C
        0x4B => ([0x1B, 0x5B, 0x44, 0], 3), // Left  → ESC [ D
        0x47 => ([0x1B, 0x5B, 0x48, 0], 3), // Home  → ESC [ H
        0x4F => ([0x1B, 0x5B, 0x46, 0], 3), // End   → ESC [ F
        0x52 => ([0x1B, 0x5B, 0x32, 0x7E], 4), // Insert → ESC [ 2 ~
        0x53 => ([0x1B, 0x5B, 0x33, 0x7E], 4), // Delete → ESC [ 3 ~
        0x49 => ([0x1B, 0x5B, 0x35, 0x7E], 4), // PgUp  → ESC [ 5 ~
        0x51 => ([0x1B, 0x5B, 0x36, 0x7E], 4), // PgDn  → ESC [ 6 ~
        _    => ([0, 0, 0, 0], 0),
    }
}

/// Push a byte into the keyboard circular buffer.
fn push_byte(kbd: &mut KeyboardState, b: u8) {
    let wi = kbd.write_idx;
    let next = (wi + 1) % 256;
    if next != kbd.read_idx {
        kbd.buf[wi] = b;
        kbd.write_idx = next;
    }
}

pub fn handle_irq1() {
    let mut data = Port::<u8>::new(KB_DATA);
    let scancode: u8;
    unsafe {
        scancode = data.read();
    }

    let mut kbd = KEYBOARD.lock();

    // ── Extended scancode (0xE0 prefix) handling ──
    if scancode == 0xE0 {
        kbd.e0_prefix = true;
        return;
    }

    if kbd.e0_prefix {
        kbd.e0_prefix = false;
        // Only handle key-down events for extended keys
        if (scancode & 0x80) == 0 {
            let (seq, len) = extended_to_escape(scancode);
            if len > 0 {
                kbd.pending = seq;
                kbd.pending_len = len;
                kbd.pending_pos = 0;
                KEY_PRESSED.store(true, Ordering::Release);
            }
        }
        return;
    }

    // ── Standard scancode handling ──
    let key_down = (scancode & 0x80) == 0;
    let key = scancode & 0x7F;

    if key == 0x2A || key == 0x36 {
        kbd.shift = key_down;
        return;
    }
    if key == 0x3A && key_down {
        kbd.caps = !kbd.caps;
        return;
    }

    if key_down && key < 128 {
        let base = if kbd.shift { SCANCODE_SHIFT } else { SCANCODE_SET1 };
        let mut c = base[key as usize];
        if kbd.caps && c >= b'a' && c <= b'z' {
            c -= 32;
        } else if kbd.caps && c >= b'A' && c <= b'Z' {
            c += 32;
        }

        if c != 0 {
            push_byte(&mut kbd, c);
        }
        KEY_PRESSED.store(true, Ordering::Release);
    }
}

pub fn read_key() -> Option<u8> {
    let mut kbd = KEYBOARD.lock();
    // Return from pending escape sequence first (e.g., ESC [ A for up arrow)
    if kbd.pending_pos < kbd.pending_len {
        let b = kbd.pending[kbd.pending_pos];
        kbd.pending_pos += 1;
        // Clear pending when fully consumed
        if kbd.pending_pos >= kbd.pending_len {
            kbd.pending_len = 0;
            kbd.pending_pos = 0;
        }
        drop(kbd);
        return Some(b);
    }
    // Then return from keyboard buffer
    if kbd.read_idx != kbd.write_idx {
        let c = kbd.buf[kbd.read_idx];
        kbd.read_idx = (kbd.read_idx + 1) % kbd.buf.len();
        let avail = kbd.read_idx != kbd.write_idx || kbd.pending_pos < kbd.pending_len;
        drop(kbd);
        KEY_PRESSED.store(avail, Ordering::Release);
        Some(c)
    } else {
        drop(kbd);
        KEY_PRESSED.store(false, Ordering::Release);
        None
    }
}

pub fn is_key_available() -> bool {
    let kbd = KEYBOARD.lock();
    kbd.read_idx != kbd.write_idx || kbd.pending_pos < kbd.pending_len
}

/// Direct PS/2 controller polling — bypass IRQ1 entirely.
/// Checks status register (0x64) bit 0 for data ready, reads scancode from 0x60,
/// translates to ASCII via SCANCODE_SET1, pushes into keyboard buffer.
/// Returns true if a byte was pushed.
pub fn poll_ps2_controller() -> bool {
    let status: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") status, in("dx") 0x64u16, options(nostack, preserves_flags));
    }
    if status & 0x01 == 0 {
        return false;
    }
    let scancode: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") scancode, in("dx") 0x60u16, options(nostack, preserves_flags));
    }

    let mut kbd = KEYBOARD.lock();

    if scancode == 0xE0 {
        kbd.e0_prefix = true;
        return true;
    }
    if kbd.e0_prefix {
        kbd.e0_prefix = false;
        if (scancode & 0x80) == 0 {
            let (seq, len) = extended_to_escape(scancode);
            if len > 0 {
                kbd.pending = seq;
                kbd.pending_len = len;
                kbd.pending_pos = 0;
                KEY_PRESSED.store(true, Ordering::Release);
                return true;
            }
        }
        return false;
    }

    let key_down = (scancode & 0x80) == 0;
    let key = scancode & 0x7F;

    if key == 0x2A || key == 0x36 {
        kbd.shift = key_down;
        return false;
    }
    if key == 0x3A && key_down {
        kbd.caps = !kbd.caps;
        return false;
    }

    if key_down && key < 128 {
        let base = if kbd.shift { SCANCODE_SHIFT } else { SCANCODE_SET1 };
        let mut c = base[key as usize];
        if kbd.caps && c >= b'a' && c <= b'z' {
            c -= 32;
        } else if kbd.caps && c >= b'A' && c <= b'Z' {
            c += 32;
        }
        if c != 0 {
            push_byte(&mut kbd, c);
            KEY_PRESSED.store(true, Ordering::Release);
            return true;
        }
    }
    false
}
