#![no_std]

use zutils_common::{Args, Writer};
use zenus_fs::vfs::{self, FileSystem as _, FileType};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let pattern = match args.get(1) {
        Some(p) => p,
        None => {
            w.write_str("grep: missing pattern\r\n");
            return;
        }
    };
    let path = match args.get(2) {
        Some(p) => p,
        None => {
            w.write_str("grep: missing file\r\n");
            return;
        }
    };

    match vfs::open(path) {
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
                        for i in 0..n as usize {
                            let b = buf[i];
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
                        offset += n;
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
    }
}
