#![no_std]
#![allow(static_mut_refs)]

extern crate alloc;

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod block_cache;
pub mod cgroup;
pub mod devfs;
pub mod ext2;
pub mod ext2_fsck;
pub mod io_scheduler;
pub mod journal;
pub mod pkg;
pub mod procfs;
pub mod sysctl;
pub mod tarfs;
pub mod tmpfs;
pub mod vfs;

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// The block layer, VFS, procfs, cgroup and sysctl all talk to each other
/// through process-global tables, so nearly every test here takes `serial()`
/// first. That includes the filesystem tests: `vfs::init()` installs tmpfs as
/// the root and there is only one root.
#[cfg(test)]
mod host_tests {
    use crate::block_cache::{bc_flush, bc_read, bc_stats, bc_write};
    use crate::devfs::{block_device_count, register_block_device, BlockDeviceOps};
    use crate::vfs::{
        self, FileStat, FileType, S_IFDIR, S_IFREG, S_IROTH, S_IRGRP, S_IRUSR, S_IWGRP, S_IWOTH,
        S_IWUSR, S_IXGRP, S_IXOTH, S_IXUSR, MAX_PATH_SEGMENTS,
    };
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use zenus_sync::spinlock::{SpinLock, SpinLockGuard};

    static SERIAL: SpinLock<()> = SpinLock::new(());

