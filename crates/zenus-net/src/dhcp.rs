use crate::udp;
use crate::nic;
use crate::ipv4;

const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;

const OP_REQUEST: u8 = 1;
const OP_REPLY: u8 = 2;

const MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

const OPT_PAD: u8 = 0;
const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS: u8 = 6;
const OPT_LEASE_TIME: u8 = 51;
const OPT_MSG_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_REQ_IP: u8 = 50;
const OPT_PARAM_REQ: u8 = 55;
const OPT_END: u8 = 255;

const MSG_DISCOVER: u8 = 1;
const MSG_OFFER: u8 = 2;
const MSG_REQUEST: u8 = 3;
const MSG_ACK: u8 = 5;

const MAX_OPTIONS: usize = 312;
const BCAST_FLAG: u16 = 0x8000;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    Discovering,
    Requesting,
    Bound,
}

static mut DHCP_STATE: State = State::Idle;
static mut XID: u32 = 0;
static mut SERVER_ID: [u8; 4] = [0; 4];
static mut OFFERED_IP: [u8; 4] = [0; 4];
static mut LEASE_TIME: u32 = 0;

static mut RESP_BUF: [u8; 1500] = [0; 1500];
static mut RESP_LEN: usize = 0;
static mut RESP_READY: bool = false;

fn build_dhcp_msg(op: u8, xid: u32, ciaddr: [u8; 4], mac: &[u8; 6], msg_type: u8, server_id: Option<[u8; 4]>, req_ip: Option<[u8; 4]>) -> [u8; 548] {
    let mut buf = [0u8; 548];
    buf[0] = op;
    buf[1] = 1;
    buf[2] = 6;
    buf[3] = 0;
    buf[4..8].copy_from_slice(&xid.to_be_bytes());
    buf[10..12].copy_from_slice(&BCAST_FLAG.to_be_bytes());
    buf[12..16].copy_from_slice(&ciaddr);
    buf[28..34].copy_from_slice(&mac[..6]);

    let mut off = 236usize;
    buf[off..off + 4].copy_from_slice(&MAGIC_COOKIE);
    off += 4;

    buf[off] = OPT_MSG_TYPE;
    buf[off + 1] = 1;
    buf[off + 2] = msg_type;
    off += 3;

    if let Some(srv) = server_id {
        buf[off] = OPT_SERVER_ID;
        buf[off + 1] = 4;
        buf[off + 2..off + 6].copy_from_slice(&srv);
        off += 6;
    }

    if let Some(rip) = req_ip {
        buf[off] = OPT_REQ_IP;
        buf[off + 1] = 4;
        buf[off + 2..off + 6].copy_from_slice(&rip);
        off += 6;
    }

    buf[off] = OPT_PARAM_REQ;
    buf[off + 1] = 3;
    buf[off + 2] = OPT_SUBNET_MASK;
    buf[off + 3] = OPT_ROUTER;
    buf[off + 4] = OPT_DNS;
    off += 5;

    buf[off] = OPT_END;
    off += 1;

    for i in off..548 {
        buf[i] = OPT_PAD;
    }

    buf
}

pub fn handle_receive(_iface_idx: usize, _src_ip: [u8; 4], packet: &[u8]) -> bool {
    if packet.len() < 8 {
        return false;
    }
    let (hdr, payload) = match udp::parse(packet) {
        Some(h) => h,
        None => return false,
    };

    if hdr.src_port != DHCP_SERVER_PORT || hdr.dst_port != DHCP_CLIENT_PORT {
        return false;
    }

    if payload.len() < 240 {
        return false;
    }

    let magic = [payload[236], payload[237], payload[238], payload[239]];
    if magic != MAGIC_COOKIE {
        return false;
    }

    let rxid = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if rxid != unsafe { XID } {
        return false;
    }

    let resp_len = core::cmp::min(packet.len(), unsafe { RESP_BUF.len() });
    unsafe {
        RESP_BUF[..resp_len].copy_from_slice(&packet[..resp_len]);
        RESP_LEN = resp_len;
        RESP_READY = true;
    }

    true
}

fn parse_dhcp_options(payload: &[u8]) -> (u8, [u8; 4], [u8; 4], [u8; 4], u32) {
    let mut msg_type = 0;
    let mut server_id = [0u8; 4];
    let mut subnet_mask = [0u8; 4];
    let mut gateway = [0u8; 4];
    let mut lease_time = 0u32;

    let mut off = 240usize;
    while off + 1 < payload.len() {
        let opt = payload[off];
        if opt == OPT_END {
            break;
        }
        if opt == OPT_PAD {
            off += 1;
            continue;
        }
        if off + 1 >= payload.len() {
            break;
        }
        let len = payload[off + 1] as usize;
        if off + 2 + len > payload.len() {
            break;
        }
        let val = &payload[off + 2..off + 2 + len];
        match opt {
            OPT_MSG_TYPE => {
                if len >= 1 { msg_type = val[0]; }
            }
            OPT_SERVER_ID => {
                if len >= 4 { server_id.copy_from_slice(&val[..4]); }
            }
            OPT_SUBNET_MASK => {
                if len >= 4 { subnet_mask.copy_from_slice(&val[..4]); }
            }
            OPT_ROUTER => {
                if len >= 4 { gateway.copy_from_slice(&val[..4]); }
            }
            OPT_LEASE_TIME => {
                if len >= 4 { lease_time = u32::from_be_bytes([val[0], val[1], val[2], val[3]]); }
            }
            _ => {}
        }
        off += 2 + len;
    }
    (msg_type, server_id, subnet_mask, gateway, lease_time)
}

