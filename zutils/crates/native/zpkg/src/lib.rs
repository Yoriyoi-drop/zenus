#![no_std]

extern crate alloc;

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let subcmd = args.get(1).unwrap_or("list");
    match subcmd {
        "list" | "ls" => {
            let pkgs = zenus_fs::pkg::pkg_list();
            if pkgs.is_empty() {
                w.write_str("No packages installed\r\n");
                return;
            }
            w.write_str("Installed packages:\r\n");
            for pkg in pkgs {
                w.write_str("  ");
                w.write_str(&pkg.name);
                w.write_str(" v");
                w.write_str(&pkg.version);
                w.write_str(" (");
                w.write_u64(pkg.file_count as u64);
                w.write_str(" files)\r\n");
            }
        }
        "info" => {
            let name = match args.get(2) {
                Some(n) => n,
                None => { w.write_str("Usage: zpkg info <name>\r\n"); return; }
            };
            match zenus_fs::pkg::pkg_info(name) {
                Some(info) => {
                    w.write_str("Package: "); w.write_str(&info.name); w.write_str("\r\n");
                    w.write_str("Version: "); w.write_str(&info.version); w.write_str("\r\n");
                    w.write_str("Files:   "); w.write_u64(info.file_count as u64); w.write_str("\r\n");
                    for f in &info.files {
                        w.write_str("  "); w.write_str(f); w.write_str("\r\n");
                    }
                }
                None => { w.write_str("Package not found\r\n"); }
            }
        }
        "install" | "i" => {
            let path = match args.get(2) {
                Some(p) => p,
                None => { w.write_str("Usage: zpkg install <path>\r\n"); return; }
            };
            let node = match zenus_fs::vfs::open(path) {
                Some(n) => n,
                None => { w.write_str("zpkg: file not found\r\n"); return; }
            };
            let stat = node.fs.stat(node.inode);
            let size = stat.size as usize;
            if size == 0 || size > 65536 { w.write_str("zpkg: invalid file size\r\n"); return; }
            let mut buf = alloc::vec![0u8; size];
            if node.fs.read(node.inode, 0, &mut buf).is_none() { w.write_str("zpkg: read failed\r\n"); return; }
            if zenus_fs::pkg::pkg_install(&buf, 0) {
                w.write_str("Installed successfully\r\n");
            } else {
                w.write_str("Installation failed\r\n");
            }
        }
        "remove" | "rm" => {
            let name = match args.get(2) {
                Some(n) => n,
                None => { w.write_str("Usage: zpkg remove <name>\r\n"); return; }
            };
            if zenus_fs::pkg::pkg_remove(name) {
                w.write_str("Removed: "); w.write_str(name); w.write_str("\r\n");
            } else {
                w.write_str("Package not found\r\n");
            }
        }
        _ => {
            w.write_str("Usage: zpkg <list|info|install|remove> [args]\r\n");
        }
    }
}
