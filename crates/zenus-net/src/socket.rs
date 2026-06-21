use crate::tcp;

pub const AF_INET: u8 = 2;
pub const SOCK_STREAM: u8 = 1;
pub const SOCK_DGRAM: u8 = 2;
pub const SOL_SOCKET: u8 = 1;

const MAX_SOCKETS: usize = 32;
const MAX_UDP_RECV: usize = 8;

#[derive(Clone, Copy, PartialEq)]
enum SocketState {
    Free,
    Created,
    Bound,
    Listening,
    Connected,
    Closing,
}

#[derive(Clone, Copy)]
struct UdpBuffer {
    data: [u8; 1500],
    len: usize,
}

#[derive(Clone, Copy)]
struct UdpSocket {
    local_port: u16,
    local_ip: [u8; 4],
    dst_ip: [u8; 4],
    dst_port: u16,
    connected: bool,
    recv_buf: [UdpBuffer; MAX_UDP_RECV],
    recv_count: usize,
    recv_head: usize,
}

#[derive(Clone, Copy)]
enum SocketKind {
    Tcp { conn: usize },
    Udp(UdpSocket),
}

#[derive(Clone, Copy)]
struct Socket {
    state: SocketState,
    domain: u8,
    kind: SocketKind,
}

pub struct SocketPool {
    sockets: [Socket; MAX_SOCKETS],
}

static mut SOCKET_POOL: SocketPool = SocketPool {
    sockets: [Socket {
        state: SocketState::Free,
        domain: 0,
        kind: SocketKind::Tcp { conn: 0 },
    }; MAX_SOCKETS],
};

fn alloc_sock() -> Option<usize> {
    unsafe {
        for i in 0..MAX_SOCKETS {
            if SOCKET_POOL.sockets[i].state == SocketState::Free {
                SOCKET_POOL.sockets[i].state = SocketState::Created;
                return Some(i);
            }
        }
    }
    None
}

fn state(sock: usize) -> SocketState {
    unsafe {
        if sock >= MAX_SOCKETS {
            return SocketState::Free;
        }
        SOCKET_POOL.sockets[sock].state
    }
}

fn find_tcp_listener_port(fd: usize) -> Option<u16> {
    unsafe {
        let s = &SOCKET_POOL.sockets[fd];
        if s.state != SocketState::Listening {
            return None;
        }
        match s.kind {
            SocketKind::Tcp { conn } => {
                if let Some((_, _, _, _, local_port)) = tcp::get_conn_info(conn) {
                    return Some(local_port);
                }
                None
            }
            _ => None,
        }
    }
}

pub fn socket(domain: u8, type_: u8, _protocol: u8) -> Option<usize> {
    if domain != AF_INET {
        return None;
    }
    let fd = alloc_sock()?;
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        s.domain = domain;
        s.kind = match type_ {
            SOCK_STREAM => SocketKind::Tcp { conn: 0 },
            SOCK_DGRAM => SocketKind::Udp(UdpSocket {
                local_port: 0,
                local_ip: [0; 4],
                dst_ip: [0; 4],
                dst_port: 0,
                connected: false,
                recv_buf: [UdpBuffer { data: [0; 1500], len: 0 }; MAX_UDP_RECV],
                recv_count: 0,
                recv_head: 0,
            }),
            _ => {
                s.state = SocketState::Free;
                return None;
            }
        };
    }
    Some(fd)
}

pub fn bind(fd: usize, port: u16) -> bool {
    if state(fd) != SocketState::Created {
        return false;
    }
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { ref mut conn } => {
                match tcp::listen(port) {
                    Some(c) => {
                        *conn = c;
                        s.state = SocketState::Bound;
                        true
                    }
                    None => false,
                }
            }
            SocketKind::Udp(ref mut us) => {
                us.local_port = port;
                s.state = SocketState::Bound;
                true
            }
        }
    }
}

pub fn listen(fd: usize, _backlog: usize) -> bool {
    if state(fd) != SocketState::Bound {
        return false;
    }
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { .. } => {
                s.state = SocketState::Listening;
                true
            }
            _ => false,
        }
    }
}

pub fn accept(fd: usize, iface_idx: usize) -> Option<usize> {
    if state(fd) != SocketState::Listening {
        return None;
    }
    find_tcp_listener_port(fd)?;
    unsafe {
        for i in 0..16 {
            if let Some((state, _remote_ip, _remote_port, _local_ip, _local_port)) = tcp::get_conn_info(i) {
                if state == 4 && !is_socket_for_conn(i) {
                    let new_fd = alloc_sock()?;
                    let s = &mut SOCKET_POOL.sockets[new_fd];
                    s.state = SocketState::Connected;
                    s.kind = SocketKind::Tcp { conn: i };
                    tcp::poll_retransmit(iface_idx);
                    return Some(new_fd);
                }
            }
        }
        tcp::poll_retransmit(iface_idx);
    }
    None
}

