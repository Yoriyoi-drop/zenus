#![no_std]

extern crate alloc;

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod ipc;
pub mod mnt;
pub mod net;
pub mod pid;
pub mod user;
pub mod uts;

use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_NS_ID: AtomicU32 = AtomicU32::new(1);

pub type NsId = u32;

pub const NS_ROOT: NsId = 0;

pub const CLONE_NEWUTS: u64 = 0x04000000;
pub const CLONE_NEWPID: u64 = 0x20000000;
pub const CLONE_NEWNS: u64 = 0x00020000;
pub const CLONE_NEWNET: u64 = 0x40000000;
pub const CLONE_NEWUSER: u64 = 0x10000000;
pub const CLONE_NEWIPC: u64 = 0x08000000;

pub fn alloc_ns_id() -> NsId {
    NEXT_NS_ID.fetch_add(1, Ordering::SeqCst)
}

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// Each namespace kind owns a private table, so the tests below can run in
/// parallel as long as one test sticks to one kind. Tests that share a table
/// take `serial()`.
#[cfg(test)]
mod host_tests {
    use super::{
        alloc_ns_id, ipc, mnt, net, pid, user, uts, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS,
        CLONE_NEWPID, CLONE_NEWUSER, CLONE_NEWUTS, NS_ROOT,
    };
    use zenus_sync::spinlock::{SpinLock, SpinLockGuard};

    static SERIAL: SpinLock<()> = SpinLock::new(());

