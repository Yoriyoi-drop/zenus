use crate::ipv4;

use zenus_sync::spinlock::SpinLock;

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

pub const MAX_CONNS: usize = 256;
const MAX_RETRIES: u8 = 5;
const RETRY_INTERVAL: u8 = 10;
const MSS: u16 = 1460;
const INIT_CWND: u16 = 1460;
const INIT_SSTHRESH: u16 = 65535;
const KEEPALIVE_IDLE: u64 = 7200;
const KEEPALIVE_PROBE_INTERVAL: u64 = 75;
const KEEPALIVE_PROBES: u8 = 9;

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
    cwnd: u16,
    ssthresh: u16,
    dupack_count: u8,
    last_ack_seq: u32,
    keepalive_probes: u8,
    keepalive_time: u64,
    sack_blocks: [(u32, u32); 4],
}

struct TcpState {
    conns: [Option<Tcb>; MAX_CONNS],
    next_conn_id: usize,
}

static TCP_STATE: SpinLock<TcpState> = SpinLock::new(TcpState {
    conns: [None; MAX_CONNS],
    next_conn_id: 0,
});

fn seq_before(a: u32, b: u32) -> bool {
    ((a.wrapping_sub(b)) as i32) < 0
}

fn seq_before_eq(a: u32, b: u32) -> bool {
    a == b || seq_before(a, b)
}

fn find_slot(state: &TcpState) -> Option<usize> {
    for i in 0..MAX_CONNS {
        if state.conns[i].is_none() {
            return Some(i);
        }
    }
    for i in 0..MAX_CONNS {
        if let Some(ref t) = state.conns[i] {
            if t.state == TCP_CLOSED {
                return Some(i);
            }
        }
    }
    None
}

