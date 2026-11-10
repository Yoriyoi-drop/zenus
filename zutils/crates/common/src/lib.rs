#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub trait Writer {
    fn write_str(&mut self, s: &str);
    fn write_byte(&mut self, b: u8);
    fn write_u64(&mut self, v: u64);
    fn write_i64(&mut self, v: i64);
    fn write_hex(&mut self, v: u64);
    fn write_ip(&mut self, ip: [u8; 4]);
}

/// Source of input for a command. The shell feeds pipeline stages through a
/// `SliceReader` over the previous stage's captured output; interactive use
/// passes an empty reader so `cat`/`grep` with no file simply produce nothing
/// instead of blocking on the console.
pub trait Reader {
    fn read_byte(&mut self) -> Option<u8>;
}

/// In-memory reader over a byte slice. Used to connect pipeline stages: the
/// bytes one stage wrote become the stdin of the next.
pub struct SliceReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SliceReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        SliceReader { data, pos: 0 }
    }

    pub fn empty() -> SliceReader<'static> {
        SliceReader { data: &[], pos: 0 }
    }
}

impl Reader for SliceReader<'_> {
    fn read_byte(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }
}

pub fn write_u64<W: Writer + ?Sized>(w: &mut W, v: u64) {
    if v == 0 {
        w.write_byte(b'0');
        return;
    }
    let mut tmp = [0u8; 20];
    let mut i = 20;
    let mut n = v;
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    w.write_str(core::str::from_utf8(&tmp[i..]).unwrap_or(""));
}

pub fn write_i64<W: Writer + ?Sized>(w: &mut W, v: i64) {
    if v < 0 {
        w.write_byte(b'-');
        write_u64(w, (-v) as u64);
    } else {
        write_u64(w, v as u64);
    }
}

pub fn write_hex<W: Writer + ?Sized>(w: &mut W, v: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut started = false;
    for s in (0..16).rev() {
        let nib = ((v >> (s * 4)) & 0xf) as u8;
        if nib != 0 || started || s == 0 {
            w.write_byte(HEX[nib as usize]);
            started = true;
        }
    }
}

pub fn write_ip<W: Writer + ?Sized>(w: &mut W, ip: [u8; 4]) {
    write_u64(w, ip[0] as u64);
    w.write_byte(b'.');
    write_u64(w, ip[1] as u64);
    w.write_byte(b'.');
    write_u64(w, ip[2] as u64);
    w.write_byte(b'.');
    write_u64(w, ip[3] as u64);
}

#[derive(Clone, Copy)]
pub struct ExitCode(pub i32);

pub const EXIT_SUCCESS: ExitCode = ExitCode(0);
pub const EXIT_FAILURE: ExitCode = ExitCode(1);

const MAX_ARGS: usize = 32;

pub struct Args<'a> {
    pub parts: [&'a str; MAX_ARGS],
    pub count: usize,
    pub cmd: &'a str,
}

impl<'a> Args<'a> {
    pub fn parse(line: &'a str) -> Self {
        let mut parts: [&str; MAX_ARGS] = [""; MAX_ARGS];
        let mut count = 0;
        for arg in line.split_whitespace() {
            if count >= MAX_ARGS {
                break;
            }
            parts[count] = arg;
            count += 1;
        }
        let cmd = if count > 0 { parts[0] } else { "" };
        Args { parts, count, cmd }
    }

    pub fn args(&self) -> &[&'a str] {
        if self.count > 1 {
            &self.parts[1..self.count]
        } else {
            &[]
        }
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.parts[..self.count].contains(&flag)
    }

    pub fn get(&self, index: usize) -> Option<&'a str> {
        if index < self.count {
            Some(self.parts[index])
        } else {
            None
        }
    }
}

pub struct OutputBuf<'a> {
    pub buf: &'a mut [u8],
    pub pos: usize,
}

impl<'a> OutputBuf<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        OutputBuf { buf, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }
}

impl Writer for OutputBuf<'_> {
    fn write_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let avail = self.buf.len().saturating_sub(self.pos);
        let n = bytes.len().min(avail);
        if n > 0 {
            self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
            self.pos += n;
        }
    }

    fn write_byte(&mut self, b: u8) {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
    }

    fn write_u64(&mut self, v: u64) {
        write_u64(self, v);
    }

    fn write_i64(&mut self, v: i64) {
        write_i64(self, v);
    }

    fn write_hex(&mut self, v: u64) {
        write_hex(self, v);
    }

    fn write_ip(&mut self, ip: [u8; 4]) {
        write_ip(self, ip);
    }
}

pub fn writeln<W: Writer + ?Sized>(w: &mut W, s: &str) {
    w.write_str(s);
    w.write_byte(b'\n');
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn slice_reader_yields_all_bytes_then_none() {
        let data = [1u8, 2, 3];
        let mut r = SliceReader::new(&data);
        assert_eq!(r.read_byte(), Some(1));
        assert_eq!(r.read_byte(), Some(2));
        assert_eq!(r.read_byte(), Some(3));
        assert_eq!(r.read_byte(), None);
        assert_eq!(r.read_byte(), None);
    }

    #[test]
    fn empty_reader_returns_none() {
        let mut r = SliceReader::empty();
        assert_eq!(r.read_byte(), None);
        assert_eq!(r.read_byte(), None);
    }

    #[test]
    fn output_buf_collects_str_and_byte() {
        let mut store = [0u8; 16];
        let len;
        {
            let mut buf = OutputBuf::new(&mut store);
            buf.write_str("hi");
            buf.write_byte(b'!');
            len = buf.len();
        }
        assert_eq!(&store[..len], b"hi!");
    }
}