    fn serial() -> SpinLockGuard<'static, ()> {
        SERIAL.lock()
    }

    #[test]
    fn ns_ids_are_unique_and_never_reuse_root() {
        let a = alloc_ns_id();
        let b = alloc_ns_id();
        assert_ne!(a, b);
        assert_ne!(a, NS_ROOT);
        assert_ne!(b, NS_ROOT);
        assert!(a > NS_ROOT);
    }

    #[test]
    fn clone_flags_are_distinct_single_bits() {
        let flags = [
            CLONE_NEWUTS,
            CLONE_NEWPID,
            CLONE_NEWNS,
            CLONE_NEWNET,
            CLONE_NEWUSER,
            CLONE_NEWIPC,
        ];
        for (i, a) in flags.iter().enumerate() {
            assert_eq!(a.count_ones(), 1, "clone flag must be one bit: {a:#x}");
            for b in flags.iter().skip(i + 1) {
                assert_eq!(a & b, 0, "clone flags {a:#x} and {b:#x} overlap");
            }
        }
    }

    #[test]
    fn pid_namespace_maps_local_to_global() {
        let _serial = serial();
        pid::init();

        let ns = pid::create().expect("create pid namespace");
        assert_ne!(ns, NS_ROOT);

        let local_a = pid::register_task(ns, 100).expect("register");
        let local_b = pid::register_task(ns, 101).expect("register");
        assert_eq!(local_a, 1, "first local pid in a fresh namespace is 1");
        assert_eq!(local_b, 2);

        assert_eq!(pid::global_tid(ns, local_a), Some(100));
        assert_eq!(pid::global_tid(ns, local_b), Some(101));
        assert_eq!(pid::local_pid(ns, 101), Some(local_b));
        assert_eq!(pid::global_tid(ns, 999), None);

        // A task registered in one namespace must not be visible in another.
        let other = pid::create().expect("create second pid namespace");
        assert_eq!(pid::global_tid(other, local_a), None);

        pid::unregister_task(ns, 100);
        assert_eq!(pid::global_tid(ns, local_a), None);
        assert_eq!(pid::local_pid(ns, 100), None);
        // The compacted table must still resolve the surviving entry.
        assert_eq!(pid::global_tid(ns, local_b), Some(101));

        assert_eq!(pid::register_task(0xDEAD_BEEF, 1), None);
    }

    #[test]
    fn uts_namespace_isolates_hostname() {
        let _serial = serial();
        uts::init();

        assert_eq!(&uts::get_hostname(NS_ROOT)[..6], b"zenus\0");
        assert_eq!(&uts::get_domainname(NS_ROOT)[..7], b"(none)\0");

        let ns = uts::create().expect("create uts namespace");
        assert!(uts::set_hostname(ns, b"container-a"));
        assert!(uts::set_domainname(ns, b"lab.local"));

        assert_eq!(&uts::get_hostname(ns)[..12], b"container-a\0");
        assert_eq!(&uts::get_domainname(ns)[..10], b"lab.local\0");

        // Writing in the child namespace must not touch the root one.
        assert_eq!(&uts::get_hostname(NS_ROOT)[..6], b"zenus\0");

        // Over-long names are truncated to MAX_HOSTNAME-1 (leave room for NUL).
        assert!(uts::set_hostname(ns, &[b'x'; 200]));
        let long = uts::get_hostname(ns);
        assert_eq!(long[63], 0);
        assert_eq!(long[62], b'x');

        assert!(!uts::set_hostname(0xDEAD_BEEF, b"nope"));
        assert_eq!(uts::get_hostname(0xDEAD_BEEF), [0; 64]);
    }

    #[test]
    fn net_namespace_tracks_interface_bits() {
        let _serial = serial();
        net::init();

        let ns = net::create().expect("create net namespace");
        assert_eq!(net::get_interfaces(ns), [0; 8]);

        assert!(net::add_interface(ns, 0));
        assert!(net::add_interface(ns, 9));
        let ifaces = net::get_interfaces(ns);
        assert_eq!(ifaces[0] & 1, 1, "iface 0 -> bit 0 of byte 0");
        assert_eq!(ifaces[1] & 2, 2, "iface 9 -> bit 1 of byte 1");

        // Interface 63 is the last representable slot; 64 would overflow.
        assert!(net::add_interface(ns, 63));
        assert_eq!(net::get_interfaces(ns)[7] & 0x80, 0x80);
        assert!(!net::add_interface(ns, 64));

        // Root namespace keeps its own (empty) view.
        assert_eq!(net::get_interfaces(NS_ROOT), [0; 8]);

        net::destroy(ns);
        assert!(!net::add_interface(ns, 1), "destroyed ns is unusable");
        // The root namespace must survive destroy().
        net::destroy(NS_ROOT);
        assert!(net::add_interface(NS_ROOT, 0));
    }

    #[test]
    fn user_namespace_maps_uids() {
        let _serial = serial();
        user::init();

        assert_eq!(user::translate_uid(NS_ROOT, 0), Some(0));

        let ns = user::create().expect("create user namespace");
        assert_eq!(user::translate_uid(ns, 0), None, "empty map has no entries");
        assert!(user::map_uid(ns, 0, 1000));
        assert!(user::map_uid(ns, 1, 1001));

        assert_eq!(user::translate_uid(ns, 0), Some(1000));
        assert_eq!(user::translate_uid(ns, 1), Some(1001));
        assert_eq!(user::translate_uid(ns, 2), None);

        // The parent namespace's mapping is untouched.
        assert_eq!(user::translate_uid(NS_ROOT, 0), Some(0));
        assert_eq!(user::translate_uid(NS_ROOT, 1), None);

        assert!(!user::map_uid(0xDEAD_BEEF, 0, 0));
    }

    #[test]
    fn ipc_and_mnt_namespaces_exist_and_survive_root_destroy() {
        let _serial = serial();
        ipc::init();
        mnt::init();

        assert!(ipc::exists(NS_ROOT));
        let ns = ipc::create().expect("create ipc namespace");
        assert!(ipc::exists(ns));
        ipc::destroy(ns);
        assert!(!ipc::exists(ns));
        ipc::destroy(NS_ROOT);
        assert!(ipc::exists(NS_ROOT), "root ipc namespace is indestructible");

        let mns = mnt::create().expect("create mnt namespace");
        assert!(mnt::exists(mns));
        mnt::destroy(mns);
        assert!(!mnt::exists(mns));
        mnt::destroy(NS_ROOT);
        assert!(mnt::exists(NS_ROOT), "root mnt namespace is indestructible");
    }
}
