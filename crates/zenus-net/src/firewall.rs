use zenus_sync::spinlock::SpinLock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FirewallAction {
    Accept,
    Drop,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FirewallProto {
    Any,
    Tcp,
    Udp,
    Icmp,
}

pub const RULE_NAME_LEN: usize = 32;
pub const MAX_RULES: usize = 32;
pub const MAX_CONNTRACK: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct FirewallRule {
    pub name: [u8; RULE_NAME_LEN],
    pub enabled: bool,
    pub action: FirewallAction,
    pub proto: FirewallProto,
    pub src_ip: [u8; 4],
    pub src_mask: [u8; 4],
    pub dst_ip: [u8; 4],
    pub dst_mask: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub established: bool,
    pub packets_matched: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnState {
    New,
    Established,
    Related,
}

#[derive(Debug, Clone, Copy)]
pub struct ConnTrack {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: FirewallProto,
    pub state: ConnState,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PacketInfo {
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: FirewallProto,
}

struct FirewallState {
    rules: [Option<FirewallRule>; MAX_RULES],
    conns: [Option<ConnTrack>; MAX_CONNTRACK],
}

static FIREWALL: SpinLock<FirewallState> = SpinLock::new(FirewallState {
    rules: [None; MAX_RULES],
    conns: [None; MAX_CONNTRACK],
});

pub fn firewall_init() {
    let mut fw = FIREWALL.lock();
    fw.rules = [None; MAX_RULES];
    fw.conns = [None; MAX_CONNTRACK];
}

pub fn firewall_add_rule(rule: FirewallRule) -> bool {
    let mut fw = FIREWALL.lock();
    for i in 0..MAX_RULES {
        if fw.rules[i].is_none() {
            fw.rules[i] = Some(rule);
            return true;
        }
    }
    false
}

pub fn firewall_remove_rule(index: usize) -> bool {
    if index >= MAX_RULES {
        return false;
    }
    let mut fw = FIREWALL.lock();
    if fw.rules[index].is_some() {
        fw.rules[index] = None;
        true
    } else {
        false
    }
}

pub fn firewall_list_rules() -> [Option<FirewallRule>; MAX_RULES] {
    let fw = FIREWALL.lock();
    fw.rules
}

pub fn firewall_check(pkt: &PacketInfo) -> FirewallAction {
    let mut fw = FIREWALL.lock();

    let mut found_established = false;
    for i in 0..MAX_CONNTRACK {
        if let Some(ref conn) = fw.conns[i] {
            if conn.src_ip == pkt.src_ip
                && conn.dst_ip == pkt.dst_ip
                && conn.src_port == pkt.src_port
                && conn.dst_port == pkt.dst_port
                && conn.proto == pkt.proto
            {
                if conn.state == ConnState::Established || conn.state == ConnState::Related {
                    found_established = true;
                }
                break;
            }
        }
    }

    for i in 0..MAX_RULES {
        if let Some(ref mut rule) = fw.rules[i] {
            if !rule.enabled {
                continue;
            }

            if rule.proto != FirewallProto::Any && rule.proto != pkt.proto {
                continue;
            }

            let mut match_src = true;
            for j in 0..4 {
                if (pkt.src_ip[j] & rule.src_mask[j]) != (rule.src_ip[j] & rule.src_mask[j]) {
                    match_src = false;
                    break;
                }
            }
            if !match_src {
                continue;
            }

            let mut match_dst = true;
            for j in 0..4 {
                if (pkt.dst_ip[j] & rule.dst_mask[j]) != (rule.dst_ip[j] & rule.dst_mask[j]) {
                    match_dst = false;
                    break;
                }
            }
            if !match_dst {
                continue;
            }

            if pkt.proto == FirewallProto::Tcp || pkt.proto == FirewallProto::Udp {
                if rule.src_port != 0 && rule.src_port != pkt.src_port {
                    continue;
                }
                if rule.dst_port != 0 && rule.dst_port != pkt.dst_port {
                    continue;
                }
            }

            if rule.established && !found_established {
                continue;
            }

            rule.packets_matched = rule.packets_matched.wrapping_add(1);
            return rule.action;
        }
    }

    FirewallAction::Accept
}

pub fn firewall_track_connection(conn: ConnTrack) {
    let mut fw = FIREWALL.lock();

    for i in 0..MAX_CONNTRACK {
        if let Some(ref mut c) = fw.conns[i] {
            if c.src_ip == conn.src_ip
                && c.dst_ip == conn.dst_ip
                && c.src_port == conn.src_port
                && c.dst_port == conn.dst_port
                && c.proto == conn.proto
            {
                c.state = conn.state;
                c.last_seen = conn.last_seen;
                return;
            }
        }
    }

    for i in 0..MAX_CONNTRACK {
        if fw.conns[i].is_none() {
            fw.conns[i] = Some(conn);
            return;
        }
    }
}

pub fn firewall_clear_connections() {
    let mut fw = FIREWALL.lock();
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    for i in 0..MAX_CONNTRACK {
        if let Some(ref conn) = fw.conns[i] {
            if ticks.wrapping_sub(conn.last_seen) > 300 {
                fw.conns[i] = None;
            }
        }
    }
}

pub fn firewall_rule_count() -> usize {
    let fw = FIREWALL.lock();
    let mut count = 0;
    for i in 0..MAX_RULES {
        if fw.rules[i].is_some() {
            count += 1;
        }
    }
    count
}

pub fn firewall_conn_count() -> usize {
    let fw = FIREWALL.lock();
    let mut count = 0;
    for i in 0..MAX_CONNTRACK {
        if fw.conns[i].is_some() {
            count += 1;
        }
    }
    count
}