fn conn_by_port(port: u16, state: &TcpState) -> Option<usize> {
    for i in 0..MAX_CONNS {
        if let Some(ref t) = state.conns[i] {
            if t.listening && t.local_port == port {
                return Some(i);
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

fn build_syn_segment(
    src_port: u16, dst_port: u16,
    seq: u32, ack: u32,
    flags: u8, window: u16,
) -> [u8; 1500] {
    let mut seg = build_segment(src_port, dst_port, seq, ack, flags, window, &[]);
    seg[20] = 2; seg[21] = 4;
    seg[22] = (MSS >> 8) as u8; seg[23] = (MSS & 0xFF) as u8;
    seg[24] = 3; seg[25] = 3; seg[26] = 7;
    seg[27] = 4; seg[28] = 2;
    seg[29] = 1; seg[30] = 1; seg[31] = 1;
    seg[12] = 0x80;
    seg
}

fn add_sack_block(blocks: &mut [(u32, u32); 4], left: u32, right: u32) {
    let mut new_left = left;
    let mut new_right = right;
    for i in 0..4 {
        let (l, r) = blocks[i];
        if l == 0 && r == 0 { continue; }
        if r >= new_left && l <= new_right {
            new_left = core::cmp::min(new_left, l);
            new_right = core::cmp::max(new_right, r);
            blocks[i] = (0, 0);
        }
    }
    for i in 0..4 {
        if blocks[i].0 == 0 && blocks[i].1 == 0 {
            blocks[i] = (new_left, new_right);
            return;
        }
    }
    for i in 0..3 {
        blocks[i] = blocks[i + 1];
    }
    blocks[3] = (new_left, new_right);
}

fn send_sack_ack(
    iface_idx: usize,
    src_ip: [u8; 4], dst_ip: [u8; 4],
    src_port: u16, dst_port: u16,
    seq: u32, ack: u32,
    window: u16,
    sack_blocks: &[(u32, u32); 4],
) {
    let mut seg = [0u8; 1500];
    seg[0..2].copy_from_slice(&src_port.to_be_bytes());
    seg[2..4].copy_from_slice(&dst_port.to_be_bytes());
    seg[4..8].copy_from_slice(&seq.to_be_bytes());
    seg[8..12].copy_from_slice(&ack.to_be_bytes());
    seg[12] = 0x50;
    seg[13] = TCP_FLAG_ACK;
    seg[14..16].copy_from_slice(&window.to_be_bytes());

    let mut n = 0;
    for i in 0..4 {
        if sack_blocks[i].0 != 0 || sack_blocks[i].1 != 0 {
            n += 1;
        }
    }

    let mut hdr_len = 20;
    if n > 0 {
        let opt_len = 2 + n * 8;
        let padded = (opt_len + 3) & !3;
        hdr_len = 20 + padded;
        seg[20] = 5;
        seg[21] = opt_len as u8;
        let mut off = 22;
        for i in 0..4 {
            let (l, r) = sack_blocks[i];
            if l != 0 || r != 0 {
                seg[off..off + 4].copy_from_slice(&l.to_be_bytes());
                seg[off + 4..off + 8].copy_from_slice(&r.to_be_bytes());
                off += 8;
            }
        }
        while off < hdr_len {
            seg[off] = 1;
            off += 1;
        }
        seg[12] = ((hdr_len >> 2) as u8) << 4;
    }

    let csum = checksum(src_ip, dst_ip, &seg[..hdr_len]);
    seg[16] = (csum >> 8) as u8;
    seg[17] = (csum & 0xFF) as u8;
    ipv4::send(iface_idx, dst_ip, ipv4::PROTO_TCP, &seg[..hdr_len]);
}

fn rand_isn() -> u32 {
    let r = zenus_arch::random::get_random_u64();
    let lo: u32;
    let hi: u32;
    unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack)); }
    let rdtsc = (lo as u64) | ((hi as u64) << 32);
    let counter = zenus_arch::interrupts::pit::get_ticks();
    let mixed = r.wrapping_mul(6364136223846793005)
        .wrapping_add(rdtsc)
        .wrapping_add(counter.wrapping_mul(123456789));
    (mixed ^ (mixed >> 16)) as u32
}

pub fn listen(port: u16) -> Option<usize> {
    let mut state = TCP_STATE.lock();
    if conn_by_port(port, &state).is_some() {
        return None;
    }
    let idx = find_slot(&state)?;
    state.conns[idx] = Some(Tcb {
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
            cwnd: INIT_CWND,
            ssthresh: INIT_SSTHRESH,
            dupack_count: 0,
            last_ack_seq: 0,
            keepalive_probes: 0,
            keepalive_time: 0,
            sack_blocks: [(0, 0); 4],
        });
        Some(idx)
}

pub fn connect(iface_idx: usize, local_port: u16, dst_ip: [u8; 4], dst_port: u16) -> Option<usize> {
    let mut state = TCP_STATE.lock();
    let idx = find_slot(&state)?;
    let local_ip = crate::nic::get_iface(iface_idx)
        .map(|iface| iface.ip)
        .unwrap_or([0; 4]);
    if local_ip == [0; 4] || local_ip == [127, 0, 0, 1] {
        return None;
    }
    let isn = rand_isn();

    let mut seg = build_syn_segment(
        local_port, dst_port,
        isn, 0,
        TCP_FLAG_SYN,
        65535,
    );
    let csum = checksum(local_ip, dst_ip, &seg[..32]);
    seg[16] = (csum >> 8) as u8;
    seg[17] = (csum & 0xFF) as u8;
    let sent = ipv4::send(iface_idx, dst_ip, ipv4::PROTO_TCP, &seg[..32]);
    if !sent {
        return None;
    }

    state.conns[idx] = Some(Tcb {
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
            cwnd: INIT_CWND,
            ssthresh: INIT_SSTHRESH,
            dupack_count: 0,
            last_ack_seq: 0,
            keepalive_probes: 0,
            keepalive_time: 0,
            sack_blocks: [(0, 0); 4],
        });

        zenus_console::kdebug!("TCP connect sending SYN from {}->{} isn={}", local_port, dst_port, isn);

        Some(idx)
}

pub fn handle_receive(
    iface_idx: usize,
    src_ip: [u8; 4], dst_ip: [u8; 4],
    segment: &[u8],
) -> bool {
    let mut state = TCP_STATE.lock();
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
        zenus_console::kdebug!("TCP-IN {}.{}.{}.{}:{}->{}.{}.{}.{}:{} flg=0x{:x} seq={} ack={} plen={}",
            src_ip[0], src_ip[1], src_ip[2], src_ip[3], src_port,
            dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_port,
            flags, seq, ack, payload.len());
    }

    let conn_idx = {
        let mut found = None;
        for i in 0..MAX_CONNS {
            match &state.conns[i] {
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

    let tcb = match &mut state.conns[conn_idx] {
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
                let child = match find_slot(&state) {
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
                        state.conns[child] = Some(Tcb {
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
                            cwnd: INIT_CWND,
                            ssthresh: INIT_SSTHRESH,
                            dupack_count: 0,
                            last_ack_seq: 0,
                            keepalive_probes: 0,
                            keepalive_time: 0,
                            sack_blocks: [(0, 0); 4],
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

                    zenus_console::kdebug!("TCP connection ESTABLISHED");
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

                        zenus_console::kdebug!("TCP SYN_RCVD->ESTABLISHED");

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

    true
}

pub fn poll_retransmit(iface_idx: usize) {
    let mut state = TCP_STATE.lock();
    for i in 0..MAX_CONNS {
        let tcb = match &mut state.conns[i] {
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
                zenus_console::kwarn!("TCP retry exhausted, closing conn {}", i);
                tcb.state = TCP_CLOSED;
            }
        }
}

pub fn send_data(conn: usize, data: &[u8]) -> bool {
    if conn >= MAX_CONNS { return false; }
    let mut state = TCP_STATE.lock();
    let tcb = match &mut state.conns[conn] {
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

pub fn flush_tx(conn: usize, iface_idx: usize) -> bool {
    if conn >= MAX_CONNS { return false; }
    let mut state = TCP_STATE.lock();
    let tcb = match &mut state.conns[conn] {
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

pub fn receive_data(conn: usize, buf: &mut [u8]) -> Option<usize> {
    if conn >= MAX_CONNS { return None; }
    let mut state = TCP_STATE.lock();
    let tcb = match &mut state.conns[conn] {
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

pub fn close(conn: usize, iface_idx: usize) -> bool {
    if conn >= MAX_CONNS { return false; }
    let mut state = TCP_STATE.lock();
    let tcb = match &mut state.conns[conn] {
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

pub fn close_conn(conn: usize) {
    if conn >= MAX_CONNS { return; }
    let mut state = TCP_STATE.lock();
    state.conns[conn] = None;
}

pub fn is_connected(conn: usize) -> bool {
    if conn >= MAX_CONNS { return false; }
    let state = TCP_STATE.lock();
    match &state.conns[conn] {
        Some(t) => t.state == TCP_ESTABLISHED || t.state == TCP_CLOSE_WAIT,
        None => false,
    }
}

pub fn has_data(conn: usize) -> bool {
    if conn >= MAX_CONNS { return false; }
    let state = TCP_STATE.lock();
    match &state.conns[conn] {
        Some(t) => t.rx_data_len > 0,
        None => false,
    }
}

pub fn get_conn_info(conn: usize) -> Option<(u8, [u8; 4], u16, [u8; 4], u16)> {
    if conn >= MAX_CONNS { return None; }
    let state = TCP_STATE.lock();
    match &state.conns[conn] {
        Some(t) => Some((t.state, t.remote_ip, t.remote_port, t.local_ip, t.local_port)),
        None => None,
    }
}

pub fn state_name(conn: usize) -> &'static str {
    if conn >= MAX_CONNS { return "NONE"; }
    let state = TCP_STATE.lock();
    match &state.conns[conn] {
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

#[derive(Clone, Copy)]
pub struct TcpConnInfo {
    pub active: bool,
    pub local_port: u16,
    pub local_ip: [u8; 4],
    pub remote_port: u16,
    pub remote_ip: [u8; 4],
    pub state: u8,
}

impl TcpConnInfo {
    pub fn state_str(&self) -> &'static str {
        match self.state {
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
        }
    }
}

pub fn tcp_status() -> [TcpConnInfo; MAX_CONNS] {
    let state = TCP_STATE.lock();
    let mut status = [TcpConnInfo {
        active: false,
        local_port: 0,
        local_ip: [0; 4],
        remote_port: 0,
        remote_ip: [0; 4],
        state: 0,
    }; MAX_CONNS];
    for i in 0..MAX_CONNS {
        if let Some(t) = &state.conns[i] {
            status[i] = TcpConnInfo {
                active: true,
                local_port: t.local_port,
                local_ip: t.local_ip,
                remote_port: t.remote_port,
                remote_ip: t.remote_ip,
                state: t.state,
            };
        }
    }
    status
}

pub fn connection_count() -> usize {
    let state = TCP_STATE.lock();
    let mut count = 0;
    for i in 0..MAX_CONNS {
        if state.conns[i].is_some() {
            count += 1;
        }
    }
    count
}
