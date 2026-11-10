#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

use zenus_fs::vfs::{self, FileType};
use zutils_common::{Args, Reader, Writer};

pub fn execute<W: Writer + ?Sized, R: Reader + ?Sized>(args: &Args, r: &mut R, w: &mut W) {
    let pattern = match args.get(1) {
        Some(p) => p,
        None => {
            w.write_str("grep: missing pattern\r\n");
            return;
        }
    };

    match args.get(2) {
        Some(path) => match vfs::open(path) {
            Some(node) => {
                let stat = node.fs.stat(node.inode);
                if stat.file_type == FileType::Directory {
                    w.write_str("grep: ");
                    w.write_str(path);
                    w.write_str(": Is a directory\r\n");
                    return;
                }
                let mut buf = [0u8; 512];
                let mut offset: u64 = 0;
                let mut line_buf: [u8; 256] = [0; 256];
                let mut line_pos = 0;
                loop {
                    match node.fs.read(node.inode, offset, &mut buf) {
                        Some(0) | None => break,
                        Some(n) => {
                            let n = n as usize;
                            for &b in &buf[..n] {
                                if b == b'\n' {
                                    let line = core::str::from_utf8(&line_buf[..line_pos])
                                        .unwrap_or("");
                                    if line.contains(pattern) {
                                        w.write_str(line);
                                        w.write_str("\r\n");
                                    }
                                    line_pos = 0;
                                } else if line_pos < 256 {
                                    line_buf[line_pos] = b;
                                    line_pos += 1;
                                }
                            }
                            offset += n as u64;
                        }
                    }
                }
                if line_pos > 0 {
                    let line = core::str::from_utf8(&line_buf[..line_pos]).unwrap_or("");
                    if line.contains(pattern) {
                        w.write_str(line);
                        w.write_str("\r\n");
                    }
                }
            }
            None => {
                w.write_str("grep: ");
                w.write_str(path);
                w.write_str(": not found\r\n");
            }
        },
        // No file argument: read from stdin (a pipeline stage).
        None => {
            let mut line_buf: [u8; 256] = [0; 256];
            let mut line_pos = 0;
            while let Some(b) = r.read_byte() {
                if b == b'\n' {
                    let line = core::str::from_utf8(&line_buf[..line_pos]).unwrap_or("");
                    if line.contains(pattern) {
                        w.write_str(line);
                        w.write_str("\r\n");
                    }
                    line_pos = 0;
                } else if line_pos < 256 {
                    line_buf[line_pos] = b;
                    line_pos += 1;
                }
            }
            if line_pos > 0 {
                let line = core::str::from_utf8(&line_buf[..line_pos]).unwrap_or("");
                if line.contains(pattern) {
                    w.write_str(line);
                    w.write_str("\r\n");
                }
            }
        }
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use zutils_common::{OutputBuf, SliceReader};

    #[test]
    fn grep_reads_stdin_and_filters() {
        let input = b"apple\nbanana\ncherry\n";
        let args = Args::parse("grep an");
        let mut r = SliceReader::new(input);
        let mut store = [0u8; 64];
        let len;
        {
            let mut w = OutputBuf::new(&mut store);
            execute(&args, &mut r, &mut w);
            len = w.len();
        }
        assert_eq!(&store[..len], b"banana\r\n");
    }

    #[test]
    fn grep_missing_pattern_errors() {
        let args = Args::parse("grep");
        let mut r = SliceReader::empty();
        let mut store = [0u8; 64];
        let len;
        {
            let mut w = OutputBuf::new(&mut store);
            execute(&args, &mut r, &mut w);
            len = w.len();
        }
        let out = core::str::from_utf8(&store[..len]).unwrap();
        assert!(out.contains("missing pattern"));
    }
}

