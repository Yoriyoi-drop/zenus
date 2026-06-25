#![no_std]
#![allow(static_mut_refs)]

pub mod nic;
pub mod ethernet;
pub mod ipv4;
pub mod tcp;
pub mod udp;
pub mod socket;
pub mod rtl8139;
pub mod arp;
pub mod icmp;
pub mod dhcp;
pub mod dhcp_server;
pub mod dns;
pub mod route;
pub mod ssh;
pub mod firewall;
