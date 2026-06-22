use crate::ipv4;
use zenus_console::serial::SerialPort;

pub const TCP_CLOSED: u8 = 0;
pub const TCP_LISTEN: u8 = 1;
pub const TCP_SYN_SENT: u8 = 2;
pub const TCP_SYN_RECEIVED: u8 = 3;
pub const TCP_ESTABLISHED: u8 = 4;
pub const TCP_FIN_WAIT1: u8 = 5;
pub const TCP_FIN_WAIT2: u8 = 6;
pub const TCP_CLOSE_WAIT: u8 = 7;
pub const TCP_CLOSING: u8 = 8;
pub const TCP_LAST_ACK: u8 = 9;
pub const TCP_TIME_WAIT: u8 = 10;

const TCP_FLAG_FIN: u8 = 0x01;
const TCP_FLAG_SYN: u8 = 0x02;
const TCP_FLAG_RST: u8 = 0x04;
const TCP_FLAG_PSH: u8 = 0x08;
const TCP_FLAG_ACK: u8 = 0x10;

pub const MAX_CONNS: usize = 16;
const MAX_RETRIES: u8 = 5;
const RETRY_INTERVAL: u8 = 10;

#[derive(Clone, Copy)]
struct Tcb {
    state: u8,
    local_ip: [u8; 4],
    local_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    send_una: u32,
    send_nxt: u32,
    recv_nxt: u32,
    recv_window: u16,
    listening: bool,
    rx_data: [u8; 4096],
    rx_data_len: usize,
    tx_data: [u8; 4096],
    tx_data_len: usize,
    retry_count: u8,
    retry_ticks: u8,
    last_ack: u32,
    time_wait_ticks: u8,
}

static mut TCP_CONNS: [Option<Tcb>; MAX_CONNS] = [None; MAX_CONNS];
static mut NEXT_CONN_ID: usize = 0;

fn seq_before(a: u32, b: u32) -> bool {
    ((a.wrapping_sub(b)) as i32) < 0
}

fn seq_before_eq(a: u32, b: u32) -> bool {
    a == b || seq_before(a, b)
}