    fn serial() -> SpinLockGuard<'static, ()> {
        SERIAL.lock()
    }

    /// A clean filesystem for one test.
    ///
    /// `vfs::init()` alone is not enough: tmpfs node slots are never recycled,
    /// so a suite that creates files across a dozen cases exhausts `MAX_NODES`
    /// and every later `create_file` silently fails — which looks exactly like
    /// a bug in whatever the test was actually checking.
    fn fresh_fs() {
        crate::tmpfs::TmpFs::reset();
        vfs::init();
    }

    // ── a fake block device ───────────────────────────────────────────────
    //
    // 64 sectors of RAM behind the devfs `BlockDeviceOps` vtable, so the block
    // cache, the I/O scheduler and the journal can be exercised without ATA.

    const FAKE_SECTORS: usize = 64;
    const SECTOR: usize = 512;

    static FAKE: SpinLock<[[u8; SECTOR]; FAKE_SECTORS]> =
        SpinLock::new([[0u8; SECTOR]; FAKE_SECTORS]);
    static FAKE_WRITES: SpinLock<u64> = SpinLock::new(0);

    fn fake_read(lba: u64, buf: &mut [u8]) -> bool {
        if lba as usize >= FAKE_SECTORS {
            return false;
        }
        let disk = FAKE.lock();
        let len = buf.len().min(SECTOR);
        buf[..len].copy_from_slice(&disk[lba as usize][..len]);
        true
    }

    fn fake_write(lba: u64, buf: &[u8]) -> bool {
        if lba as usize >= FAKE_SECTORS {
            return false;
        }
        {
            let mut disk = FAKE.lock();
            let len = buf.len().min(SECTOR);
            disk[lba as usize][..len].copy_from_slice(&buf[..len]);
        }
        *FAKE_WRITES.lock() += 1;
        true
    }

    /// Register the fake device once and return its devfs index.
    fn fake_device() -> usize {
        // devfs has no unregister, so remember the index in a static and reuse
        // it — registering twice would hand out a second, unused device.
        static INDEX: SpinLock<usize> = SpinLock::new(usize::MAX);
        let mut idx = INDEX.lock();
        if *idx == usize::MAX {
            let before = block_device_count();
            assert!(register_block_device(
                "hostfake",
                BlockDeviceOps {
                    read: fake_read,
                    write: fake_write,
                    size: (FAKE_SECTORS * SECTOR) as u64,
                }
            ));
            *idx = before;
        }
        *idx
    }

    fn fake_sector(lba: u64) -> [u8; SECTOR] {
        let disk = FAKE.lock();
        disk[lba as usize]
    }

    /// Reset the fake disk *and* the block cache that shadows it.
    ///
    /// Zeroing the RAM alone was not enough: a cached line kept serving the
    /// pre-wipe contents, and a dirty line left by an earlier test was flushed
    /// onto the zeroed disk afterwards, so tests passed by luck.
    fn wipe_fake_device() {
        assert!(
            crate::block_cache::bc_invalidate_all(),
            "could not flush the cache before wiping"
        );
        *FAKE.lock() = [[0u8; SECTOR]; FAKE_SECTORS];
        *FAKE_WRITES.lock() = 0;
    }

    // ── permission model ──────────────────────────────────────────────────

    fn stat(uid: u32, gid: u32, mode: u16, file_type: FileType) -> FileStat {
        FileStat {
            size: 0,
            file_type,
            inode: 1,
            blocks: 0,
            uid,
            gid,
            mode,
        }
    }

    #[test]
    fn access_check_root_bypasses_everything() {
        let readonly = stat(1000, 1000, S_IFREG | 0o400, FileType::File);
        assert!(vfs::access_check(0, 0, 0, 0, &readonly, false));
        assert!(vfs::access_check(0, 0, 0, 0, &readonly, true));
    }

    #[test]
    fn access_check_owner_uses_user_bits() {
        let rw = stat(1000, 1000, S_IFREG | S_IRUSR | S_IWUSR, FileType::File);
        assert!(vfs::access_check(1000, 2000, 1000, 2000, &rw, false));
        assert!(vfs::access_check(1000, 2000, 1000, 2000, &rw, true));

        let read_only = stat(1000, 1000, S_IFREG | S_IRUSR, FileType::File);
        assert!(vfs::access_check(1000, 2000, 1000, 2000, &read_only, false));
        assert!(!vfs::access_check(1000, 2000, 1000, 2000, &read_only, true));
    }

    #[test]
    fn access_check_group_and_other_bits() {
        let group_rw = stat(1000, 2000, S_IFREG | S_IRGRP | S_IWGRP, FileType::File);
        // egid matches, euid does not -> group bits apply.
        assert!(vfs::access_check(1000, 2000, 1500, 2000, &group_rw, true));
        // Neither matches -> other bits, which are clear here.
        assert!(!vfs::access_check(1000, 2000, 1500, 3000, &group_rw, true));

        let world_r = stat(1000, 2000, S_IFREG | S_IROTH, FileType::File);
        assert!(vfs::access_check(1000, 2000, 1500, 3000, &world_r, false));
        assert!(!vfs::access_check(1000, 2000, 1500, 3000, &world_r, true));
    }

    #[test]
    fn access_check_directory_write_needs_execute_bit() {
        // rwx for owner: write ok.
        let rwx = stat(1000, 1000, S_IFDIR | S_IRUSR | S_IWUSR | S_IXUSR, FileType::Directory);
        assert!(vfs::access_check(1000, 1000, 1000, 1000, &rwx, true));

        // rw- without x: creating entries inside would be unresolvable, so
        // write access must be refused.
        let rw_no_x = stat(1000, 1000, S_IFDIR | S_IRUSR | S_IWUSR, FileType::Directory);
        assert!(!vfs::access_check(1000, 1000, 1000, 1000, &rw_no_x, true));
        // Reading the directory is still fine.
        assert!(vfs::access_check(1000, 1000, 1000, 1000, &rw_no_x, false));
    }

    #[test]
    fn perm_str_renders_every_file_type_and_bit() {
        let file = vfs::perm_str(S_IFREG | S_IRUSR | S_IWUSR | S_IXUSR | S_IRGRP | S_IXOTH);
        assert_eq!(&file[..], b"-rwxr----x");

        let dir = vfs::perm_str(S_IFDIR | S_IRUSR | S_IWUSR | S_IXUSR | S_IRGRP | S_IWGRP | S_IXGRP);
        // No "other" bits were passed, so they must render as dashes.
        assert_eq!(&dir[..], b"drwxrwx---");

        // `perm_str` decodes the type from the 0x2/0x4/0x6/0xA/0x8 nibble of
        // the mode; those bits are not exported as S_IF* constants.
        assert_eq!(&vfs::perm_str(0x2000 | S_IWUSR)[..], b"c-w-------", "char device");
        assert_eq!(&vfs::perm_str(0x6000 | S_IRUSR)[..], b"br--------", "block device");
        assert_eq!(&vfs::perm_str(0xA000 | S_IRUSR | S_IWUSR)[..], b"lrw-------", "symlink");
        // Mode 0 has no type nibble at all, which `perm_str` renders as '?'.
        assert_eq!(&vfs::perm_str(0)[..], b"?---------");
    }

    // ── block cache + io scheduler over the fake device ───────────────────

    #[test]
    fn block_cache_write_read_flush_roundtrip() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        let payload = [0xA5u8; 8];
        assert!(bc_write(dev as u8, 3, &payload));

        // Read back through the cache — this is a hit path, not the device.
        let mut buf = [0u8; 8];
        let (hits_before, _) = bc_stats(); // process-wide and cumulative
        assert!(bc_read(dev as u8, 3, &mut buf));
        assert_eq!(&buf, &payload);
        assert_eq!(
            bc_stats().0,
            hits_before + 1,
            "the second read must be a cache hit"
        );

        // Nothing reached the device yet: the entry is still dirty.
        assert_eq!(fake_sector(3)[0], 0);
        assert!(bc_flush());
        assert_eq!(fake_sector(3)[0], 0xA5);
    }

    #[test]
    fn block_cache_partial_write_does_not_clobber_neighbouring_bytes() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        // Fill a sector through the device, then write a short buffer into the
        // middle of it: read-modify-write must preserve the rest.
        let full = [0x5Au8; SECTOR];
        assert!(fake_write(4, &full));
        assert!(bc_read(dev as u8, 4, &mut [0u8; SECTOR]));
        assert!(bc_write(dev as u8, 4, &[0x11, 0x22]));
        assert!(bc_flush());

        let sector = fake_sector(4);
        assert_eq!(&sector[..2], &[0x11, 0x22]);
        assert!(sector[2..].iter().all(|&b| b == 0x5A));
    }

    #[test]
    fn block_cache_rejects_out_of_range_sectors() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        let mut buf = [0u8; 8];
        assert!(!bc_read(dev as u8, FAKE_SECTORS as u64 + 100, &mut buf));
        assert!(!bc_write(dev as u8, FAKE_SECTORS as u64 + 100, &[1, 2, 3]));
    }

    #[test]
    fn io_scheduler_counts_only_completed_io() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        let (before, reads, writes) = crate::io_scheduler::io_stats();

        // A completed write *through the scheduler* is counted. (Calling
        // `bc_write` directly was the bug in this test: it never touches
        // `total_ios`.)
        assert!(crate::io_scheduler::io_submit_write(dev as u8, 5, &[0x42; 4]));
        let (after_write, _, _) = crate::io_scheduler::io_stats();
        assert_eq!(after_write, before + 1, "one completed IO must be counted");

        // A failing read is not counted.
        let mut buf = [0u8; 4];
        assert!(!crate::io_scheduler::io_submit_read(dev as u8, 999, &mut buf));
        assert_eq!(crate::io_scheduler::io_stats().0, after_write);

        // A succeeding one is.
        assert!(crate::io_scheduler::io_submit_read(dev as u8, 5, &mut buf));
        assert_eq!(&buf, &[0x42; 4]);
        assert_eq!(crate::io_scheduler::io_stats().0, after_write + 1);

        // The read/write split is not tracked; the API says so.
        assert_eq!((reads, writes), (0, 0));
    }

    // ── journal over the fake device ──────────────────────────────────────

    #[test]
    fn journal_commit_replays_onto_targets() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        const START: u64 = 8;
        assert!(crate::journal::journal_init(dev as u8, START, 16));
        assert!(!crate::journal::is_journal_active());

        // Writing without an open transaction must fail.
        assert!(!crate::journal::journal_write(40, &[0xEE; 16]));

        assert!(crate::journal::journal_begin());
        assert!(crate::journal::is_journal_active());
        // Nested begin is refused.
        assert!(!crate::journal::journal_begin());

        assert!(crate::journal::journal_write(40, &[0xEE; 16]));
        assert!(crate::journal::journal_write(41, &[0xEF; 16]));
        assert!(crate::journal::journal_commit());
        assert!(!crate::journal::is_journal_active());

        assert_eq!(fake_sector(40)[0], 0xEE);
        assert_eq!(fake_sector(41)[0], 0xEF);
    }

    /// Write a journal header straight to the device, bypassing the block
    /// cache — this is how a crash window looks: the data blocks and a
    /// COMMITTED header are on the disk, but the target writes never landed.
    fn plant_committed_journal(start_block: u64, targets: &[u32], bodies: &[&[u8]]) {
        let mut hdr = [0u8; SECTOR];
        hdr[0..4].copy_from_slice(&0x4A524E4Cu32.to_ne_bytes()); // "JRNL"
        hdr[4..8].copy_from_slice(&7u32.to_ne_bytes()); // sequence
        hdr[8..12].copy_from_slice(&(targets.len() as u32).to_ne_bytes());
        hdr[12..16].copy_from_slice(&2u32.to_ne_bytes()); // JNL_STATE_COMMITTED
        for (i, &target) in targets.iter().enumerate() {
            let off = 20 + i * 4;
            hdr[off..off + 4].copy_from_slice(&target.to_ne_bytes());
        }
        assert!(fake_write(start_block, &hdr));

        for (i, body) in bodies.iter().enumerate() {
            let mut data = [0u8; SECTOR];
            data[..body.len()].copy_from_slice(body);
            assert!(fake_write(start_block + 1 + i as u64, &data));
        }
    }

    #[test]
    fn journal_replay_redoes_a_committed_transaction() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        // Fresh sector range: no other test in this file has cached these, so
        // the replay sees the planted state instead of a stale cache line.
        const START: u64 = 20;
        plant_committed_journal(START, &[55, 56], &[&[0x77u8; 4][..], &[0x88u8; 4][..]]);

        assert!(crate::journal::journal_replay(dev as u8, START, 16));
        assert_eq!(fake_sector(55)[0], 0x77, "replay must redo block 55");
        assert_eq!(fake_sector(56)[0], 0x88, "replay must redo block 56");

        // The header is retired, so a second replay has nothing to do.
        assert!(crate::journal::journal_replay(dev as u8, START, 16));
    }

    #[test]
    fn journal_replay_discards_an_uncommitted_transaction() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        // Same planted layout, but the header says ACTIVE: the transaction
        // never committed, so its data must be discarded, not redone.
        const START: u64 = 24;
        let mut hdr = [0u8; SECTOR];
        hdr[0..4].copy_from_slice(&0x4A524E4Cu32.to_ne_bytes());
        hdr[8..12].copy_from_slice(&1u32.to_ne_bytes());
        hdr[12..16].copy_from_slice(&1u32.to_ne_bytes()); // JNL_STATE_ACTIVE
        hdr[20..24].copy_from_slice(&57u32.to_ne_bytes());
        assert!(fake_write(START, &hdr));
        let mut data = [0u8; SECTOR];
        data[..4].copy_from_slice(&[0x99u8; 4]);
        assert!(fake_write(START + 1, &data));

        assert!(crate::journal::journal_replay(dev as u8, START, 16));
        assert_eq!(fake_sector(57)[0], 0, "a torn transaction must not be redone");
    }

    /// Regression: `journal_write` bounded entries by `MAX_ENTRIES` (123), the
    /// size of the in-memory `targets[]` array, instead of by the journal size
    /// the device was given. A 16-block journal therefore accepted 123 entries
    /// and entry #16 wrote its redo image to `start_block + 17` — a live ext2
    /// block. Bounding by `MAX_ENTRIES` means the redo image corrupts the very
    /// filesystem it was journalling.
    #[test]
    fn journal_write_refuses_to_leave_its_own_block_range() {
        use crate::journal::{capacity_for_blocks, data_block_for};

        // The boot configuration: 1 header block + 15 redo blocks.
        assert_eq!(capacity_for_blocks(16), 15);
        assert_eq!(data_block_for(3000, 16, 14), Some(3015));
        assert_eq!(
            data_block_for(3000, 16, 15),
            None,
            "entry 15 would land on block 3016, outside the journal"
        );

        // One-block journal: header only, no room for redo at all.
        assert_eq!(capacity_for_blocks(1), 0);
        assert_eq!(capacity_for_blocks(0), 0);
        assert_eq!(data_block_for(3000, 0, 0), None);

        // A journal larger than the targets[] array is still capped by it, so
        // `hdr.targets[idx]` can never go out of bounds.
        assert_eq!(capacity_for_blocks(1000), 123);
        assert_eq!(data_block_for(0, 1000, 122), Some(123));
        assert_eq!(data_block_for(0, 1000, 123), None);
    }

    /// `num_entries` is read back off the device and used as a loop bound in
    /// both `journal_commit` and `journal_replay`, so it must be clamped to
    /// the array bound *and* the journal's real size before it is used.
    #[test]
    fn journal_replay_clamps_a_num_entries_field_from_the_disk() {
        use crate::journal::replay_entry_limit;

        // Honest header on a 16-block journal.
        assert_eq!(replay_entry_limit(15, 16), 15);
        assert_eq!(replay_entry_limit(0, 16), 0);

        // A tampered header claiming 123 entries on a 16-block journal must
        // replay only the 15 that physically exist.
        assert_eq!(replay_entry_limit(123, 16), 15);
        assert_eq!(replay_entry_limit(u32::MAX, 16), 15);

        // Beyond the array bound even on a huge journal.
        assert_eq!(replay_entry_limit(u32::MAX, 100_000), 123);
    }

    /// End-to-end: the on-device limit, not just the helper. A 4-block journal
    /// (1 header + 3 redo) must refuse the 4th entry and leave the sector past
    /// the journal untouched — that sector stands in for live filesystem data.
    #[test]
    fn journal_write_stops_at_the_end_of_the_journal() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        const START: u64 = 30;
        assert!(crate::journal::journal_init(dev as u8, START, 4));
        assert!(crate::journal::journal_begin());

        // Three entries fit.
        for t in 40..43 {
            assert!(
                crate::journal::journal_write(t, &[0x5A; 8]),
                "entry for block {t} must fit"
            );
        }

        // The fourth would write to START + 4, one past the journal.
        assert!(
            !crate::journal::journal_write(43, &[0x5A; 8]),
            "a 4-block journal holds 3 redo entries, not 4"
        );
        assert!(crate::journal::journal_commit());

        assert_eq!(fake_sector(40)[0], 0x5A);
        assert_eq!(fake_sector(42)[0], 0x5A);
        assert_eq!(
            fake_sector(43)[0],
            0,
            "the refused entry must not have touched the block past the journal"
        );
    }

    #[test]
    fn journal_replay_on_empty_journal_is_a_no_op() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        const START: u64 = 8;
        assert!(crate::journal::journal_init(dev as u8, START, 16));
        assert!(!crate::journal::is_journal_active());

        // A fresh header is EMPTY, so replay has nothing to redo, says so, and
        // must not touch a single sector.
        let before: Vec<[u8; SECTOR]> = (0..FAKE_SECTORS as u64).map(fake_sector).collect();
        assert!(
            crate::journal::journal_replay(dev as u8, START, 16),
            "an empty journal is not an error"
        );
        for (lba, sector) in before.iter().enumerate() {
            assert_eq!(fake_sector(lba as u64), *sector, "sector {lba} was modified");
        }
    }

    // ── tmpfs through the VFS ─────────────────────────────────────────────

    #[test]
    fn tmpfs_file_lifecycle() {
        let _serial = serial();
        fresh_fs();

        assert!(vfs::create_file("/hello.txt"));
        let node = vfs::open("/hello.txt").expect("file is visible");
        assert!(node.fs.write(node.inode, 0, b"hello zenus").is_some());

        let stat = node.fs.stat(node.inode);
        assert_eq!(stat.size, 11);
        assert_eq!(stat.file_type, FileType::File);

        let mut buf = [0u8; 11];
        let n = node.fs.read(node.inode, 0, &mut buf).expect("read works");
        assert_eq!(&buf[..n as usize], b"hello zenus");

        // POSIX write() semantics: a short write does *not* shrink the file —
        // only truncate does. The stale tail stays visible in the page cache.
        assert!(node.fs.write(node.inode, 0, b"hi").is_some());
        assert_eq!(node.fs.stat(node.inode).size, 11);
        let n = node.fs.read(node.inode, 0, &mut buf).unwrap();
        assert_eq!(n, 11);
        assert_eq!(&buf[..2], b"hi", "the first two bytes were overwritten");
        assert_eq!(&buf[2..5], b"llo", "the tail survives, as POSIX write() says");

        let entries = vfs::read_dir("/");
        assert!(entries.iter().any(|e| e.name == "hello.txt"));

        assert!(vfs::remove("/hello.txt"));
        assert!(vfs::open("/hello.txt").is_none());
    }

    #[test]
    fn tmpfs_directories_and_enumeration() {
        let _serial = serial();
        fresh_fs();

        assert!(vfs::create_dir("/dir"));
        assert!(vfs::create_file("/dir/a"));
        assert!(vfs::create_file("/dir/b"));

        let entries = vfs::read_dir("/dir");
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["a", "b"]);

        // Removing a child leaves the directory alone, and the directory is
        // NOT removable while `b` is still inside it.
        assert!(vfs::remove("/dir/a"));
        assert!(vfs::open("/dir/a").is_none());
        assert!(vfs::open("/dir").is_some());
        assert!(!vfs::remove("/dir"), "still holds b");

        assert!(vfs::remove("/dir/b"));
        assert!(vfs::remove("/dir"));
        assert!(vfs::open("/dir").is_none());
    }

    #[test]
    fn tmpfs_rejects_missing_parents_and_duplicate_creation() {
        let _serial = serial();
        fresh_fs();

        assert!(!vfs::create_file("/nope/deep/file"), "parent must exist");
        assert!(vfs::create_file("/dup"));
        assert!(!vfs::create_file("/dup"), "creating twice fails");
    }

    // ── procfs ────────────────────────────────────────────────────────────

    #[test]
    fn procfs_reads_registered_sources() {
        let _serial = serial();
        use crate::procfs::{self, ProcFs};
        use crate::vfs::FileSystem;
        use alloc::string::String;

        procfs::register_cpuinfo(|| String::from("processor\t: 0\nmodel name\t: Test CPU\n"));
        procfs::register_meminfo(|| String::from("MemTotal: 2048 kB\n"));
        procfs::register_task_count(|| 7);

        let procfs_fs = ProcFs;

        let mut buf = [0u8; 128];
        let n = procfs_fs.read(1, 0, &mut buf).expect("cpuinfo readable");
        assert_eq!(&buf[..n as usize], b"processor\t: 0\nmodel name\t: Test CPU\n");

        let n = procfs_fs.read(2, 0, &mut buf).expect("meminfo readable");
        assert_eq!(&buf[..n as usize], b"MemTotal: 2048 kB\n");

        // Unregistered source falls back to placeholder text, not a crash.
        let n = procfs_fs.read(3, 0, &mut buf).expect("uptime readable");
        assert!(n > 0);

        assert!(procfs_fs.read(999, 0, &mut buf).is_none(), "unknown inode");
    }

    #[test]
    fn procfs_exposes_the_expected_directory_listing() {
        // `read_dir` walks the same process-global sources that
        // `procfs_reads_registered_sources` registers, so this needs the lock
        // too — otherwise the listing depends on which test runs first.
        let _serial = serial();
        use crate::procfs::ProcFs;
        use crate::vfs::FileSystem;

        let entries = ProcFs.read_dir(ProcFs.root_inode());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        for expected in ["cpuinfo", "meminfo", "uptime", "version", "stat", "loadavg"] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        assert!(names.contains(&"self"));
    }

    // ── cgroup2 ───────────────────────────────────────────────────────────

    #[test]
    fn cgroup_fs_exposes_v2_layout() {
        use crate::cgroup::CgroupFs;
        use crate::vfs::FileSystem;

        let fs = CgroupFs;
        let entries = fs.read_dir(fs.root_inode());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        for expected in [
            "cgroup.procs",
            "cgroup.tasks",
            "cgroup.controllers",
            "cgroup.subtree_control",
            "cpu.pressure",
            "io.pressure",
            "memory.pressure",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }

        let mut buf = [0u8; 64];
        let n = fs.read(4, 0, &mut buf).expect("controllers readable");
        assert_eq!(&buf[..n as usize], b"cpu io memory pids\n");

        // This is a read-only view: no cgroup can be created, removed or
        // configured here, and every mutation must report failure instead of a
        // silent no-op.
        assert!(fs.create(fs.root_inode(), "test", FileType::Directory).is_none());
        assert!(!fs.unlink(fs.root_inode(), "cgroup.procs"));
        assert!(fs.write(4, 0, b"+cpu").is_none());
        assert!(fs.write(2, 0, b"1").is_none(), "cgroup.procs is not writable");
        assert!(fs.lookup(fs.root_inode(), "cgroup.procs").is_some());
        assert!(fs.lookup(fs.root_inode(), "nope").is_none());
    }

    // ── sysctl ────────────────────────────────────────────────────────────

    #[test]
    fn sysctl_registers_the_documented_defaults() {
        let _serial = serial();
        use crate::sysctl::{self, SysctlValue};

        sysctl::sysctl_init();
        sysctl::sysctl_init(); // idempotent: must not append a second copy

        assert_eq!(sysctl::sysctl_count(), 8);
        for name in [
            "kernel.hostname",
            "kernel.log_level",
            "kernel.version",
            "kernel.uptime",
            "kernel.max_tasks",
            "kernel.watchdog_timeout",
            "net.ipv4.ip_forward",
            "net.dns.server",
        ] {
            assert!(sysctl::sysctl_find(name).is_some(), "missing sysctl {name}");
        }

        let hostname = sysctl::sysctl_find("kernel.hostname").unwrap();
        assert_eq!(
            sysctl::sysctl_get(hostname).unwrap().value.as_str(),
            Some("zenus")
        );
    }

    #[test]
    fn sysctl_set_enforces_type_and_read_only() {
        let _serial = serial();
        use crate::sysctl::{self, SysctlValue};

        sysctl::sysctl_init();

        let level = sysctl::sysctl_find("kernel.log_level").unwrap();
        assert!(sysctl::sysctl_set(level, SysctlValue::IntVal(4)));
        assert_eq!(sysctl::sysctl_get(level).unwrap().value.as_int(), Some(4));

        // Type mismatch is rejected.
        assert!(!sysctl::sysctl_set(level, SysctlValue::BoolVal(true)));
        assert_eq!(sysctl::sysctl_get(level).unwrap().value.as_int(), Some(4));

        // kernel.uptime is read-only.
        let uptime = sysctl::sysctl_find("kernel.uptime").unwrap();
        assert!(sysctl::sysctl_get(uptime).unwrap().read_only);
        assert!(!sysctl::sysctl_set(uptime, SysctlValue::UintVal(999)));

        // Out-of-range index.
        assert!(!sysctl::sysctl_set(9_999, SysctlValue::IntVal(1)));
        assert!(sysctl::sysctl_get(9_999).is_none());
        assert!(sysctl::sysctl_find("does.not.exist").is_none());
    }

    #[test]
    fn sysctl_uptime_advances_with_ticks() {
        let _serial = serial();
        use crate::sysctl::{self, SysctlValue};

        sysctl::sysctl_init();
        let uptime = sysctl::sysctl_find("kernel.uptime").unwrap();

        // The scheduler tick calls sysctl_tick() 100 times per second, so 250
        // ticks must read back as 2 seconds (the old /1000 divisor reported 0).
        for _ in 0..250 {
            sysctl::sysctl_tick();
        }
        assert_eq!(sysctl::sysctl_get(uptime).unwrap().value.as_uint(), Some(2));
        assert!(matches!(
            sysctl::sysctl_get(uptime).unwrap().value,
            SysctlValue::UintVal(_)
        ));
    }

    #[test]
    fn sysctl_value_accessors_are_type_checked() {
        use crate::sysctl::SysctlValue;

        assert_eq!(SysctlValue::IntVal(-3).ty(), crate::sysctl::SysctlType::Int);
        assert_eq!(SysctlValue::IntVal(-3).as_int(), Some(-3));
        assert_eq!(SysctlValue::IntVal(-3).as_uint(), None);
        assert_eq!(SysctlValue::UintVal(7).as_uint(), Some(7));
        assert_eq!(SysctlValue::UintVal(7).as_int(), None);
        assert_eq!(SysctlValue::BoolVal(true).as_bool(), Some(true));
        assert_eq!(SysctlValue::BoolVal(true).as_str(), None);
        assert_eq!(SysctlValue::StrVal("x").as_str(), Some("x"));
    }

    // ── package manager ───────────────────────────────────────────────────

    /// Build a minimal `.zpk` image: header, one file entry, one file body.
    fn zpk_image(name: &str, version: &str, path: &str, body: &[u8]) -> Vec<u8> {
        use crate::pkg::{ZpkFileEntry, ZpkHeader};
        use core::mem::size_of;

        let header_size = size_of::<ZpkHeader>();
        let entry_size = size_of::<ZpkFileEntry>();
        let mut image = vec![0u8; header_size + entry_size + body.len()];

        image[0..4].copy_from_slice(b"ZPK1");
        image[4..4 + name.len().min(64)].copy_from_slice(&name.as_bytes()[..name.len().min(64)]);
        image[68..68 + version.len().min(16)]
            .copy_from_slice(&version.as_bytes()[..version.len().min(16)]);
        image[84..88].copy_from_slice(&1u32.to_le_bytes()); // file_count
        image[88..92].copy_from_slice(&(body.len() as u32).to_le_bytes());

        let base = header_size;
        image[base..base + path.len().min(128)].copy_from_slice(&path.as_bytes()[..path.len().min(128)]);
        image[base + 128..base + 132].copy_from_slice(&(body.len() as u32).to_le_bytes());
        image[base + 132..base + 134].copy_from_slice(&0o644u16.to_le_bytes());
        image[base + 134] = 0; // regular file

        let body_at = base + entry_size;
        image[body_at..body_at + body.len()].copy_from_slice(body);
        image
    }

    #[test]
    fn pkg_header_layout_is_stable() {
        use crate::pkg::{ZpkFileEntry, ZpkHeader};
        use core::mem::size_of;

        // The on-disk format is documented as 500-byte records; a silent repr
        // change would make every existing .zpk unreadable.
        assert_eq!(size_of::<ZpkHeader>(), 500);
        assert_eq!(size_of::<ZpkFileEntry>(), 500);
    }

    #[test]
    fn pkg_install_list_and_remove() {
        let _serial = serial();
        fresh_fs();
        assert!(crate::pkg::pkg_init());

        let image = zpk_image("demo", "1.0", "/bin/demo", b"#!/bin/sh\necho hi\n");
        assert!(crate::pkg::pkg_install(&image, 0));

        let installed = vfs::open("/usr/local/bin/demo").expect("payload installed");
        let mut buf = [0u8; 32];
        let n = installed.fs.read(installed.inode, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n as usize], b"#!/bin/sh\necho hi\n");

        assert_eq!(crate::pkg::pkg_installed_count(), 1);
        let listed = crate::pkg::pkg_list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "demo");
        assert_eq!(listed[0].version, "1.0");

        let info = crate::pkg::pkg_info("demo").expect("info available");
        assert_eq!(info.files.len(), 1);
        assert_eq!(info.files[0], "/usr/local/bin/demo");

        assert!(crate::pkg::pkg_remove("demo"));
        assert_eq!(crate::pkg::pkg_installed_count(), 0);
        assert!(vfs::open("/usr/local/bin/demo").is_none());
    }

    #[test]
    fn pkg_install_rejects_malformed_images() {
        let _serial = serial();
        fresh_fs();
        assert!(crate::pkg::pkg_init());

        // Too short to even hold a header.
        assert!(!crate::pkg::pkg_install(b"ZPK1", 0));
        assert!(!crate::pkg::pkg_install(&[], 0));

        // Right size, wrong magic.
        let mut bad_magic = zpk_image("demo", "1.0", "/bin/demo", b"x");
        bad_magic[0] = b'X';
        assert!(!crate::pkg::pkg_install(&bad_magic, 0));

        // Good header but the file body is truncated.
        let mut truncated = zpk_image("demo", "1.0", "/bin/demo", b"0123456789");
        truncated.truncate(truncated.len() - 4);
        assert!(!crate::pkg::pkg_install(&truncated, 0));
    }

    // ── path resolution hardening ────────────────────────────────────────

    /// Regression: a path with more than `MAX_PATH_SEGMENTS` components used to
    /// be silently truncated to its first 32, so the caller got a *different*
    /// file than it asked for.
    #[test]
    fn over_long_paths_are_refused_not_truncated() {
        let _serial = serial();
        fresh_fs();

        // Build "/x/x/.../deep" with MAX_PATH_SEGMENTS + 1 components.
        let mut deep = String::new();
        for _ in 0..MAX_PATH_SEGMENTS + 1 {
            deep.push_str("/x");
        }
        deep.push_str("/secret");
        assert!(vfs::open(&deep).is_none(), "over-long path must not resolve");

        // Exactly at the limit is still resolved (and simply not found).
        let mut at_limit = String::new();
        for _ in 0..MAX_PATH_SEGMENTS {
            at_limit.push_str("/x");
        }
        assert!(vfs::open(&at_limit).is_none());
    }

    /// Regression: `access_check` applied the "directory needs +x" rule to the
    /// owner and group branches but not to "other", so a `--w-------` directory
    /// looked writable to everyone else.
    #[test]
    fn access_check_other_branch_follows_the_directory_rule() {
        let dir_wx = stat(1000, 2000, S_IFDIR | S_IWOTH | S_IXOTH, FileType::Directory);
        assert!(vfs::access_check(1500, 3000, 1500, 3000, &dir_wx, true));

        let dir_w_only = stat(1000, 2000, S_IFDIR | S_IWOTH, FileType::Directory);
        assert!(
            !vfs::access_check(1500, 3000, 1500, 3000, &dir_w_only, true),
            "writing a directory without +x must be refused for others too"
        );
        // The same mode on a plain file is fine.
        let file_w = stat(1000, 2000, S_IFREG | S_IWOTH, FileType::File);
        assert!(vfs::access_check(1500, 3000, 1500, 3000, &file_w, true));
    }

    // ── ext2 structure decoders ───────────────────────────────────────────

    /// Regression: `s_inodes_per_group` and `s_blocks_per_group` are plain
    /// `u32` fields in the superblock and nothing rejected a zero in either.
    /// Every group lookup then divided by them as `(count + per_group - 1) /
    /// per_group`, so a superblock with a zero divisor took a `#DE`.
    ///
    /// `fsck` was the worst case: it called `add_msg("inodes_per_group is
    /// zero")` and then executed the division on the very next statement, so
    /// the tool whose job is to report a damaged filesystem crashed on it.
    /// `Ext2Fs::mount` validated `s_inode_size` and `s_log_block_size` but not
    /// these two, which meant `read_inode_raw` and `alloc_block` faulted on any
    /// mounted hostile image without `fsck` being involved at all.
    ///
    /// `group_count` refuses a zero divisor, and its ceiling uses `div_ceil` so
    /// the intermediate addition cannot overflow on a large `count`.
    #[test]
    fn ext2_group_count_refuses_a_zero_divisor_instead_of_faulting() {
        use crate::ext2::group_count;

        // A zero divisor is refused, not divided by. These are the two superblock
        // fields, and both were reachable with a crafted image.
        assert_eq!(
            group_count(1024, 0),
            None,
            "inodes_per_group == 0 must not be used as a divisor"
        );
        assert_eq!(group_count(0, 0), None, "a zero divisor is invalid regardless of count");

        // Ceiling division: exact multiples do not round up.
        assert_eq!(group_count(8192, 8192), Some(1));
        assert_eq!(group_count(8192, 4096), Some(2));
        assert_eq!(group_count(0, 4096), Some(0), "nothing needs zero groups");

        // A remainder rounds up — the case the old `(count + per_group - 1)`
        // form handled correctly and must keep handling.
        assert_eq!(group_count(1, 8192), Some(1));
        assert_eq!(group_count(8193, 4096), Some(3));
        assert_eq!(group_count(u64::MAX, 8192), Some(u64::MAX / 8192 + 1));

        // The overflow the ceiling form had: `count + per_group - 1` wraps for a
        // count near u64::MAX, which would report *fewer* groups than needed.
        let big = u64::MAX - 10;
        assert_eq!(group_count(big, 8192), Some(big / 8192 + 1));
        assert_eq!(
            group_count(u64::MAX, 2),
            Some(u64::MAX / 2 + 1),
            "a count near u64::MAX must not wrap its way into a smaller answer"
        );
    }

    /// Regression: fsck walked `num_groups` — the *larger* of the inode-derived
    /// and block-derived group counts — and then computed the size of the last
    /// group as `total - (num_groups - 1) * per_group` with plain subtraction.
    ///
    /// A filesystem whose two counts disagree is a real filesystem, not a
    /// malformed one: `inodes_per_group = 8192` with 8193 inodes gives 2 inode
    /// groups, while `blocks_per_group = 1024` with 8192 blocks gives 8 block
    /// groups. fsck reported the mismatch as warning 11, then walked 8 groups,
    /// then computed `8193 - 7 * 8192` — negative. `u32` subtraction, so a
    /// dev build aborted with an overflow and a release build wrapped to
    /// roughly 4 billion free inodes in the final group.
    ///
    /// The checker faulted on the exact input it was reporting.
    #[test]
    fn ext2_fsck_last_group_survives_a_mismatched_group_count() {
        use crate::ext2_fsck::last_group_size;

        // The shape from above: 2 inode groups, 8 block groups, fsck walks 8.
        let inodes_count = 8193u64;
        let inodes_per_group = 8192u64;
        let blocks_count = 8192u64;
        let blocks_per_group = 1024u64;
        let num_groups = 8u64;

        // `8193 - 7 * 8192` is -50111. It must not wrap to ~1.8e19, and it
        // must not panic.
        assert_eq!(
            last_group_size(inodes_count, inodes_per_group, num_groups),
            0,
            "an impossible last group clamps to 0 instead of wrapping"
        );
        assert_eq!(last_group_size(blocks_count, blocks_per_group, num_groups), 1024);

        // A filesystem whose group counts agree is unaffected: the last group
        // holds the remainder.
        assert_eq!(last_group_size(8193, 8192, 2), 1);
        assert_eq!(last_group_size(8192, 4096, 2), 4096);
        assert_eq!(last_group_size(1, 8192, 1), 1, "a single group holds everything");
        assert_eq!(last_group_size(0, 8192, 1), 0);

        // Exactly divisible: the last group is full, and a zero total is 0
        // rather than a wrap.
        assert_eq!(last_group_size(8192, 8192, 1), 8192);
        assert_eq!(last_group_size(0, 8192, 0), 0);
    }

    /// Regression: `fsck`'s `read_inode` sized its sector loop from the
    /// superblock's `s_inode_size` and wrote into a fixed 1024-byte stack
    /// buffer with no check that the count still fit:
    ///
    /// ```ignore
    /// let needed_sectors = (offset_in_sector + inode_size as usize + 511) / 512;
    /// for i in 0..needed_sectors {
    ///     bc_read(dev_id, sector + i as u64, &mut buf[base..base + 512])
    /// }
    /// ```
    ///
    /// `s_inode_size = 0xFFFF` with an inode starting 511 bytes into a sector
    /// asked for 130 sectors and wrote `buf[1024..1536]` — 512 bytes past the
    /// end of a stack array. `Ext2Fs::mount` has always rejected an inode size
    /// outside 128..=256, but `fsck` only *recorded* `inode_size < 128` as a
    /// message and carried on, which is the entire reason to run fsck on a
    /// filesystem you suspect.
    #[test]
    fn ext2_fsck_refuses_to_read_an_inode_that_does_not_fit_its_buffer() {
        use crate::ext2_fsck::inode_read_sectors;

        // The reported crash: 1024 bytes of buffer, an inode size of 0xFFFF.
        assert_eq!(
            inode_read_sectors(511, 0xFFFF, 1024),
            None,
            "an inode larger than the buffer must be refused, not read past it"
        );
        assert_eq!(inode_read_sectors(0, 0xFFFF, 1024), None);

        // An inode that fits in the buffer but not on a sector boundary still
        // needs the second sector, and that is allowed.
        assert_eq!(inode_read_sectors(400, 256, 1024), Some(2));
        assert_eq!(inode_read_sectors(0, 256, 1024), Some(1));

        // The largest start offset and size that still fit exactly.
        assert_eq!(inode_read_sectors(768, 256, 1024), Some(2));
        assert_eq!(inode_read_sectors(768, 257, 1024), None, "one byte over");

        // Every real ext2 inode size is accepted, so the guard is not
        // over-broad for a healthy filesystem.
        for size in [128usize, 256] {
            for offset in [0usize, 1, 255, 256, 511] {
                assert!(
                    inode_read_sectors(offset, size, 1024).is_some(),
                    "inode size {size} at offset {offset} must stay readable"
                );
            }
        }

        // The addition itself cannot overflow, so the bounds check is reached
        // rather than bypassed by wrapping.
        assert_eq!(inode_read_sectors(usize::MAX, 2, 1024), None);
        assert_eq!(inode_read_sectors(usize::MAX - 1, usize::MAX, 1024), None);
    }

    /// Regression: the fixed-size on-disk decoders did
    /// `copy_nonoverlapping(size_of::<T>())` without checking `buf.len()`, so a
    /// short read (truncated image, fuzz case) read past the end of the slice.
    #[test]
    fn ext2_decoders_reject_short_buffers() {
        use crate::ext2;

        // Every decoder refuses anything smaller than its structure and accepts
        // exactly its own size.
        let sb = [0u8; core::mem::size_of::<ext2::RawSuperblock>()];
        assert!(ext2::read_unaligned::<ext2::RawSuperblock>(&sb[..sb.len() - 1]).is_none());
        assert!(ext2::read_unaligned::<ext2::RawSuperblock>(&sb).is_some());

        let bgd = [0u8; core::mem::size_of::<ext2::RawBlockGroupDescriptor>()];
        assert!(ext2::read_unaligned::<ext2::RawBlockGroupDescriptor>(&bgd[..7]).is_none());
        assert!(ext2::read_unaligned::<ext2::RawBlockGroupDescriptor>(&bgd).is_some());

        let ino = [0u8; core::mem::size_of::<ext2::RawInode>()];
        assert!(ext2::read_unaligned::<ext2::RawInode>(&ino[..1]).is_none());
        assert!(ext2::read_unaligned::<ext2::RawInode>(&ino).is_some());
    }

    // ── package manager ───────────────────────────────────────────────────

    /// Regression: `.zpk` entries are attacker-supplied. A path of
    /// `../../../etc/x` used to be prefixed with `/usr/local` and then resolved
    /// *through* `..`, so `pkg-install` wrote outside its prefix and
    /// `pkg-remove` deleted it again.
    #[test]
    fn pkg_install_rejects_path_traversal() {
        let _serial = serial();
        fresh_fs();
        assert!(crate::pkg::pkg_init());

        for evil in [
            "/../../etc/zenus-escape",
            "../../etc/zenus-escape",
            "/usr/local/../../../etc/zenus-escape",
            "/bin/../../../../etc/zenus-escape",
            "",
        ] {
            let image = zpk_image("evil", "1.0", evil, b"pwned");
            assert!(
                !crate::pkg::pkg_install(&image, 0),
                "path {evil:?} must be rejected"
            );
        }

        // Nothing escaped and nothing was left behind: a rejected image must
        // not create a package directory either (it used to, which is why
        // `pkg_installed_count` and `pkg_list` disagreed).
        assert!(
            crate::pkg::pkg_info("evil").is_none(),
            "the rejected image must not be recorded as installed"
        );
        let db_entries = vfs::read_dir(crate::pkg::PKG_DB_DIR)
            .iter()
            .filter(|e| e.name == "evil")
            .count();
        assert_eq!(db_entries, 0, "a rejected install left a directory behind");

        // A legitimate nested path still installs. The package DB is shared
        // with the other pkg tests, so assert on *this* package rather than on
        // a global count.
        let good = zpk_image("good", "1.0", "/bin/tool", b"ok");
        assert!(crate::pkg::pkg_install(&good, 0));
        assert!(vfs::open("/usr/local/bin/tool").is_some());
        let info = crate::pkg::pkg_info("good").expect("good is installed");
        assert_eq!(info.files, ["/usr/local/bin/tool"]);
        assert!(crate::pkg::pkg_remove("good"));
        assert!(crate::pkg::pkg_info("good").is_none());
    }

    // ── cgroup consistency ────────────────────────────────────────────────

    /// Regression: `cgroup.defaults` was listed as a file by `read_dir` but
    /// stat'ed as `FileType::None` with mode 0, and `chmod`/`chown` reported
    /// success on a filesystem that cannot store them.
    #[test]
    fn cgroup_stat_agrees_with_the_listing() {
        use crate::cgroup::CgroupFs;
        use crate::vfs::FileSystem;

        let fs = CgroupFs;
        let ino = fs.lookup(fs.root_inode(), "cgroup.defaults").expect("listed");
        let stat = fs.stat(ino);
        assert_eq!(stat.file_type, FileType::File);
        assert_ne!(stat.mode, 0, "a listed file must have a mode");

        assert!(!fs.chmod(ino, 0o777), "read-only fs must refuse chmod");
        assert!(!fs.chown(ino, 0, 0), "read-only fs must refuse chown");
    }

    // ── tmpfs directory removal ───────────────────────────────────────────

    /// Regression: `rmdir` on a directory that still had entries reported
    /// success and orphaned the children (they became unreachable but were
    /// still consuming nodes).
    #[test]
    fn tmpfs_refuses_to_remove_a_non_empty_directory() {
        let _serial = serial();
        fresh_fs();

        assert!(vfs::create_dir("/d"));
        assert!(vfs::create_file("/d/child"));
        assert!(!vfs::remove("/d"), "non-empty directory must not be removed");
        assert!(vfs::open("/d").is_some());
        assert!(vfs::open("/d/child").is_some(), "child is not orphaned");

        assert!(vfs::remove("/d/child"));
        assert!(vfs::remove("/d"), "empty directory can be removed");
        assert!(vfs::open("/d").is_none());
    }

    // ── sysctl idempotence ────────────────────────────────────────────────

    /// Regression: `sysctl_init` appended a fresh copy of every default on each
    /// call (8 -> 16 -> 24 entries) because nothing reset the table, so the
    /// value `sysctl_set` wrote was not the value `sysctl_get` returned.
    #[test]
    fn sysctl_init_is_idempotent() {
        let _serial = serial();
        use crate::sysctl::{self, SysctlValue};

        sysctl::sysctl_init();
        let first = sysctl::sysctl_count();
        sysctl::sysctl_init();
        assert_eq!(sysctl::sysctl_count(), first, "init must not grow the table");

        // The write must be visible on the same key afterwards.
        let level = sysctl::sysctl_find("kernel.log_level").expect("key exists");
        assert!(sysctl::sysctl_set(level, SysctlValue::IntVal(3)));
        assert_eq!(sysctl::sysctl_get(level).unwrap().value.as_int(), Some(3));
    }

    // ── mount namespaces ──────────────────────────────────────────────────

    /// `mount_in_ns`/`umount_in_ns` had no coverage at all: a mount made in one
    /// namespace must not be visible in another, and the root mount must not be
    /// removable.
    #[test]
    fn mount_is_per_namespace() {
        let _serial = serial();
        fresh_fs();
        zenus_ns::mnt::init();
        assert!(vfs::create_dir("/mnt"));

        let ns = zenus_ns::mnt::create().expect("create mount namespace");
        assert!(vfs::create_mnt_ns(ns), "namespace needs its own mount table");

        static OTHER_FS: crate::procfs::ProcFs = crate::procfs::ProcFs;
        assert!(vfs::mount_in_ns(ns, "/mnt", &OTHER_FS));
        assert!(vfs::umount_in_ns(ns, "/mnt"), "the mount must be there");
        assert!(!vfs::umount_in_ns(ns, "/mnt"), "second umount must fail");
        assert!(
            !vfs::umount("/mnt"),
            "the root namespace never had this mount"
        );
    }

    // ── .zpk on-disk ABI ──────────────────────────────────────────────────

    /// The `.zpk` reader used to cast byte offsets to `*const ZpkHeader` /
    /// `*const ZpkFileEntry`, whose 4-byte alignment a packed byte stream does
    /// not provide: a two-file image with a 5-byte first payload put the second
    /// record at a misaligned address and aborted the process. These offsets
    /// are what the (alignment-free) reader uses.
    #[test]
    fn zpk_layout_matches_the_structs() {
        use crate::pkg::{layout, ZpkFileEntry, ZpkHeader};
        use core::mem::{offset_of, size_of};

        assert_eq!(offset_of!(ZpkHeader, magic), layout::MAGIC);
        assert_eq!(offset_of!(ZpkHeader, name), layout::NAME);
        assert_eq!(offset_of!(ZpkHeader, version), layout::VERSION);
        assert_eq!(offset_of!(ZpkHeader, file_count), layout::FILE_COUNT);
        assert_eq!(offset_of!(ZpkHeader, total_size), layout::TOTAL_SIZE);
        assert_eq!(size_of::<ZpkHeader>(), layout::HEADER_SIZE);

        assert_eq!(offset_of!(ZpkFileEntry, path), layout::ENTRY_PATH);
        assert_eq!(offset_of!(ZpkFileEntry, size), layout::ENTRY_LEN);
        assert_eq!(offset_of!(ZpkFileEntry, mode), layout::ENTRY_MODE);
        assert_eq!(offset_of!(ZpkFileEntry, file_type), layout::ENTRY_FILE_TYPE);
        assert_eq!(size_of::<ZpkFileEntry>(), layout::ENTRY_SIZE);
    }

    /// Two files in one image: the record walk (`offset += entry + payload`)
    /// with a payload length that is not a multiple of 4. That is the case the
    /// pointer casts got wrong.
    #[test]
    fn pkg_install_handles_a_multi_file_image() {
        let _serial = serial();
        fresh_fs();
        assert!(crate::pkg::pkg_init());

        use crate::pkg::layout as L;

        let body_a: &[u8] = b"first"; // 5 bytes: not a multiple of 4
        let body_b: &[u8] = b"second-file";
        let entry_a = L::HEADER_SIZE;
        let body_a_at = entry_a + L::ENTRY_SIZE;
        let entry_b = body_a_at + body_a.len();
        let body_b_at = entry_b + L::ENTRY_SIZE;

        let mut image = alloc::vec![0u8; body_b_at + body_b.len()];
        image[L::MAGIC..L::MAGIC + 4].copy_from_slice(b"ZPK1");
        image[L::NAME..L::NAME + 5].copy_from_slice(b"multi");
        image[L::VERSION..L::VERSION + 3].copy_from_slice(b"2.0");
        image[L::FILE_COUNT..L::FILE_COUNT + 4].copy_from_slice(&2u32.to_le_bytes());

        let mut put = |entry_at: usize, body_at: usize, path: &str, body: &[u8]| {
            image[entry_at..entry_at + path.len()].copy_from_slice(path.as_bytes());
            image[entry_at + L::ENTRY_LEN..entry_at + L::ENTRY_LEN + 4]
                .copy_from_slice(&(body.len() as u32).to_le_bytes());
            image[entry_at + L::ENTRY_MODE..entry_at + L::ENTRY_MODE + 2]
                .copy_from_slice(&0o644u16.to_le_bytes());
            image[body_at..body_at + body.len()].copy_from_slice(body);
        };
        put(entry_a, body_a_at, "/a.txt", body_a);
        put(entry_b, body_b_at, "/sub/b.txt", body_b);

        assert!(
            crate::pkg::pkg_install(&image, 0),
            "a packed two-file image must install"
        );

        let mut buf = [0u8; 32];
        let a = vfs::open("/usr/local/a.txt").expect("first file installed");
        let n = a.fs.read(a.inode, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n as usize], body_a);

        let b = vfs::open("/usr/local/sub/b.txt").expect("second file installed");
        let n = b.fs.read(b.inode, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n as usize], body_b);

        let info = crate::pkg::pkg_info("multi").expect("manifest written");
        assert_eq!(info.file_count, 2);
        assert_eq!(info.version, "2.0");
        assert_eq!(info.files, ["/usr/local/a.txt", "/usr/local/sub/b.txt"]);
        assert!(crate::pkg::pkg_remove("multi"));
    }

    // ── read offsets ──────────────────────────────────────────────────────

    /// Every procfs read so far used offset 0, so a reader that ignored the
    /// offset entirely would still pass.
    #[test]
    fn procfs_honours_read_offsets() {
        let _serial = serial();
        use crate::procfs::{self, ProcFs};
        use crate::vfs::FileSystem;
        use alloc::string::String;

        procfs::register_cpuinfo(|| String::from("processor\t: 0\n"));
        let fs = ProcFs;

        let mut whole = [0u8; 64];
        let n = fs.read(1, 0, &mut whole).unwrap();
        assert_eq!(&whole[..n as usize], b"processor\t: 0\n");

        let mut part = [0u8; 4];
        let n = fs.read(1, 10, &mut part).unwrap();
        assert_eq!(&part[..n as usize], b": 0\n", "offset 10 of the source");
        assert_eq!(fs.read(1, 9_999, &mut part).unwrap(), 0, "past the end");
    }

    #[test]
    fn cgroup_honours_read_offsets() {
        use crate::cgroup::CgroupFs;
        use crate::vfs::FileSystem;

        let fs = CgroupFs;
        let mut buf = [0u8; 8];
        let n = fs.read(4, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n as usize], b"cpu io m");
        assert_eq!(fs.read(4, 9_999, &mut buf).unwrap(), 0);
    }

    // ── block cache eviction ──────────────────────────────────────────────

    /// `evict_one` used to return the hash slot unconditionally once a 4-slot
    /// window was full, overwriting whatever lived there — including a dirty
    /// entry — while hundreds of free slots sat unused elsewhere.
    #[test]
    fn block_cache_survives_a_full_hash_window() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        // More distinct blocks than the associativity of one window, spread
        // across the whole (64-sector) device.
        for lba in 0..FAKE_SECTORS as u64 {
            assert!(bc_write(dev as u8, lba, &[lba as u8; 8]), "write {lba}");
        }
        // Everything written must still read back, i.e. no entry was
        // overwritten on the way.
        let mut buf = [0u8; 8];
        for lba in 0..FAKE_SECTORS as u64 {
            assert!(bc_read(dev as u8, lba, &mut buf), "read {lba}");
            assert_eq!(buf[0], lba as u8, "sector {lba} was overwritten");
        }

        // Dirty entries are still pending: nothing may have been lost.
        assert!(bc_flush());
        assert_eq!(fake_sector(FAKE_SECTORS as u64 - 1)[0], (FAKE_SECTORS - 1) as u8);
    }

    /// Regression: a write to a sector the device rejects used to be cached
    /// anyway and could never be flushed, so one bad LBA made every later
    /// `bc_flush()` (and any read that had to evict) fail for the rest of the
    /// session.
    #[test]
    fn a_rejected_sector_does_not_poison_the_cache() {
        let _serial = serial();
        let dev = fake_device();
        wipe_fake_device();

        assert!(bc_write(dev as u8, 7, &[0x11; 8]));
        assert!(!bc_write(dev as u8, 9_000, &[0x22; 8]), "out-of-range write");
        assert!(bc_flush(), "a rejected write must not wedge later flushes");

        let mut buf = [0u8; 8];
        assert!(bc_read(dev as u8, 7, &mut buf));
        assert_eq!(&buf, &[0x11; 8]);
    }
}