pub fn dhcp_start(iface_idx: usize) -> bool {
    unsafe {
        DHCP_STATE = State::Idle;
        RESP_READY = false;
    }

    let iface = match nic::get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };

    let mac = iface.mac;
    let xid = u32::from_le_bytes([mac[0] as u32 as u8, mac[1] as u32 as u8, mac[2] as u32 as u8, mac[3] as u32 as u8]).wrapping_mul(0x01000001).wrapping_add(0xDEAD0001);
    unsafe { XID = xid; }

    let bcast_ip = [255u8; 4];
    let zero_ip = [0u8; 4];

    let mut retries = 0;
    let max_retries = 3;
    let timeout_ticks: usize = 50;

    unsafe { DHCP_STATE = State::Discovering; }

    loop {
        match unsafe { DHCP_STATE } {
            State::Bound => return true,
            State::Idle => return false,
            _ => {}
        }

        let is_discover = matches!(unsafe { DHCP_STATE }, State::Discovering);

        if retries >= max_retries && is_discover {
            unsafe { DHCP_STATE = State::Idle; }
            return false;
        }

        if is_discover || matches!(unsafe { DHCP_STATE }, State::Requesting) {
            let msg_type = if is_discover { MSG_DISCOVER } else { MSG_REQUEST };
            let ciaddr = if is_discover { zero_ip } else { unsafe { OFFERED_IP } };
            let server_id = if is_discover { None } else { Some(unsafe { SERVER_ID }) };
            let req_ip = if is_discover { None } else { Some(unsafe { OFFERED_IP }) };

            let dhcp_msg = build_dhcp_msg(OP_REQUEST, xid, ciaddr, &mac, msg_type, server_id, req_ip);
            send_dhcp_udp(iface_idx, &dhcp_msg, zero_ip, bcast_ip);
            retries += 1;
        }

        let mut waited = 0;
        while waited < timeout_ticks {
            nic::net_poll();
            unsafe {
                if RESP_READY {
                    if let Some(iface2) = nic::get_iface(iface_idx) {
                        dhcp_handle_response(iface2.mac);
                    }
                }
            }
            if matches!(unsafe { DHCP_STATE }, State::Bound | State::Idle) {
                break;
            }
            waited += 1;
        }
    }
}

fn send_dhcp_udp(iface_idx: usize, dhcp_msg: &[u8; 548], src_ip: [u8; 4], dst_ip: [u8; 4]) {
    let udp_len = 8 + 548;
    let mut udp_buf = [0u8; 1500];
    udp_buf[0..2].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
    udp_buf[2..4].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
    udp_buf[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    udp_buf[8..udp_len].copy_from_slice(dhcp_msg);

    let total_len = 20 + udp_len;
    let mut ip_pkt = [0u8; 1500];
    ip_pkt[0] = 0x45;
    ip_pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip_pkt[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip_pkt[8] = 64;
    ip_pkt[9] = ipv4::PROTO_UDP;
    ip_pkt[12..16].copy_from_slice(&src_ip);
    ip_pkt[16..20].copy_from_slice(&dst_ip);
    ip_pkt[20..total_len].copy_from_slice(&udp_buf[..udp_len]);

    let csum = ipv4::internet_checksum(&ip_pkt[..20]);
    ip_pkt[10] = (csum >> 8) as u8;
    ip_pkt[11] = (csum & 0xFF) as u8;

    nic::send_broadcast_packet(iface_idx, &ip_pkt[..total_len]);
}

fn dhcp_handle_response(our_mac: [u8; 6]) -> bool {
    unsafe {
        if !RESP_READY {
            return false;
        }
        RESP_READY = false;

        let payload = &RESP_BUF[..RESP_LEN];
        let (hdr, udp_data) = match udp::parse(payload) {
            Some(h) => h,
            None => return false,
        };

        if hdr.src_port != DHCP_SERVER_PORT || hdr.dst_port != DHCP_CLIENT_PORT {
            return false;
        }

        if udp_data.len() < 240 {
            return false;
        }

        let magic = [udp_data[236], udp_data[237], udp_data[238], udp_data[239]];
        if magic != MAGIC_COOKIE {
            return false;
        }

        let rxid = u32::from_be_bytes([udp_data[4], udp_data[5], udp_data[6], udp_data[7]]);
        if rxid != XID {
            return false;
        }

        let op = udp_data[0];
        if op != OP_REPLY {
            return false;
        }

        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(&our_mac);
        if udp_data[28..34] != chaddr[..6] {
            return false;
        }

        let (msg_type, server_id, subnet_mask, gateway, lease_time) = parse_dhcp_options(udp_data);

        match msg_type {
            MSG_OFFER => {
                if !matches!(DHCP_STATE, State::Discovering) {
                    return false;
                }
                OFFERED_IP.copy_from_slice(&udp_data[16..20]);
                SERVER_ID = server_id;
                LEASE_TIME = lease_time;
                DHCP_STATE = State::Requesting;
            }
            MSG_ACK => {
                if !matches!(DHCP_STATE, State::Requesting) {
                    return false;
                }
                let yiaddr = [udp_data[16], udp_data[17], udp_data[18], udp_data[19]];
                let gw = if gateway == [0; 4] { server_id } else { gateway };
                nic::set_iface_ip(1, yiaddr, subnet_mask, gw);
                crate::route::clear();
                crate::route::add_direct(
                    [yiaddr[0] & subnet_mask[0], yiaddr[1] & subnet_mask[1], yiaddr[2] & subnet_mask[2], yiaddr[3] & subnet_mask[3]],
                    subnet_mask, 1,
                );
                crate::route::add_default(gw, 1);
                DHCP_STATE = State::Bound;
            }
            _ => {}
        }
        true
    }
}
