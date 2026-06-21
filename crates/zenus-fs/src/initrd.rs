use core::slice;
use zenus_console::serial::SerialPort;

#[repr(C, packed)]
struct UstarHeader {
    name: [u8; 100],
    mode: [u8; 8],
    uid: [u8; 8],
    gid: [u8; 8],
    size: [u8; 12],
    mtime: [u8; 12],
    checksum: [u8; 8],
    type_flag: u8,
    link_name: [u8; 100],
    magic: [u8; 6],
    version: [u8; 2],
    uname: [u8; 32],
    gname: [u8; 32],
    dev_major: [u8; 8],
    dev_minor: [u8; 8],
    prefix: [u8; 155],
    padding: [u8; 12],
}

fn parse_octal(buf: &[u8]) -> u64 {
    let s = core::str::from_utf8(buf).unwrap_or("0");
    u64::from_str_radix(s.trim_end_matches('\0'), 8).unwrap_or(0)
}

pub fn load_initrd(addr: u64, len: u64) {
    let mut s = SerialPort::new(0x3F8);
    s.write_str("[INITRD] Loading at ");
    s.write_hex(addr);
    s.write_str(" size ");
    s.write_u64(len);
    s.write_str("...\n");

    let data = unsafe { slice::from_raw_parts(addr as *const u8, len as usize) };
    let mut offset = 0usize;

    while offset + 512 <= len as usize {
        let header = unsafe { &*(data.as_ptr().add(offset) as *const UstarHeader) };

        let magic = core::str::from_utf8(&header.magic).unwrap_or("");
        if magic != "ustar" {
            break;
        }

        let name = core::str::from_utf8(&header.name).unwrap_or("");
        let file_size = parse_octal(&header.size);
        let name = name.trim_end_matches('\0');
        let entry_type = header.type_flag;

        if !name.is_empty() {
            let type_str = match entry_type {
                b'0' | b'\0' => "FILE",
                b'5' => "DIR ",
                _ => "OTHER",
            };
            s.write_str("  ");
            s.write_str(type_str);
            s.write_str(" ");
            s.write_str(name);
            s.write_str(" (");
            s.write_u64(file_size);
            s.write_str(" bytes)\n");
        }

        offset += 512;
        if entry_type == b'0' || entry_type == b'\0' {
            offset += ((file_size + 511) / 512) as usize * 512;
        }
    }

    s.write_str("[OK] Initrd loaded\n");
}
