pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::checksum;

    pub fn test_checksum_zero() -> Result<(), &'static str> {
        let data = [0u8; 20];
        let c = checksum(&data);
        if c != 0xFFFF {
            return Err("Checksum of all zeros should be 0xFFFF");
        }
        Ok(())
    }

    pub fn test_checksum_simple() -> Result<(), &'static str> {
        let data = [0x45, 0x00, 0x00, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x40, 0x06, 0x00, 0x00, 0xC0, 0xA8, 0x01, 0x01, 0xC0, 0xA8, 0x01, 0x02];
        let c = checksum(&data);
        if c == 0 {
            return Err("Non-trivial checksum should not be zero");
        }
        Ok(())
    }
}
