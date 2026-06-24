#![no_std]

use zutils_common::{Args, Writer};
use zenus_net::nic;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let count = nic::iface_count();
    for i in 0..count {
        if let Some(iface) = nic::get_iface(i) {
            w.write_str("Interface ");
            w.write_u64(i as u64);
            w.write_str(":\r\n");
            w.write_str("  MAC: ");
            for (j, b) in iface.mac.iter().enumerate() {
                if j > 0 { w.write_byte(b':'); }
                w.write_hex(*b as u64);
            }
            w.write_str("\r\n  IP: ");
            w.write_ip(iface.ip);
            w.write_str("\r\n  Link: ");
            if iface.link_up {
                w.write_str("UP\r\n");
            } else {
                w.write_str("DOWN\r\n");
            }
        }
    }
}
