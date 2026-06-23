use core::sync::atomic::{AtomicUsize, Ordering};

const VGA_PHYS: u64 = 0xB8000;
const WIDTH: usize = 80;
const HEIGHT: usize = 25;

#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

fn make_attr(fg: Color, bg: Color) -> u8 {
    (bg as u8) << 4 | (fg as u8)
}

fn vga_base(hhdm_offset: u64) -> *mut u8 {
    (VGA_PHYS + hhdm_offset) as *mut u8
}

static ROW: AtomicUsize = AtomicUsize::new(0);
static COL: AtomicUsize = AtomicUsize::new(0);
static ATTR: AtomicUsize = AtomicUsize::new(0);

pub fn init(hhdm_offset: u64) {
    ATTR.store(make_attr(Color::LightGray, Color::Black) as usize, Ordering::Relaxed);
    clear(hhdm_offset);
    ROW.store(0, Ordering::Relaxed);
    COL.store(0, Ordering::Relaxed);
}

pub fn clear(hhdm_offset: u64) {
    let base = vga_base(hhdm_offset);
    let attr = ATTR.load(Ordering::Relaxed) as u8;
    for i in 0..(WIDTH * HEIGHT) {
        let off = (i * 2) as isize;
        unsafe {
            core::ptr::write_volatile(base.offset(off), b' ');
            core::ptr::write_volatile(base.offset(off + 1), attr);
        }
    }
    ROW.store(0, Ordering::Relaxed);
    COL.store(0, Ordering::Relaxed);
}

fn scroll(hhdm_offset: u64) {
    let base = vga_base(hhdm_offset);
    unsafe {
        for row in 1..HEIGHT {
            for col in 0..WIDTH {
                let src_off = ((row * WIDTH + col) * 2) as isize;
                let dst_off = (((row - 1) * WIDTH + col) * 2) as isize;
                core::ptr::write_volatile(
                    base.offset(dst_off),
                    core::ptr::read_volatile(base.offset(src_off)),
                );
                core::ptr::write_volatile(
                    base.offset(dst_off + 1),
                    core::ptr::read_volatile(base.offset(src_off + 1)),
                );
            }
        }
        for col in 0..WIDTH {
            let off = (((HEIGHT - 1) * WIDTH + col) * 2) as isize;
            core::ptr::write_volatile(base.offset(off), b' ');
            core::ptr::write_volatile(base.offset(off + 1), ATTR.load(Ordering::Relaxed) as u8);
        }
    }
}

pub fn write_str(s: &str, hhdm_offset: u64) {
    let base = vga_base(hhdm_offset);
    for byte in s.bytes() {
        match byte {
            b'\n' => {
                COL.store(0, Ordering::Relaxed);
                ROW.fetch_add(1, Ordering::Relaxed);
            }
            b'\r' => {
                COL.store(0, Ordering::Relaxed);
            }
            b'\t' => {
                let tab_stop = 4;
                let spaces = tab_stop - (COL.load(Ordering::Relaxed) % tab_stop);
                for _ in 0..spaces {
                    put_char(base, b' ');
                }
            }
            0x20..=0x7E => {
                put_char(base, byte);
            }
            _ => {}
        }
        let col = COL.load(Ordering::Relaxed);
        if col >= WIDTH {
            COL.store(0, Ordering::Relaxed);
            ROW.fetch_add(1, Ordering::Relaxed);
        }
        if ROW.load(Ordering::Relaxed) >= HEIGHT {
            scroll(hhdm_offset);
            ROW.store(HEIGHT - 1, Ordering::Relaxed);
        }
    }
}

fn put_char(base: *mut u8, byte: u8) {
    let col = COL.load(Ordering::Relaxed);
    let row = ROW.load(Ordering::Relaxed);
    let off = ((row * WIDTH + col) * 2) as isize;
    unsafe {
        core::ptr::write_volatile(base.offset(off), byte);
        core::ptr::write_volatile(base.offset(off + 1), ATTR.load(Ordering::Relaxed) as u8);
    }
    COL.fetch_add(1, Ordering::Relaxed);
}
