#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let iterations = args.get(1).and_then(|a| a.parse::<u64>().ok()).unwrap_or(10000);
    w.write_str("Zenus Benchmark\r\n");
    w.write_str("Iterations: ");
    w.write_u64(iterations);
    w.write_str("\r\n\r\n");

    let start = zenus_arch::interrupts::pit::get_ticks();
    let mut sum: u64 = 0;
    for i in 0..iterations {
        sum = sum.wrapping_add(i).wrapping_mul(7).wrapping_add(3);
    }
    let end = zenus_arch::interrupts::pit::get_ticks();
    let cpu_ticks = end.saturating_sub(start);
    w.write_str("CPU loop:   ");
    w.write_u64(cpu_ticks);
    w.write_str(" ticks (");
    w.write_u64(cpu_ticks * 10);
    w.write_str(" ms)\r\n");

    let start = zenus_arch::interrupts::pit::get_ticks();
    let mut buf = [0u8; 512];
    for i in 0..512 {
        buf[i] = (i as u8).wrapping_mul(31);
    }
    let end = zenus_arch::interrupts::pit::get_ticks();
    let mem_ticks = end.saturating_sub(start);
    w.write_str("Memory:     ");
    w.write_u64(mem_ticks);
    w.write_str(" ticks (");
    w.write_u64(mem_ticks * 10);
    w.write_str(" ms)\r\n");

    if cpu_ticks > 0 {
        w.write_str("CPI (approx): ");
        w.write_u64(sum / iterations.max(1));
        w.write_str("\r\n");
    }
}
