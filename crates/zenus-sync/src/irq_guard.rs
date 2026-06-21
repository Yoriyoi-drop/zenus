use x86_64::instructions::interrupts;

pub struct IrqGuard {
    was_enabled: bool,
}

impl IrqGuard {
    pub fn new() -> Self {
        let was_enabled = interrupts::are_enabled();
        if was_enabled {
            interrupts::disable();
        }
        IrqGuard { was_enabled }
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            interrupts::enable();
        }
    }
}
