use crate::fb;
use crate::vga;

/// Write a string to the active display (framebuffer or VGA text mode).
/// Framebuffer takes priority if initialized; falls back to VGA.
pub fn write_str(s: &str) {
    if fb::is_initialized() {
        fb::write_str(s);
    } else {
        vga::write_str(s);
    }
}

/// Clear the active display.
pub fn clear() {
    if fb::is_initialized() {
        fb::clear();
    } else {
        vga::clear();
    }
}
