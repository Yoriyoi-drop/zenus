#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

use zenus_fs::vfs::{self, FileType};
use zutils_common::{Args, Reader, Writer};

pub fn execute<W: Writer + ?Sized, R: Reader + ?Sized>(args: &Args, r: &mut R, w: &mut W) {
    let path = args.args().iter().find(|a| !a.is_empty());

    match path {
        Some(path) => match vfs::open(path) {
            Some(node) => {
                let stat = node.fs.stat(node.inode);
                if stat.file_type == FileType::Directory {
                    w.write_str("cat: ");
                    w.write_str(path);
                    w.write_str(": Is a directory\r\n");
                    return;
                }
                let mut buf = [0u8; 512];
                let mut offset: u64 = 0;
                let mut last_byte: u8 = 0;
                loop {
                    match node.fs.read(node.inode, offset, &mut buf) {
                        Some(0) | None => break,
                        Some(n) => {
                            let n = n as usize;
                            for &b in &buf[..n] {
                                w.write_byte(b);
                                last_byte = b;
                            }
                            offset += n as u64;
                        }
                    }
                }
                if offset > 0 && last_byte != b'\n' {
                    w.write_byte(b'\n');
                }
            }
            None => {
                w.write_str("cat: ");
                w.write_str(path);
                w.write_str(": not found\r\n");
            }
        },
        // No file argument: read from stdin (a pipeline stage or an empty
        // interactive reader). POSIX `cat` with no operands copies stdin.
        None => {
            while let Some(b) = r.read_byte() {
                w.write_byte(b);
            }
        }
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use zutils_common::{OutputBuf, SliceReader};

    #[test]
    fn cat_reads_stdin_from_reader() {
        let input = b"hello\nworld\n";
        let args = Args::parse("cat");
        let mut r = SliceReader::new(input);
        let mut store = [0u8; 64];
        let len;
        {
            let mut w = OutputBuf::new(&mut store);
            execute(&args, &mut r, &mut w);
            len = w.len();
        }
        assert_eq!(&store[..len], input);
    }
}
