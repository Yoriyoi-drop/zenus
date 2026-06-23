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
}

static KEYBOARD: SpinLock<KeyboardState> = SpinLock::new(KeyboardState {
    shift: false,
    caps: false,
    buf: [0; 256],
    read_idx: 0,
    write_idx: 0,
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

    let s = zenus_console::serial::SerialPort::new(0x3F8);

    // Route IRQ1 through IOAPIC for APIC mode
    if crate::interrupts::ioapic::is_initialized() {
        let vector = 33u8;
        let apic_id = crate::interrupts::apic::current_apic_id() as u8;
        if crate::interrupts::ioapic::route_irq(1, vector, apic_id) {
            s.write_str("[IOAPIC] Keyboard IRQ1 -> vector 33\n");
        } else {
            s.write_str("[IOAPIC] Keyboard IRQ1 -> FAILED\n");
        }
    }

    s.write_str("[OK] PS/2 Keyboard initialized\n");
}

pub fn handle_irq1() {
    let mut data = Port::<u8>::new(KB_DATA);
    let scancode: u8;
    unsafe {
        scancode = data.read();
    }

    if scancode == 0xE0 {
        return;
    }

    let key_down = (scancode & 0x80) == 0;
    let key = scancode & 0x7F;

    let mut kbd = KEYBOARD.lock();
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
            let wi = kbd.write_idx;
            let next = (wi + 1) % 256;
            if next != kbd.read_idx {
                kbd.buf[wi] = c;
                kbd.write_idx = next;
            }
        }
        KEY_PRESSED.store(true, Ordering::Release);
    }
}

pub fn read_key() -> Option<u8> {
    let mut kbd = KEYBOARD.lock();
    if kbd.read_idx != kbd.write_idx {
        let c = kbd.buf[kbd.read_idx];
        kbd.read_idx = (kbd.read_idx + 1) % kbd.buf.len();
        let avail = kbd.read_idx != kbd.write_idx;
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
    kbd.read_idx != kbd.write_idx
}
