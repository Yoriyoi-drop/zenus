use zenus_console::serial::SerialPort;

type TestFn = fn() -> Result<(), &'static str>;

struct TestCase {
    name: &'static str,
    func: TestFn,
}

macro_rules! test {
    ($name:expr, $func:expr) => {
        TestCase { name: $name, func: $func }
    };
}

const TESTS: &[TestCase] = &[
    // Block cache tests
    test!("bc/new_cache_empty",       zenus_fs::block_cache::tests::test_new_cache_empty),
    test!("bc/evict_on_empty",        zenus_fs::block_cache::tests::test_evict_on_empty_returns_index_0),
    test!("bc/find_entry_empty",      zenus_fs::block_cache::tests::test_find_entry_empty_returns_none),
    test!("bc/stats_empty",           zenus_fs::block_cache::tests::test_stats_empty),
    test!("bc/lru_counter_init",      zenus_fs::block_cache::tests::test_lru_counter_increments_on_evict),
    test!("bc/cache_size_constant",   zenus_fs::block_cache::tests::test_cache_size_constant),
    test!("bc/sector_size_constant",  zenus_fs::block_cache::tests::test_sector_size_constant),
    // VFS path resolution tests
    test!("vfs/parent_dir_root",      zenus_fs::vfs::tests::test_parent_dir_root),
    test!("vfs/parent_dir_simple",    zenus_fs::vfs::tests::test_parent_dir_simple),
    test!("vfs/parent_dir_top",       zenus_fs::vfs::tests::test_parent_dir_top_level),
    test!("vfs/parent_dir_trailing",  zenus_fs::vfs::tests::test_parent_dir_trailing_slash),
    test!("vfs/file_name_simple",     zenus_fs::vfs::tests::test_file_name_simple),
    test!("vfs/file_name_root",       zenus_fs::vfs::tests::test_file_name_root),
    test!("vfs/file_name_top",        zenus_fs::vfs::tests::test_file_name_top),
    test!("vfs/file_name_trailing",   zenus_fs::vfs::tests::test_file_name_trailing_slash),
    // Ext2 struct tests
    test!("ext2/magic_constant",      zenus_fs::ext2::tests::test_magic_constant),
    test!("ext2/root_inode_constant", zenus_fs::ext2::tests::test_root_inode_constant),
    test!("ext2/superblock_size",     zenus_fs::ext2::tests::test_raw_superblock_size),
    test!("ext2/inode_size",          zenus_fs::ext2::tests::test_raw_inode_size),
    test!("ext2/dirent_size",         zenus_fs::ext2::tests::test_raw_dir_entry_size),
    test!("ext2/bgdt_size",           zenus_fs::ext2::tests::test_raw_bgdt_size),
    test!("ext2/file_type",           zenus_fs::ext2::tests::test_inode_file_type),
    // Paging tests
    test!("paging/page_size_value",           zenus_mem::paging::tests::test_page_size_value),
    test!("paging/page_size_power_of_two",    zenus_mem::paging::tests::test_page_size_is_power_of_two),
    test!("paging/page_size_aligned",         zenus_mem::paging::tests::test_page_size_aligned),
];

pub fn run_tests(serial: &mut SerialPort) {
    serial.write_str("\n=== Zenus Test Suite ===\n");

    let mut passed = 0u64;
    let mut failed = 0u64;

    for test in TESTS {
        serial.write_str("  [TEST] ");
        serial.write_str(test.name);
        serial.write_str("... ");

        match (test.func)() {
            Ok(()) => {
                serial.write_str("OK\n");
                passed += 1;
            }
            Err(msg) => {
                serial.write_str("FAIL: ");
                serial.write_str(msg);
                serial.write_str("\n");
                failed += 1;
            }
        }
    }

    serial.write_str("\n=== Results: ");
    serial.write_u64(passed);
    serial.write_str(" passed, ");
    serial.write_u64(failed);
    serial.write_str(" failed, ");
    serial.write_u64(passed + failed);
    serial.write_str(" total ===\n");

    if failed > 0 {
        serial.write_str("[WARN] Some tests FAILED\n");
    } else {
        serial.write_str("[OK] All tests passed\n");
    }
}
