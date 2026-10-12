//! Host tests for the cgroup controller (`crate::cgroup`).
//!
//! Two layers, and they test different things:
//!
//! * [`CgroupTable`] itself — pure, so the enforcement *decisions* (`pids.max`,
//!   `memory.max`, moving a task between groups, releasing a charge) are checked
//!   here without a scheduler or a page table. This is where the limits are
//!   actually proven to bind.
//! * [`CgroupFs`] over the same table — the filesystem must report what the table
//!   holds and refuse what the table refuses, so a `write` that "succeeded" while
//!   changing nothing cannot hide.
//!
//! The global `CGROUP_TABLE` is shared, so every test takes `serial()` and calls
//! `reset_for_tests()` first.

use crate::cgroup::{reset_for_tests, AttachError, CgroupFs, CgroupTable, CGROUP_UNLIMITED};
use crate::vfs::{FileSystem, FileType};
use alloc::string::String;
use alloc::vec::Vec;

// ── the table ────────────────────────────────────────────────────────────

/// The root always exists, is never removable, and is never limited — which is
/// what makes an unconfigured system behave exactly as it did before cgroups.
#[test]
fn the_root_group_accepts_everything_and_cannot_be_removed() {
    let mut t = CgroupTable::empty();
    t.reset();
    assert_eq!(t.lookup_name("/"), Some(0));
    assert_eq!(t.pids_max(0), Some(CGROUP_UNLIMITED));
    assert_eq!(t.memory_max(0), Some(CGROUP_UNLIMITED));
    assert!(!t.remove(0), "the root cannot be removed");

    // A task in the root is always attachable and always chargeable.
    assert_eq!(t.attach(0, 1), Ok(()));
    assert!(t.charge(1, 1 << 40), "the root has no memory limit");
    assert_eq!(t.pids_current(0), 1);
}

/// `create` rejects the three ways a cgroup name can be bad, so the table cannot
/// be grown by a hostile `mkdir` loop.
#[test]
fn create_rejects_empty_duplicate_and_over_long_names() {
    let mut t = CgroupTable::empty();
    t.reset();
    assert!(t.create("").is_none(), "empty name");
    assert!(t.create("a").is_some());
    assert!(t.create("a").is_none(), "duplicate name");
    let long = "x".repeat(65);
    assert!(t.create(&long).is_none(), "over-long name");
}

/// The core of `pids.max`: the limit is checked at attach, and the group refuses
/// the task that would exceed it while still accepting tasks up to it.
#[test]
fn pids_max_refuses_the_attach_that_would_exceed_it() {
    let mut t = CgroupTable::empty();
    t.reset();
    let g = t.create("limited").expect("created");
    assert!(t.set_pids_max(g, 2));

    assert_eq!(t.attach(g, 10), Ok(()));
    assert_eq!(t.attach(g, 11), Ok(()));
    assert_eq!(t.attach(g, 12), Err(AttachError::PidsLimit));
    assert_eq!(t.pids_current(g), 2, "the refused task must not be counted");

    // A task that leaves makes room again.
    t.detach(10);
    assert_eq!(t.attach(g, 12), Ok(()));
    assert_eq!(t.pids_current(g), 2);
}

/// Attaching is idempotent: re-attaching a task already in the group is not a
/// second member, so a double call cannot fill the group by itself.
#[test]
fn re_attaching_a_task_does_not_count_twice() {
    let mut t = CgroupTable::empty();
    t.reset();
    let g = t.create("g").expect("created");
    t.set_pids_max(g, 1);

    assert_eq!(t.attach(g, 5), Ok(()));
    assert_eq!(t.attach(g, 5), Ok(()), "idempotent");
    assert_eq!(t.pids_current(g), 1);
    assert_eq!(t.attach(g, 6), Err(AttachError::PidsLimit), "still full");
}

