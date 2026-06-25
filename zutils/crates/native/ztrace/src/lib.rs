#![no_std]

use zutils_common::{Args, Writer};

static mut TRACE_ENABLED: bool = false;
static mut TRACED_PID: u64 = 0;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let subcmd = args.get(1).unwrap_or("help");
    match subcmd {
        "on" | "start" => {
            unsafe { TRACE_ENABLED = true; }
            w.write_str("Syscall tracing enabled\r\n");
        }
        "off" | "stop" => {
            unsafe { TRACE_ENABLED = false; }
            w.write_str("Syscall tracing disabled\r\n");
        }
        "status" => {
            unsafe {
                if TRACE_ENABLED {
                    w.write_str("Syscall tracing: ON");
                    if TRACED_PID > 0 {
                        w.write_str(" (PID ");
                        w.write_u64(TRACED_PID);
                        w.write_str(" only)");
                    }
                    w.write_str("\r\n");
                } else {
                    w.write_str("Syscall tracing: OFF\r\n");
                }
            }
        }
        "pid" => {
            let pid = match args.get(2).and_then(|a| a.parse::<u64>().ok()) {
                Some(p) => p,
                None => { w.write_str("Usage: ztrace pid <PID>\r\n"); return; }
            };
            unsafe { TRACED_PID = pid; }
            w.write_str("Tracing PID: ");
            w.write_u64(pid);
            w.write_str("\r\n");
        }
        _ => {
            w.write_str("Zenus Syscall Tracer\r\n");
            w.write_str("Usage: ztrace <on|off|status|pid <PID>>\r\n");
            w.write_str("Traces syscalls made by processes\r\n");
        }
    }
}
