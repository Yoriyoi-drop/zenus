#[cfg(target_os = "none")]
use x86_64::instructions::interrupts;

pub struct IrqGuard {
    was_enabled: bool,
}

impl IrqGuard {
    pub fn new() -> Self {
        // Bare metal clears IF for the guard's lifetime. The host test build
        // runs in ring 3 where `cli` faults, so the masking is compiled out
        // there and the struct degenerates into a no-op scope guard.
        #[cfg(target_os = "none")]
        let was_enabled = {
            let enabled = interrupts::are_enabled();
            if enabled {
                interrupts::disable();
            }
            enabled
        };
        #[cfg(not(target_os = "none"))]
        let was_enabled = false;

        IrqGuard { was_enabled }
    }
}

impl Default for IrqGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "none")]
        if self.was_enabled {
            interrupts::enable();
        }
        #[cfg(not(target_os = "none"))]
        let _ = self.was_enabled;
    }
}