/// A task moves between groups rather than existing in two, and its memory
/// charge travels with it. Otherwise a task could be counted against two limits
/// at once, or its charge could be lost on the move.
#[test]
fn moving_a_task_carries_its_charge_and_leaves_no_copy_behind() {
    let mut t = CgroupTable::empty();
    t.reset();
    let a = t.create("a").expect("a");
    let b = t.create("b").expect("b");

    assert_eq!(t.attach(a, 7), Ok(()));
    assert!(t.charge(7, 4096));
    assert_eq!(t.memory_current(a), 4096);

    assert_eq!(t.attach(b, 7), Ok(()));
    assert_eq!(t.group_of_task(7), Some(b));
    assert_eq!(t.pids_current(a), 0, "the task left a");
    assert_eq!(t.memory_current(a), 0, "and took its charge with it");
    assert_eq!(t.memory_current(b), 4096);
}

/// `memory.max` binds, and a refused charge changes nothing — the page that ties
/// the group over the limit must not be mapped *and* must not be counted.
#[test]
fn memory_max_refuses_the_charge_that_would_exceed_it() {
    let mut t = CgroupTable::empty();
    t.reset();
    let g = t.create("mem").expect("created");
    assert!(t.set_memory_max(g, 8192));
    assert_eq!(t.attach(g, 3), Ok(()));

    assert!(t.charge(3, 4096), "first page fits");
    assert!(!t.charge(3, 8192), "second page would exceed 8192");
    assert_eq!(
        t.memory_current(g),
        4096,
        "the refused charge is not counted"
    );

    assert!(t.charge(3, 4096), "exactly at the limit still fits");
    assert!(!t.charge(3, 1), "one byte over does not");

    t.uncharge(3, 4096);
    assert_eq!(t.memory_current(g), 4096);
    assert!(t.charge(3, 4096), "and there is room again");
}

/// `uncharge` saturates: an over-release is a caller bug, and turning it into a
/// panic would make it a kernel panic on the `munmap` path.
#[test]
fn uncharging_more_than_was_charged_saturates_at_zero() {
    let mut t = CgroupTable::empty();
    t.reset();
    let g = t.create("g").expect("created");
    t.attach(g, 4).expect("attach");
    t.charge(4, 4096);
    t.uncharge(4, 1 << 20);
    assert_eq!(t.memory_current(g), 0, "not wrapped around");
}

/// A cgroup holding a task cannot be removed — removing it would orphan the
/// membership and lose the accounting.
#[test]
fn a_non_empty_group_cannot_be_removed() {
    let mut t = CgroupTable::empty();
    t.reset();
    let g = t.create("g").expect("created");
    t.attach(g, 1).expect("attach");
    assert!(!t.remove(g), "non-empty group must not be removed");
    t.detach(1);
    assert!(t.remove(g), "empty group is removable");
    assert_eq!(t.attach(g, 1), Err(AttachError::NoSuchGroup));
}

/// Attaching to a group that does not exist fails instead of silently landing in
/// the root.
#[test]
fn attaching_to_an_unknown_group_fails() {
    let mut t = CgroupTable::empty();
    t.reset();
    assert_eq!(t.attach(999, 1), Err(AttachError::NoSuchGroup));
    assert_eq!(t.group_of_task(1), None, "the task is still unattached");
}

// ── the filesystem ───────────────────────────────────────────────────────

fn read_str(fs: &CgroupFs, inode: u64) -> String {
    let mut buf = [0u8; 256];
    let n = fs.read(inode, 0, &mut buf).expect("readable");
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}

/// `pids.max` written through the filesystem must bind the table: the whole
/// point of the feature is that the write is not a no-op.
#[test]
fn writing_pids_max_through_the_filesystem_enforces_it() {
    let _serial = crate::host_tests::serial();
    reset_for_tests();
    let fs = CgroupFs;

    let dir = fs
        .create(fs.root_inode(), "cg", FileType::Directory)
        .expect("mkdir");
    let pids_max = fs.lookup(dir, "pids.max").expect("listed");
    let pids_current = fs.lookup(dir, "pids.current").expect("listed");
    let procs = fs.lookup(dir, "cgroup.procs").expect("listed");

    assert!(fs.write(pids_max, 0, b"2").is_some(), "set the limit");
    assert_eq!(read_str(&fs, pids_max), "2\n");

    // Attach two tasks through cgroup.procs; the third must be refused.
    assert!(fs.write(procs, 0, b"10").is_some());
    assert!(fs.write(procs, 0, b"11").is_some());
    assert!(
        fs.write(procs, 0, b"12").is_none(),
        "a write past pids.max must fail, not silently succeed"
    );
    assert_eq!(read_str(&fs, pids_current), "2\n");
    assert_eq!(read_str(&fs, procs), "10\n11\n");
}