fn find_slot() -> Option<usize> {
    unsafe {
        for i in 0..MAX_CONNS {
            if TCP_CONNS[i].is_none() {
                return Some(i);
            }
        }
        for i in 0..MAX_CONNS {
            if let Some(ref t) = TCP_CONNS[i] {
                if t.state == TCP_CLOSED {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn conn_by_port(port: u16) -> Option<usize> {
    unsafe {
        for i in 0..MAX_CONNS {
            if let Some(ref t) = TCP_CONNS[i] {
                if t.listening && t.local_port == port {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn checksum(src_ip: [u8; 4], dst_ip: [u8; 4], segment: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    sum += u32::from(u16::from_be_bytes([src_ip[0], src_ip[1]]));
    sum += u32::from(u16::from_be_bytes([src_ip[2], src_ip[3]]));
    sum += u32::from(u16::from_be_bytes([dst_ip[0], dst_ip[1]]));
    sum += u32::from(u16::from_be_bytes([dst_ip[2], dst_ip[3]]));
    sum += 0x0006;
    let tcp_len = segment.len() as u16;
    sum += u32::from(tcp_len);
    let mut i = 0;
    while i + 1 < segment.len() {
        sum += u32::from(u16::from_be_bytes([segment[i], segment[i + 1]]));
        i += 2;
    }
    if i < segment.len() {
        sum += u32::from(segment[i]) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn build_segment(
    src_port: u16, dst_port: u16,
    seq: u32, ack: u32,
    flags: u8, window: u16,
    payload: &[u8],
) -> [u8; 1500] {
    let mut seg = [0u8; 1500];
    let payload_len = core::cmp::min(payload.len(), 1480);
    seg[0..2].copy_from_slice(&src_port.to_be_bytes());
    seg[2..4].copy_from_slice(&dst_port.to_be_bytes());
    seg[4..8].copy_from_slice(&seq.to_be_bytes());
    seg[8..12].copy_from_slice(&ack.to_be_bytes());
    seg[12] = 0x50;
    seg[13] = flags;
    seg[14..16].copy_from_slice(&window.to_be_bytes());
    seg[20..20 + payload_len].copy_from_slice(&payload[..payload_len]);
    seg
}

fn rand_isn() -> u32 {
    let r = zenus_arch::random::get_random_u64();
    (r & 0xFFFFFFFF) as u32
}

pub fn listen(port: u16) -> Option<usize> {
    unsafe {
        if conn_by_port(port).is_some() {
            return None;
        }
        let idx = find_slot()?;
        TCP_CONNS[idx] = Some(Tcb {
            state: TCP_LISTEN,
            local_ip: [0; 4],
            local_port: port,
            remote_ip: [0; 4],
            remote_port: 0,
            send_una: 0,
            send_nxt: 0,
            recv_nxt: 0,
            recv_window: 4096,
            listening: true,
            rx_data: [0; 4096],
            rx_data_len: 0,
            tx_data: [0; 4096],
            tx_data_len: 0,
            retry_count: 0,
            retry_ticks: 0,
            last_ack: 0,
            time_wait_ticks: 0,
        });
        Some(idx)
    }
}

pub fn connect(iface_idx: usize, local_port: u16, dst_ip: [u8; 4], dst_port: u16) -> Option<usize> {
    unsafe {
        let idx = find_slot()?;
        let local_ip = crate::nic::get_iface(iface_idx)
            .map(|iface| iface.ip)
            .unwrap_or([0; 4]);
        if local_ip == [0; 4] || local_ip == [127, 0, 0, 1] {
            return None;
        }
        let isn = rand_isn();

        let mut seg = build_segment(
            local_port, dst_port,
            isn, 0,
            TCP_FLAG_SYN,
            65535,
            &[],
        );
        let csum = checksum(local_ip, dst_ip, &seg[..20]);
        seg[16] = (csum >> 8) as u8;
        seg[17] = (csum & 0xFF) as u8;
        let sent = ipv4::send(iface_idx, dst_ip, ipv4::PROTO_TCP, &seg[..20]);
        if !sent {
            return None;
        }

        TCP_CONNS[idx] = Some(Tcb {
            state: TCP_SYN_SENT,
            local_ip,
            local_port,
            remote_ip: dst_ip,
            remote_port: dst_port,
            send_una: isn,
            send_nxt: isn + 1,
            recv_nxt: 0,
            recv_window: 4096,
            listening: false,
            rx_data: [0; 4096],
            rx_data_len: 0,
            tx_data: [0; 4096],
            tx_data_len: 0,
            retry_count: MAX_RETRIES,
            retry_ticks: RETRY_INTERVAL,
            last_ack: 0,
            time_wait_ticks: 0,
        });

        let mut s = SerialPort::new(0x3F8);
        s.write_str("[TCP] connect sending SYN from ");
        s.write_u64(local_port as u64);
        s.write_str("->");
        s.write_u64(dst_port as u64);
        s.write_str(" isn=");
        s.write_u64(isn as u64);
        s.write_str("\n");

        Some(idx)
    }
}

pub fn handle_receive(
    iface_idx: usize,
    src_ip: [u8; 4], dst_ip: [u8; 4],
    segment: &[u8],
) -> bool {
    if segment.len() < 20 {
        return false;
    }
    if checksum(src_ip, dst_ip, segment) != 0 {
        return false;
    }

    let src_port = u16::from_be_bytes([segment[0], segment[1]]);
    let dst_port = u16::from_be_bytes([segment[2], segment[3]]);
    let seq = u32::from_be_bytes([segment[4], segment[5], segment[6], segment[7]]);
    let ack = u32::from_be_bytes([segment[8], segment[9], segment[10], segment[11]]);
    let flags = segment[13];
    let window = u16::from_be_bytes([segment[14], segment[15]]);
    let hdr_len = ((segment[12] >> 4) * 4) as usize;
    if hdr_len < 20 || hdr_len > segment.len() {
        return false;
    }
    let payload = &segment[hdr_len..];

    let log = if (flags & TCP_FLAG_SYN) != 0 || (flags & TCP_FLAG_FIN) != 0 || (flags & TCP_FLAG_RST) != 0 || !payload.is_empty() {
        true
    } else {
        false
    };

    if log {
        let mut s = SerialPort::new(0x3F8);
        s.write_str("[TCP-IN] ");
        s.write_u64(src_ip[0] as u64); s.write_str(".");
        s.write_u64(src_ip[1] as u64); s.write_str(".");
        s.write_u64(src_ip[2] as u64); s.write_str(".");
        s.write_u64(src_ip[3] as u64);
        s.write_str(":");
        s.write_u64(src_port as u64);
        s.write_str("->");
        s.write_u64(dst_ip[0] as u64); s.write_str(".");
        s.write_u64(dst_ip[1] as u64); s.write_str(".");
        s.write_u64(dst_ip[2] as u64); s.write_str(".");
        s.write_u64(dst_ip[3] as u64);
        s.write_str(":");
        s.write_u64(dst_port as u64);
        s.write_str(" flg=0x");
        s.write_hex(flags as u64);
        s.write_str(" seq=");
        s.write_u64(seq as u64);
        s.write_str(" ack=");
        s.write_u64(ack as u64);
        s.write_str(" plen=");
        s.write_u64(payload.len() as u64);
        s.write_str("\n");
    }

    let conn_idx = unsafe {
        let mut found = None;
        for i in 0..MAX_CONNS {
            match &TCP_CONNS[i] {
                Some(tcb) if tcb.listening && tcb.local_port == dst_port => {
                    found = Some(i);
                }
                Some(tcb) if !tcb.listening
                    && tcb.local_port == dst_port
                    && tcb.remote_port == src_port
                    && tcb.remote_ip == src_ip =>
                {
                    found = Some(i);
                }
                _ => {}
            }
        }
        found
    };

    let conn_idx = match conn_idx {
        Some(idx) => idx,
        None => {
            if (flags & TCP_FLAG_RST) == 0 && (flags & TCP_FLAG_SYN) == 0 {
                let rst_seq = if (flags & TCP_FLAG_ACK) != 0 { ack } else { 0 };
                let mut rst = build_segment(dst_port, src_port, rst_seq, seq + payload.len() as u32 + if (flags & TCP_FLAG_FIN) != 0 { 1 } else { 0 }, TCP_FLAG_RST | TCP_FLAG_ACK, 0, &[]);
                let csum = checksum(dst_ip, src_ip, &rst[..20]);
                rst[16] = (csum >> 8) as u8;
                rst[17] = (csum & 0xFF) as u8;
                ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &rst[..20]);
            }
            return false;
        }
    };

    unsafe {
        let tcb = match &mut TCP_CONNS[conn_idx] {
            Some(t) => t,
            None => return false,
        };

        if (flags & TCP_FLAG_RST) != 0 {
            tcb.state = TCP_CLOSED;
            return true;
        }

        match tcb.state {
            TCP_LISTEN => {
                if (flags & TCP_FLAG_SYN) != 0 && (flags & TCP_FLAG_ACK) == 0 {
                    let child = match find_slot() {
                        Some(idx) => idx,
                        None => return false,
                    };
                    let isn = rand_isn();

                    let mut syn_ack = build_segment(
                        dst_port, src_port,
                        isn, seq + 1,
                        TCP_FLAG_SYN | TCP_FLAG_ACK,
                        65535,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &syn_ack[..20]);
                    syn_ack[16] = (csum >> 8) as u8;
                    syn_ack[17] = (csum & 0xFF) as u8;
                    let sent = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &syn_ack[..20]);

                    if sent {
                        TCP_CONNS[child] = Some(Tcb {
                            state: TCP_SYN_RECEIVED,
                            local_ip: dst_ip,
                            local_port: dst_port,
                            remote_ip: src_ip,
                            remote_port: src_port,
                            send_una: isn,
                            send_nxt: isn + 1,
                            recv_nxt: seq + 1,
                            recv_window: 4096,
                            listening: false,
                            rx_data: [0; 4096],
                            rx_data_len: 0,
                            tx_data: [0; 4096],
                            tx_data_len: 0,
                            retry_count: MAX_RETRIES,
                            retry_ticks: RETRY_INTERVAL,
                            last_ack: 0,
                            time_wait_ticks: 0,
                        });
                    }
                }
            }

            TCP_SYN_SENT => {
                if (flags & TCP_FLAG_SYN) != 0 && (flags & TCP_FLAG_ACK) != 0 {
                    tcb.recv_nxt = seq + 1;
                    tcb.send_una = ack;
                    tcb.recv_window = window;

                    let mut ack_seg = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, tcb.recv_nxt,
                        TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                    ack_seg[16] = (csum >> 8) as u8;
                    ack_seg[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);

                    tcb.state = TCP_ESTABLISHED;
                    tcb.retry_count = 0;
                    tcb.retry_ticks = 0;

                    let mut s = SerialPort::new(0x3F8);
                    s.write_str("[TCP] connection ESTABLISHED\n");
                } else if (flags & TCP_FLAG_SYN) != 0 {
                    let mut syn_ack = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, seq + 1,
                        TCP_FLAG_SYN | TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &syn_ack[..20]);
                    syn_ack[16] = (csum >> 8) as u8;
                    syn_ack[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &syn_ack[..20]);
                }
            }

            TCP_SYN_RECEIVED => {
                if (flags & TCP_FLAG_ACK) != 0 {
                    if seq != tcb.recv_nxt {
                        let mut ack_seg = build_segment(
                            dst_port, src_port,
                            tcb.send_nxt, tcb.recv_nxt,
                            TCP_FLAG_ACK,
                            tcb.recv_window,
                            &[],
                        );
                        let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                        ack_seg[16] = (csum >> 8) as u8;
                        ack_seg[17] = (csum & 0xFF) as u8;
                        let _ = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                    } else {
                        tcb.state = TCP_ESTABLISHED;
                        tcb.send_una = ack;
                        tcb.recv_window = window;
                        tcb.retry_count = 0;
                        tcb.retry_ticks = 0;

                        let mut s = SerialPort::new(0x3F8);
                        s.write_str("[TCP] SYN_RCVD->ESTABLISHED\n");

                        if !payload.is_empty() {
                            let copy_len = core::cmp::min(payload.len(), tcb.rx_data.len() - tcb.rx_data_len);
                            if copy_len > 0 {
                                tcb.rx_data[tcb.rx_data_len..tcb.rx_data_len + copy_len].copy_from_slice(&payload[..copy_len]);
                                tcb.rx_data_len += copy_len;
                            }
                            tcb.recv_nxt = seq + copy_len as u32;

                            let mut ack_seg = build_segment(
                                dst_port, src_port,
                                tcb.send_nxt, tcb.recv_nxt,
                                TCP_FLAG_ACK,
                                tcb.recv_window,
                                &[],
                            );
                            let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                            ack_seg[16] = (csum >> 8) as u8;
                            ack_seg[17] = (csum & 0xFF) as u8;
                            let _ = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                        }
                    }
                } else if (flags & TCP_FLAG_SYN) != 0 && (flags & TCP_FLAG_ACK) == 0 {
                    let mut syn_ack = build_segment(
                        dst_port, src_port,
                        tcb.send_una, tcb.recv_nxt,
                        TCP_FLAG_SYN | TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &syn_ack[..20]);
                    syn_ack[16] = (csum >> 8) as u8;
                    syn_ack[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &syn_ack[..20]);
                }
            }

            TCP_ESTABLISHED | TCP_CLOSE_WAIT => {
                if (flags & TCP_FLAG_ACK) != 0 {
                    if seq_before(tcb.send_una, ack) && seq_before_eq(ack, tcb.send_nxt) {
                        let acked_bytes = ack.wrapping_sub(tcb.send_una);
                        tcb.send_una = ack;
                        // Remove acknowledged data from tx buffer
                        if acked_bytes as usize <= tcb.tx_data_len {
                            let acked = acked_bytes as usize;
                            if acked < tcb.tx_data_len {
                                tcb.tx_data.copy_within(acked..tcb.tx_data_len, 0);
                            }
                            tcb.tx_data_len -= acked;
                        }
                        if !seq_before(tcb.send_una, tcb.send_nxt) {
                            tcb.retry_count = 0;
                            tcb.retry_ticks = 0;
                        }
                    }
                    tcb.recv_window = window;
                }

                if !payload.is_empty() && (flags & TCP_FLAG_ACK) != 0 {
                    if seq == tcb.recv_nxt {
                        let copy_len = core::cmp::min(payload.len(), tcb.rx_data.len() - tcb.rx_data_len);
                        if copy_len > 0 {
                            tcb.rx_data[tcb.rx_data_len..tcb.rx_data_len + copy_len].copy_from_slice(&payload[..copy_len]);
                            tcb.rx_data_len += copy_len;
                        }
                        tcb.recv_nxt = seq + copy_len as u32;

                        let window = 65535u16.saturating_sub(tcb.rx_data_len as u16);

                        let mut ack_seg = build_segment(
                            dst_port, src_port,
                            tcb.send_nxt, tcb.recv_nxt,
                            TCP_FLAG_ACK,
                            window,
                            &[],
                        );
                        let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                        ack_seg[16] = (csum >> 8) as u8;
                        ack_seg[17] = (csum & 0xFF) as u8;
                        let _ = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                    } else {
                        let mut ack_seg = build_segment(
                            dst_port, src_port,
                            tcb.send_nxt, tcb.recv_nxt,
                            TCP_FLAG_ACK,
                            tcb.recv_window,
                            &[],
                        );
                        let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                        ack_seg[16] = (csum >> 8) as u8;
                        ack_seg[17] = (csum & 0xFF) as u8;
                        let _ = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                    }
                }

                if (flags & TCP_FLAG_FIN) != 0 {
                    tcb.recv_nxt = tcb.recv_nxt.wrapping_add(1);
                    if tcb.state == TCP_ESTABLISHED {
                        tcb.state = TCP_CLOSE_WAIT;
                    }
                    let mut ack_seg = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, tcb.recv_nxt,
                        TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                    ack_seg[16] = (csum >> 8) as u8;
                    ack_seg[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                }
            }

            TCP_FIN_WAIT1 => {
                if (flags & TCP_FLAG_RST) != 0 {
                    tcb.state = TCP_CLOSED;
                } else if (flags & TCP_FLAG_ACK) != 0 {
                    if ack >= tcb.send_nxt {
                        tcb.state = TCP_FIN_WAIT2;
                    }
                }
                if (flags & TCP_FLAG_FIN) != 0 {
                    tcb.recv_nxt = seq + payload.len() as u32 + 1;
                    let mut ack_seg = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, tcb.recv_nxt,
                        TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                    ack_seg[16] = (csum >> 8) as u8;
                    ack_seg[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                    tcb.state = TCP_CLOSING;
                }
            }

            TCP_FIN_WAIT2 => {
                if (flags & TCP_FLAG_RST) != 0 {
                    tcb.state = TCP_CLOSED;
                } else if (flags & TCP_FLAG_FIN) != 0 {
                    tcb.recv_nxt = seq + payload.len() as u32 + 1;
                    let mut ack_seg = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, tcb.recv_nxt,
                        TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                    ack_seg[16] = (csum >> 8) as u8;
                    ack_seg[17] = (csum & 0xFF) as u8;
                    ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                    tcb.state = TCP_TIME_WAIT;
                    tcb.time_wait_ticks = 20;
                }
            }

            TCP_TIME_WAIT => {
                if (flags & TCP_FLAG_FIN) != 0 {
                    let mut ack_seg = build_segment(
                        dst_port, src_port,
                        tcb.send_nxt, tcb.recv_nxt,
                        TCP_FLAG_ACK,
                        tcb.recv_window,
                        &[],
                    );
                    let csum = checksum(dst_ip, src_ip, &ack_seg[..20]);
                    ack_seg[16] = (csum >> 8) as u8;
                    ack_seg[17] = (csum & 0xFF) as u8;
                    let _ = ipv4::send(iface_idx, src_ip, ipv4::PROTO_TCP, &ack_seg[..20]);
                }
            }

            TCP_CLOSING => {
                if (flags & TCP_FLAG_ACK) != 0 {
                    if ack >= tcb.send_nxt {
                        tcb.state = TCP_TIME_WAIT;
                        tcb.time_wait_ticks = 20;
                    }
                }
            }

            TCP_LAST_ACK => {
                if (flags & TCP_FLAG_RST) != 0 {
                    tcb.state = TCP_CLOSED;
                } else if (flags & TCP_FLAG_ACK) != 0 {
                    if ack >= tcb.send_nxt {
                        tcb.state = TCP_CLOSED;
                    }
                }
            }

            _ => {}
        }
    }

    true
}

pub fn poll_retransmit(iface_idx: usize) {
    unsafe {
        for i in 0..MAX_CONNS {
            let tcb = match &mut TCP_CONNS[i] {
                Some(t) => t,
                None => continue,
            };
            if tcb.listening {
                continue;
            }
            if tcb.state == TCP_TIME_WAIT {
                if tcb.time_wait_ticks == 0 {
                    tcb.time_wait_ticks = 20;
                } else {
                    tcb.time_wait_ticks -= 1;
                    if tcb.time_wait_ticks == 0 {
                        tcb.state = TCP_CLOSED;
                    }
                }
                continue;
            }
            match tcb.state {
                TCP_SYN_SENT | TCP_SYN_RECEIVED | TCP_ESTABLISHED | TCP_CLOSE_WAIT | TCP_LAST_ACK | TCP_FIN_WAIT1 | TCP_CLOSING => {}
                _ => continue,
            }
            if !seq_before(tcb.send_una, tcb.send_nxt) {
                tcb.retry_count = 0;
                tcb.retry_ticks = 0;
                continue;
            }
            if tcb.retry_count == 0 {
                continue;
            }
            if tcb.retry_ticks > 0 {
                tcb.retry_ticks -= 1;
                continue;
            }

            let remote_ip = tcb.remote_ip;
            let local_ip = tcb.local_ip;
            let src_port = tcb.local_port;
            let dst_port = tcb.remote_port;
            let send_una = tcb.send_una;

            if tcb.state == TCP_SYN_SENT || tcb.state == TCP_SYN_RECEIVED {
                let mut seg = build_segment(
                    src_port, dst_port,
                    send_una, tcb.recv_nxt,
                    if tcb.state == TCP_SYN_SENT { TCP_FLAG_SYN } else { TCP_FLAG_SYN | TCP_FLAG_ACK },
                    tcb.recv_window,
                    &[],
                );
                let csum = checksum(local_ip, remote_ip, &seg[..20]);
                seg[16] = (csum >> 8) as u8;
                seg[17] = (csum & 0xFF) as u8;
                ipv4::send(iface_idx, remote_ip, ipv4::PROTO_TCP, &seg[..20]);
            } else if tcb.state == TCP_LAST_ACK || tcb.state == TCP_FIN_WAIT1 || tcb.state == TCP_CLOSING {
                let mut seg = build_segment(
                    src_port, dst_port,
                    send_una, tcb.recv_nxt,
                    TCP_FLAG_FIN | TCP_FLAG_ACK,
                    tcb.recv_window,
                    &[],
                );
                let csum = checksum(local_ip, remote_ip, &seg[..20]);
                seg[16] = (csum >> 8) as u8;
                seg[17] = (csum & 0xFF) as u8;
                ipv4::send(iface_idx, remote_ip, ipv4::PROTO_TCP, &seg[..20]);
            } else {
                let payload = &tcb.tx_data[..core::cmp::min(tcb.tx_data_len, 1460)];
                let seg_len = 20 + payload.len();
                let mut seg = build_segment(
                    src_port, dst_port,
                    send_una,
                    tcb.recv_nxt,
                    TCP_FLAG_ACK | TCP_FLAG_PSH,
                    tcb.recv_window,
                    payload,
                );
                let csum = checksum(local_ip, remote_ip, &seg[..seg_len]);
                seg[16] = (csum >> 8) as u8;
                seg[17] = (csum & 0xFF) as u8;
                ipv4::send(iface_idx, remote_ip, ipv4::PROTO_TCP, &seg[..seg_len]);
            }

            tcb.retry_count -= 1;
            tcb.retry_ticks = RETRY_INTERVAL;

            if tcb.retry_count == 0 {
                let mut s = SerialPort::new(0x3F8);
                s.write_str("[TCP] retry exhausted, closing conn ");
                s.write_u64(i as u64);
                s.write_str("\n");
                tcb.state = TCP_CLOSED;
            }
        }
    }
}

pub fn send_data(conn: usize, data: &[u8]) -> bool {
    unsafe {
        let tcb = match &mut TCP_CONNS[conn] {
            Some(t) if t.state == TCP_ESTABLISHED || t.state == TCP_CLOSE_WAIT => t,
            _ => return false,
        };

        let available = tcb.tx_data.len().saturating_sub(tcb.tx_data_len);
        let copy_len = core::cmp::min(data.len(), available);
        if copy_len > 0 {
            tcb.tx_data[tcb.tx_data_len..tcb.tx_data_len + copy_len].copy_from_slice(&data[..copy_len]);
            tcb.tx_data_len += copy_len;
        }
        copy_len > 0
    }
}

pub fn flush_tx(conn: usize, iface_idx: usize) -> bool {
    unsafe {
        let tcb = match &mut TCP_CONNS[conn] {
            Some(t) if t.state == TCP_ESTABLISHED || t.state == TCP_CLOSE_WAIT => t,
            _ => return false,
        };

        if tcb.tx_data_len == 0 {
            return true;
        }

        let payload_len = core::cmp::min(tcb.tx_data_len, 1460);
        let payload = &tcb.tx_data[..payload_len];

        let mut seg = build_segment(
            tcb.local_port, tcb.remote_port,
            tcb.send_nxt, tcb.recv_nxt,
            TCP_FLAG_ACK | TCP_FLAG_PSH,
            tcb.recv_window,
            payload,
        );
        let seg_len = 20 + payload_len;
        let csum = checksum(tcb.local_ip, tcb.remote_ip, &seg[..seg_len]);
        seg[16] = (csum >> 8) as u8;
        seg[17] = (csum & 0xFF) as u8;
        let result = ipv4::send(iface_idx, tcb.remote_ip, ipv4::PROTO_TCP, &seg[..seg_len]);
        if result {
            tcb.send_nxt += payload_len as u32;

            if tcb.tx_data_len > 0 {
                tcb.retry_count = MAX_RETRIES;
                tcb.retry_ticks = RETRY_INTERVAL;
            }
        }
        result
    }
}

pub fn receive_data(conn: usize, buf: &mut [u8]) -> Option<usize> {
    unsafe {
        let tcb = match &mut TCP_CONNS[conn] {
            Some(t) if t.rx_data_len > 0 => t,
            _ => return None,
        };

        let copy_len = core::cmp::min(tcb.rx_data_len, buf.len());
        buf[..copy_len].copy_from_slice(&tcb.rx_data[..copy_len]);
        if copy_len < tcb.rx_data_len {
            tcb.rx_data.copy_within(copy_len..tcb.rx_data_len, 0);
        }
        tcb.rx_data_len -= copy_len;
        Some(copy_len)
    }
}

pub fn close(conn: usize, iface_idx: usize) -> bool {
    unsafe {
        let tcb = match &mut TCP_CONNS[conn] {
            Some(t) if t.state == TCP_ESTABLISHED || t.state == TCP_CLOSE_WAIT => t,
            _ => return false,
        };

        let mut fin = build_segment(
            tcb.local_port, tcb.remote_port,
            tcb.send_nxt, tcb.recv_nxt,
            TCP_FLAG_FIN | TCP_FLAG_ACK,
            tcb.recv_window,
            &[],
        );
        let csum = checksum(tcb.local_ip, tcb.remote_ip, &fin[..20]);
        fin[16] = (csum >> 8) as u8;
        fin[17] = (csum & 0xFF) as u8;
        let result = ipv4::send(iface_idx, tcb.remote_ip, ipv4::PROTO_TCP, &fin[..20]);
        if result {
            tcb.send_nxt += 1;
            tcb.state = if tcb.state == TCP_CLOSE_WAIT { TCP_LAST_ACK } else { TCP_FIN_WAIT1 };
            tcb.retry_count = MAX_RETRIES;
            tcb.retry_ticks = RETRY_INTERVAL;
        }
        result
    }
}

pub fn close_conn(conn: usize) {
    unsafe {
        TCP_CONNS[conn] = None;
    }
}

pub fn is_connected(conn: usize) -> bool {
    unsafe {
        match &TCP_CONNS[conn] {
            Some(t) => t.state == TCP_ESTABLISHED || t.state == TCP_CLOSE_WAIT,
            None => false,
        }
    }
}

pub fn has_data(conn: usize) -> bool {
    unsafe {
        match &TCP_CONNS[conn] {
            Some(t) => t.rx_data_len > 0,
            None => false,
        }
    }
}

pub fn get_conn_info(conn: usize) -> Option<(u8, [u8; 4], u16, [u8; 4], u16)> {
    unsafe {
        match &TCP_CONNS[conn] {
            Some(t) => Some((t.state, t.remote_ip, t.remote_port, t.local_ip, t.local_port)),
            None => None,
        }
    }
}

pub fn state_name(conn: usize) -> &'static str {
    unsafe {
        match &TCP_CONNS[conn] {
            Some(t) => match t.state {
                TCP_CLOSED => "CLOSED",
                TCP_LISTEN => "LISTEN",
                TCP_SYN_SENT => "SYN_SENT",
                TCP_SYN_RECEIVED => "SYN_RCVD",
                TCP_ESTABLISHED => "ESTAB",
                TCP_FIN_WAIT1 => "FIN_WAIT1",
                TCP_FIN_WAIT2 => "FIN_WAIT2",
                TCP_CLOSE_WAIT => "CLOSE_WAIT",
                TCP_CLOSING => "CLOSING",
                TCP_LAST_ACK => "LAST_ACK",
                TCP_TIME_WAIT => "TIME_WAIT",
                _ => "UNKNOWN",
            },
            None => "NONE",
        }
    }
}

pub fn connection_count() -> usize {
    unsafe {
        let mut count = 0;
        for i in 0..MAX_CONNS {
            if TCP_CONNS[i].is_some() {
                count += 1;
            }
        }
        count
    }
}