fn is_socket_for_conn(conn: usize) -> bool {
    unsafe {
        for i in 0..MAX_SOCKETS {
            let s = &SOCKET_POOL.sockets[i];
            if let SocketKind::Tcp { conn: c } = s.kind {
                if c == conn && (s.state == SocketState::Connected || s.state == SocketState::Closing) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn connect(fd: usize, iface_idx: usize, dst_ip: [u8; 4], dst_port: u16) -> bool {
    if state(fd) != SocketState::Created && state(fd) != SocketState::Bound {
        return false;
    }
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { ref mut conn } => {
                let local_port = match s.state {
                    SocketState::Bound => {
                        if let Some((_, _, _, _, p)) = tcp::get_conn_info(*conn) {
                            p
                        } else {
                            return false;
                        }
                    }
                    _ => {
                        let port = allocate_ephemeral_port();
                        *conn = match tcp::listen(port) {
                            Some(c) => c,
                            None => return false,
                        };
                        port
                    }
                };
                let new_conn = match tcp::connect(iface_idx, local_port, dst_ip, dst_port) {
                    Some(c) => c,
                    None => return false,
                };
                *conn = new_conn;
                s.state = SocketState::Connected;
                true
            }
            SocketKind::Udp(ref mut us) => {
                us.dst_ip = dst_ip;
                us.dst_port = dst_port;
                us.connected = true;
                s.state = SocketState::Connected;
                true
            }
        }
    }
}

fn allocate_ephemeral_port() -> u16 {
    static mut NEXT_EPHEMERAL: u16 = 49152;
    unsafe {
        let port = NEXT_EPHEMERAL;
        NEXT_EPHEMERAL = if NEXT_EPHEMERAL >= 65535 { 49152 } else { NEXT_EPHEMERAL + 1 };
        port
    }
}

pub fn send(fd: usize, data: &[u8], iface_idx: usize) -> bool {
    unsafe {
        let s = &SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { conn } => {
                if s.state != SocketState::Connected && s.state != SocketState::Bound {
                    return false;
                }
                if tcp::send_data(conn, data) {
                    tcp::flush_tx(conn, iface_idx)
                } else {
                    false
                }
            }
            SocketKind::Udp(us) => {
                if !us.connected {
                    return false;
                }
                let local_ip = crate::nic::get_iface(iface_idx)
                    .map(|iface| iface.ip)
                    .unwrap_or([0; 4]);
                crate::udp::send(iface_idx, us.local_port, us.dst_port, local_ip, us.dst_ip, data)
            }
        }
    }
}

pub fn sendto(fd: usize, data: &[u8], iface_idx: usize, dst_ip: [u8; 4], dst_port: u16) -> bool {
    unsafe {
        let s = &SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Udp(us) => {
                let local_ip = crate::nic::get_iface(iface_idx)
                    .map(|iface| iface.ip)
                    .unwrap_or([0; 4]);
                crate::udp::send(iface_idx, us.local_port, dst_port, local_ip, dst_ip, data)
            }
            _ => false,
        }
    }
}

pub fn recv(fd: usize, buf: &mut [u8]) -> Option<usize> {
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { conn } => {
                tcp::receive_data(conn, buf)
            }
            SocketKind::Udp(ref mut us) => {
                if us.recv_count == 0 {
                    return None;
                }
                let entry = &us.recv_buf[us.recv_head];
                let copy_len = core::cmp::min(entry.len, buf.len());
                buf[..copy_len].copy_from_slice(&entry.data[..copy_len]);
                us.recv_head = (us.recv_head + 1) % MAX_UDP_RECV;
                us.recv_count -= 1;
                Some(copy_len)
            }
        }
    }
}

pub fn close(fd: usize, iface_idx: usize) -> bool {
    unsafe {
        let s = &mut SOCKET_POOL.sockets[fd];
        let kind = s.kind;
        s.state = SocketState::Free;
        match kind {
            SocketKind::Tcp { conn } => {
                let result = tcp::close(conn, iface_idx);
                tcp::close_conn(conn);
                result
            }
            SocketKind::Udp(_) => true,
        }
    }
}

pub fn poll_all(iface_idx: usize) {
    for _ in 0..4 {
        crate::nic::net_poll();
    }
    tcp::poll_retransmit(iface_idx);
    unsafe {
        for i in 0..MAX_SOCKETS {
            let s = &SOCKET_POOL.sockets[i];
            if let SocketKind::Tcp { conn } = s.kind {
                if s.state == SocketState::Connected {
                    tcp::flush_tx(conn, iface_idx);
                }
            }
        }
    }
}

pub fn udp_enqueue(port: u16, _src_ip: [u8; 4], _src_port: u16, data: &[u8]) -> bool {
    unsafe {
        for i in 0..MAX_SOCKETS {
            let s = &mut SOCKET_POOL.sockets[i];
            if let SocketKind::Udp(ref mut us) = s.kind {
                if us.local_port == port && s.state != SocketState::Free {
                    if us.recv_count >= MAX_UDP_RECV {
                        return true;
                    }
                    let tail = (us.recv_head + us.recv_count) % MAX_UDP_RECV;
                    let copy_len = core::cmp::min(data.len(), 1500);
                    us.recv_buf[tail].data[..copy_len].copy_from_slice(&data[..copy_len]);
                    us.recv_buf[tail].len = copy_len;
                    us.recv_count += 1;
                    return true;
                }
            }
        }
    }
    false
}

pub fn is_connected(fd: usize) -> bool {
    unsafe {
        if fd >= MAX_SOCKETS {
            return false;
        }
        let s = &SOCKET_POOL.sockets[fd];
        match s.kind {
            SocketKind::Tcp { conn } => {
                crate::tcp::is_connected(conn)
            }
            _ => false,
        }
    }
}
