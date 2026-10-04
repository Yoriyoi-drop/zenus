use alloc::vec::Vec;

use crate::coverage;
use crate::FuzzResult;

/// Device types for fuzzing
#[derive(Clone, Copy)]
enum DeviceType {
    Keyboard,
    Disk,
    Timer,
    Pci,
    Serial,
}

/// Fuzzing input format for devices:
/// [device_type: u8] [operation: u8] [data: bytes]
pub fn execute(input: &[u8]) -> FuzzResult {
    if input.len() < 2 {
        return FuzzResult::Normal;
    }

    let device = match input[0] % 5 {
        0 => DeviceType::Keyboard,
        1 => DeviceType::Disk,
        2 => DeviceType::Timer,
        3 => DeviceType::Pci,
        _ => DeviceType::Serial,
    };

    let operation = input[1];
    let data = if input.len() > 2 {
        &input[2..]
    } else {
        &[]
    };

    coverage::record_edge((device as u8 as u64).wrapping_mul(100).wrapping_add(operation as u64));

    match device {
        DeviceType::Keyboard => fuzz_keyboard(operation, data),
        DeviceType::Disk => fuzz_disk(operation, data),
        DeviceType::Timer => fuzz_timer(operation, data),
        DeviceType::Pci => fuzz_pci(operation, data),
        DeviceType::Serial => fuzz_serial(operation, data),
    }
}

fn fuzz_keyboard(operation: u8, data: &[u8]) -> FuzzResult {
    match operation % 6 {
        0 => {
            // Normal key press
            if data.is_empty() {
                return FuzzResult::Normal;
            }
            let scancode = data[0];
            coverage::record_edge(scancode as u64);
        }
        1 => {
            // Key release
        }
        2 => {
            // Extended key (E0 prefix)
            if data.len() < 2 {
                return FuzzResult::Normal;
            }
        }
        3 => {
            // Rapid sequence
            for &byte in data {
                coverage::record_edge(byte as u64);
            }
        }
        4 => {
            // Buffer overflow test
            if data.len() > 256 {
                return FuzzResult::Crash;
            }
        }
        _ => {
            // Invalid scancode
            if data.is_empty() {
                return FuzzResult::Normal;
            }
            let scancode = data[0];
            if scancode > 0x83 {
                // Invalid PS/2 scancode
                return FuzzResult::Normal;
            }
        }
    }

    FuzzResult::Normal
}

fn fuzz_disk(operation: u8, data: &[u8]) -> FuzzResult {
    match operation % 5 {
        0 => {
            // Read sector
            if data.len() < 8 {
                return FuzzResult::Normal;
            }
            let lba = u64::from_le_bytes([data[0], data[1], data[2], data[3], 0, 0, 0, 0]);
            if lba > 0x100000 {
                return FuzzResult::Normal;
            }
        }
        1 => {
            // Write sector
        }
        2 => {
            // DMA transfer
            if data.len() < 16 {
                return FuzzResult::Normal;
            }
        }
        3 => {
            // Invalid sector
        }
        _ => {
            // Reset
        }
    }

    FuzzResult::Normal
}

fn fuzz_timer(operation: u8, _data: &[u8]) -> FuzzResult {
    match operation % 4 {
        0 => {
            // Set frequency
        }
        1 => {
            // Get ticks
        }
        2 => {
            // Calibrate
        }
        _ => {
            // Invalid frequency
        }
    }

    FuzzResult::Normal
}

fn fuzz_pci(operation: u8, data: &[u8]) -> FuzzResult {
    match operation % 4 {
        0 => {
            // Config read
            if data.len() < 4 {
                return FuzzResult::Normal;
            }
            let bus = data[0] & 0xFF;
            // Masking first is what a PCI config-cycle accessor does, so the
            // original code's `dev > 0x1F || func > 0x07` could never be true —
            // the "validation" was dead code. Validate the raw bytes instead,
            // then mask, which is what actually reaches the hardware.
            let raw_dev = data[1];
            let raw_func = data[2];
            if raw_dev > 0x1F || raw_func > 0x07 {
                return FuzzResult::Normal;
            }
            let dev = raw_dev;
            let func = raw_func;
            coverage::record_edge(((bus as u64) << 8) | ((dev as u64) << 3) | func as u64);
        }
        1 => {
            // Config write
        }
        2 => {
            // Enumerate
        }
        _ => {
            // Invalid device
        }
    }

    FuzzResult::Normal
}

fn fuzz_serial(operation: u8, data: &[u8]) -> FuzzResult {
    match operation % 4 {
        0 => {
            // Write byte
            if data.is_empty() {
                return FuzzResult::Normal;
            }
        }
        1 => {
            // Read byte
        }
        2 => {
            // Set baud rate
        }
        _ => {
            // Invalid config
        }
    }

    FuzzResult::Normal
}

/// Device-fuzzing seeds: every device type crossed with the mutation
/// strategies the doc calls out (normal key, release, extended key, invalid
/// scancode, rapid sequence, buffer full/overflow).
pub fn generate_seeds() -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();
    for device in 0..5u8 {
        for op in 0..6u8 {
            let mut seed = Vec::with_capacity(66);
            seed.push(device);
            seed.push(op);
            // scancode payload: valid, boundary (0x83), and above range
            for b in [0x1Eu8, 0x83, 0xFF, 0x00] {
                seed.push(b);
            }
            // rapid-sequence payload
            for i in 0..32u8 {
                seed.push(i.wrapping_mul(7));
            }
            seeds.push(seed);
        }
    }
    // Buffer-overflow probe: a payload longer than any ring buffer.
    let mut big = Vec::with_capacity(300);
    big.push(0);
    big.push(4);
    big.resize(300, 0xAB);
    seeds.push(big);
    seeds
}