/// Writing `max` lifts the limit, and `memory.max` reads back the way cgroup v2
/// spells an unlimited value.
#[test]
fn unlimited_limits_read_and_write_as_max() {
    let _serial = crate::host_tests::serial();
    reset_for_tests();
    let fs = CgroupFs;

    let dir = fs
        .create(fs.root_inode(), "cg", FileType::Directory)
        .expect("mkdir");
    let mem_max = fs.lookup(dir, "memory.max").expect("listed");
    assert_eq!(read_str(&fs, mem_max), "max\n", "unlimited by default");

    assert!(fs.write(mem_max, 0, b"1048576").is_some());
    assert_eq!(read_str(&fs, mem_max), "1048576\n");
    assert!(fs.write(mem_max, 0, b"max").is_some());
    assert_eq!(read_str(&fs, mem_max), "max\n");

    // Garbage is refused rather than parsed as zero, which would freeze the
    // group instead of lifting the limit.
    assert!(fs.write(mem_max, 0, b"nonsense").is_none());
    assert_eq!(read_str(&fs, mem_max), "max\n");
}

/// The table's memory charge is visible through the group's `memory.current`.
#[test]
fn memory_current_reports_what_the_table_charged() {
    let _serial = crate::host_tests::serial();
    reset_for_tests();
    let fs = CgroupFs;

    let dir = fs
        .create(fs.root_inode(), "cg", FileType::Directory)
        .expect("mkdir");
    let procs = fs.lookup(dir, "cgroup.procs").expect("listed");
    let mem_current = fs.lookup(dir, "memory.current").expect("listed");
    assert!(fs.write(procs, 0, b"1").is_some());

    assert_eq!(read_str(&fs, mem_current), "0\n");
    assert!(crate::cgroup::charge_memory(1, 8192));
    assert_eq!(read_str(&fs, mem_current), "8192\n");
    crate::cgroup::uncharge_memory(1, 8192);
    assert_eq!(read_str(&fs, mem_current), "0\n");
}

/// A created cgroup shows up in the root listing and disappears when removed —
/// the filesystem view and the table cannot disagree.
#[test]
fn created_groups_appear_in_the_root_listing() {
    let _serial = crate::host_tests::serial();
    reset_for_tests();
    let fs = CgroupFs;

    let names = |fs: &CgroupFs| -> Vec<String> {
        fs.read_dir(fs.root_inode())
            .iter()
            .map(|e| e.name.clone())
            .collect()
    };
    assert!(!names(&fs).iter().any(|n| n == "alpha"));

    fs.create(fs.root_inode(), "alpha", FileType::Directory)
        .expect("mkdir");
    assert!(names(&fs).iter().any(|n| n == "alpha"));

    assert!(fs.unlink(fs.root_inode(), "alpha"));
    assert!(!names(&fs).iter().any(|n| n == "alpha"));
}

/// The `read`/`write` dispatch must not confuse a directory inode with the files
/// inside it: a directory has no contents, and a file's inode must resolve to its
/// own cgroup.
#[test]
fn a_directory_inode_is_not_mistaken_for_a_file() {
    let _serial = crate::host_tests::serial();
    reset_for_tests();
    let fs = CgroupFs;

    let dir = fs
        .create(fs.root_inode(), "cg", FileType::Directory)
        .expect("mkdir");
    let mut buf = [0u8; 16];
    assert_eq!(
        fs.read(dir, 0, &mut buf),
        Some(0),
        "a directory has no bytes"
    );

    // Two groups get distinct inodes and distinct files.
    let dir2 = fs
        .create(fs.root_inode(), "cg2", FileType::Directory)
        .expect("mkdir");
    assert_ne!(dir, dir2);
    assert_ne!(
        fs.lookup(dir, "pids.max"),
        fs.lookup(dir2, "pids.max"),
        "each group's files are its own"
    );
    assert_eq!(fs.stat(dir).file_type, FileType::Directory);
}
