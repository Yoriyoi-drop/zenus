#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


use zenus_net::nic;
use zutils_common::{Args, Writer};

const HEX: &[u8; 16] = b"0123456789abcdef";

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let count = nic::iface_count();
    for i in 0..count {
        if let Some(iface) = nic::get_iface(i) {
            w.write_str("Interface ");
            w.write_u64(i as u64);
            w.write_str(":\r\n");
            w.write_str("  MAC: ");
            for (j, b) in iface.mac.iter().enumerate() {
                if j > 0 {
                    w.write_byte(b':');
                }
                // Two hex digits per octet. `write_hex` emits `0x` plus 16
                // zero-padded digits, which printed every octet as
                // `0x0000000000000052`.
                w.write_byte(HEX[(b >> 4) as usize]);
                w.write_byte(HEX[(b & 0xF) as usize]);
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
