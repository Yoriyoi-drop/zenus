ARCH ?= x86_64
TARGET = $(ARCH)-unknown-none
CARGO := cargo
CARGO_FLAGS ?=
SMP ?= 4
BUILD_DIR := build
PROFILE_DIR := $(if $(filter --release,$(CARGO_FLAGS)),release,debug)
LIMINE_DIR := limine
ISO_DIR := iso_root

KERNEL := $(BUILD_DIR)/zenus
INITRD := initrd.tar
ISO := $(BUILD_DIR)/zenus.iso
IMG := $(BUILD_DIR)/zenus.hdd
LD := ld.lld

.PHONY: all clean run run-serial run-gui run-bios run-uefi run-gdb iso img test test-quiet bochs
.PHONY: test-host test-host-quiet
.PHONY: build-fuzz fuzz-smoke fuzz-coverage fuzz-regression fuzz-clean

all: $(KERNEL)

# Build kernel staticlib
target/$(TARGET)/$(PROFILE_DIR)/libzenus.a: apps/src/lib.rs $(shell find crates -name '*.rs')
	$(CARGO) build --package zenus --target $(TARGET) $(CARGO_FLAGS)

# Link kernel with custom linker script
$(KERNEL): target/$(TARGET)/$(PROFILE_DIR)/libzenus.a apps/src/linker.ld
	mkdir -p $(BUILD_DIR)
	$(LD) -T apps/src/linker.ld -o $@ \
		--nmagic -n --gc-sections \
		--whole-archive \
		target/$(TARGET)/$(PROFILE_DIR)/libzenus.a \
		--no-whole-archive

# Build userspace programs
USERSPACE_PROGS := hello echo cat exitonly args minimal
USERSPACE_BUILD := userspace/build
$(USERSPACE_BUILD)/%: userspace/%/src/lib.rs userspace/userspace.ld
	$(MAKE) -C userspace $(notdir $@)

# Build initrd
$(INITRD): mkinitrd.sh $(addprefix $(USERSPACE_BUILD)/,$(USERSPACE_PROGS))
	bash mkinitrd.sh $(INITRD)

# ISO image (BIOS + UEFI) — ISO depends on kernel + initrd
$(ISO): $(KERNEL) $(INITRD)
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot/limine
	$(LD) --strip-debug --gc-sections -T apps/src/linker.ld -o $(ISO_DIR)/boot/zenus \
		--nmagic -n \
		--whole-archive \
		target/$(TARGET)/$(PROFILE_DIR)/libzenus.a \
		--no-whole-archive
	cp $(INITRD) $(ISO_DIR)/boot/
	cp limine.conf $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios.sys $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/limine/
	mkdir -p $(ISO_DIR)/EFI/BOOT
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	cp $(LIMINE_DIR)/BOOTIA32.EFI $(ISO_DIR)/EFI/BOOT/
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		$(ISO_DIR) -o $(ISO)
	$(LIMINE_DIR)/limine bios-install $(ISO)

iso: $(ISO)

# HDD image (UEFI)
img: $(KERNEL) $(INITRD)
	dd if=/dev/zero bs=1M count=0 seek=64 of=$(IMG)
	parted -s $(IMG) mklabel gpt
	parted -s $(IMG) mkpart ESP fat32 2048s 100%
	parted -s $(IMG) set 1 esp on
	$(eval LOOP := $(shell losetup -Pf --show $(IMG)))
	mkfs.fat -F 32 $(LOOP)p1
	mount $(LOOP)p1 /mnt
	mkdir -p /mnt/EFI/BOOT /mnt/boot/limine
	cp $(KERNEL) /mnt/boot/
	cp $(INITRD) /mnt/boot/
	cp limine.conf /mnt/boot/limine/
	cp $(LIMINE_DIR)/BOOTX64.EFI /mnt/EFI/BOOT/
	cp $(LIMINE_DIR)/limine-bios.sys /mnt/boot/limine/
	umount /mnt
	losetup -d $(LOOP)
	$(LIMINE_DIR)/limine bios-install $(IMG)

# ── Interactive QEMU targets ──
# run-serial   : serial-only (piped input via ssh), no display window
# run-gui      : QEMU window + serial on stdout (keyboard+mouse input)
# run-bios     : legacy -nographic mode
# run-uefi     : UEFI firmware variant
# run-gdb      : with GDB stub

run: run-gui

run-serial: $(ISO)
	qemu-system-x86_64 -display none -serial stdio -m 2G -smp $(SMP) -cdrom $(ISO) -no-reboot \
		-cpu max -netdev user,id=net0 -device rtl8139,netdev=net0

run-gui: $(ISO)
	qemu-system-x86_64 -m 2G -smp $(SMP) -cdrom $(ISO) -no-reboot \
		-cpu max -netdev user,id=net0 -device rtl8139,netdev=net0

run-bios: $(ISO)
	qemu-system-x86_64 -nographic -m 2G -smp $(SMP) -cdrom $(ISO) -no-reboot \
		-cpu max -netdev user,id=net0 -device rtl8139,netdev=net0

run-uefi: $(ISO)
	qemu-system-x86_64 -serial mon:stdio -m 2G -smp $(SMP) -bios /usr/share/ovmf/OVMF.fd -cdrom $(ISO) -no-reboot \
		-cpu max -netdev user,id=net0 -device rtl8139,netdev=net0

run-gdb: $(ISO)
	qemu-system-x86_64 -m 2G -smp $(SMP) -cdrom $(ISO) -s -S -no-reboot \
		-cpu max -netdev user,id=net0 -device rtl8139,netdev=net0

run-qemu: run-gui

# TCP serial: connect with: nc localhost 45678
# Piped stdin (-nographic) does not work reliably with KVM in-kernel PIT
# mode (the default). KVM's in-kernel PIT processes timer interrupts
# without returning to QEMU's main loop, so stdin pipe data is never
# forwarded to the UART. TCP serial avoids this by using a socket
# backend that QEMU processes through its own fd event loop.
# Use `make run-tcp` then `nc localhost 45678` for interactive shell.
run-tcp: $(ISO)
	qemu-system-x86_64 -enable-kvm -cpu max -m 2G -smp $(SMP) -cdrom $(ISO) -no-reboot \
		-serial tcp:localhost:45678,server,nowait \
		-netdev user,id=net0 -device rtl8139,netdev=net0 &
	@sleep 1
	@echo "Connect: nc localhost 45678"
	@echo "Press Ctrl+A X to exit QEMU"
	@sleep 2
	nc localhost 45678

bochs: $(ISO)
	bochs -f bochsrc -q

# Test build — enables testing feature for unit tests
$(BUILD_DIR)/zenus-test: apps/src/lib.rs $(shell find crates -name '*.rs') apps/src/linker.ld
	$(CARGO) build --package zenus --target $(TARGET) --features testing
	mkdir -p $(BUILD_DIR)
	$(LD) -T apps/src/linker.ld -o $@ \
		--nmagic -n --gc-sections \
		--whole-archive \
		target/$(TARGET)/debug/libzenus.a \
		--no-whole-archive

test-iso: $(BUILD_DIR)/zenus-test $(INITRD)
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot/limine
	cp $(BUILD_DIR)/zenus-test $(ISO_DIR)/boot/zenus
	cp $(INITRD) $(ISO_DIR)/boot/
	cp limine.conf $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios.sys $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/limine/
	mkdir -p $(ISO_DIR)/EFI/BOOT
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	cp $(LIMINE_DIR)/BOOTIA32.EFI $(ISO_DIR)/EFI/BOOT/
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		$(ISO_DIR) -o $(BUILD_DIR)/zenus-test.iso
	$(LIMINE_DIR)/limine bios-install $(BUILD_DIR)/zenus-test.iso

# ── Host unit tests ───────────────────────────────────────────────────────
#
# The `#[cfg(test)]` modules inside the kernel crates are host-side: they cover
# the pure logic (VMA arithmetic, syscall tables, packet parsing, permission
# bits, …) without booting anything. They need a host triple, which is why
# `.cargo/config.toml` no longer sets a default target — the bare-metal
# triple has no `std`, so `cargo test` there dies with "can't find crate for
# `test`" plus a missing `#[panic_handler]`.
#
# The in-kernel suite (`make test`, below) is separate and stays: it is the only
# way to exercise real MMIO, the APIC and the IDT.
HOST_TARGET := x86_64-unknown-linux-gnu

test-host:
	$(CARGO) test --workspace --target $(HOST_TARGET)

test-host-quiet:
	$(CARGO) test --workspace --target $(HOST_TARGET) --quiet

test: test-iso
	qemu-system-x86_64 -serial mon:stdio -m 2G -smp $(SMP) -cdrom $(BUILD_DIR)/zenus-test.iso -no-reboot \
		-drive file=ext2_test.img,format=raw,if=ide 2>&1

test-quiet: test-iso
	qemu-system-x86_64 -serial mon:stdio -m 2G -smp $(SMP) -cdrom $(BUILD_DIR)/zenus-test.iso -no-reboot \
		-drive file=ext2_test.img,format=raw,if=ide 2>&1 | grep -a "\[TEST\]"

clean:
	rm -rf $(BUILD_DIR) $(ISO_DIR)
	rm -f initrd.tar
	$(CARGO) clean

# ── Fuzzing (see doc/fuzzing.md) ──
#
# A fuzzing build replaces the shell with an in-kernel campaign: the fuzzer
# executes one input at a time inside a fault-containment checkpoint
# (zenus_arch::fuzz_guard), records every distinct crash, prints a machine
# readable report and powers the VM off. The kernel signals its result with
#
#     [FUZZ] EXIT code=<0|1|2>
#
#   0 = campaign completed, no crash
#   1 = at least one crash found
#   2 = campaign aborted (a fault containment could not unwind)
#
# Single-core on purpose: the checkpoint is a single (rsp, rip) pair, so only
# the CPU that armed it may recover (zenus_arch::fuzz_guard::should_recover).

FUZZ_BUILD_DIR := $(BUILD_DIR)/fuzz
FUZZ_KERNEL := $(FUZZ_BUILD_DIR)/zenus-fuzz
FUZZ_ISO := $(FUZZ_BUILD_DIR)/zenus-fuzz.iso
FUZZ_LOG := $(FUZZ_BUILD_DIR)/fuzz.log
FUZZ_SMP ?= 1
FUZZ_MEM ?= 2G
# Hard wall-clock cap per campaign so a wedged fuzz case cannot hang the target.
FUZZ_TIMEOUT ?= 300

# $(1) cargo feature selecting the campaign mode
define fuzz_build
	@mkdir -p $(FUZZ_BUILD_DIR)
	$(CARGO) build --package zenus --target $(TARGET) --no-default-features \
		--features $(1) $(CARGO_FLAGS)
	$(LD) -T apps/src/linker.ld -o $(FUZZ_KERNEL) \
		--nmagic -n --gc-sections \
		--whole-archive \
		target/$(TARGET)/$(PROFILE_DIR)/libzenus.a \
		--no-whole-archive
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot/limine
	cp $(FUZZ_KERNEL) $(ISO_DIR)/boot/zenus
	cp $(INITRD) $(ISO_DIR)/boot/
	cp limine.conf $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios.sys $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(ISO_DIR)/boot/limine/
	cp $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/limine/
	mkdir -p $(ISO_DIR)/EFI/BOOT
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	cp $(LIMINE_DIR)/BOOTIA32.EFI $(ISO_DIR)/EFI/BOOT/
	xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		$(ISO_DIR) -o $(FUZZ_ISO)
	$(LIMINE_DIR)/limine bios-install $(FUZZ_ISO)
endef

# Run a campaign and store its serial output in $(FUZZ_LOG).
# $(1) make target to build (recursive call), $(2) human label
define fuzz_run
	$(MAKE) $(1)
	@echo "=== running fuzzing campaign ($(2)) ==="
	@rm -f $(FUZZ_LOG)
	@timeout $(FUZZ_TIMEOUT) qemu-system-x86_64 \
		-display none -serial stdio -no-reboot -no-shutdown \
		-m $(FUZZ_MEM) -smp $(FUZZ_SMP) -cdrom $(FUZZ_ISO) \
		-cpu max 2>&1 | tee $(FUZZ_LOG) | grep -a '\[FUZZ\]' || true
	@echo "--- crash list ---"
	@grep -a '\[FUZZ\] CRASH' $(FUZZ_LOG) || echo "(no crash reported)"
	@echo "--- summary ---"
	@grep -a '\[FUZZ\] SUMMARY' $(FUZZ_LOG) || echo "(campaign produced no summary)"
	@grep -a '\[FUZZ\] EXIT' $(FUZZ_LOG) || echo "(campaign did not finish: timeout or reset)"
endef

## Build the kernel with the fuzzing framework linked in (smoke mode).
build-fuzz:
	$(call fuzz_build,fuzz-smoke)
	@echo "built $(FUZZ_KERNEL) / $(FUZZ_ISO)"

## Smoke mode: 100 - 10.000 cases, run on every commit.
fuzz-smoke:
	$(call fuzz_run,build-fuzz,smoke)

## Coverage mode: 10^5 cases, hunting for new paths.
fuzz-coverage:
	$(call fuzz_run,fuzz-coverage-build,coverage)

## Regression mode: replay the recorded crashes.
fuzz-regression:
	$(call fuzz_run,fuzz-regression-build,regression)

fuzz-coverage-build:
	$(call fuzz_build,fuzz-coverage)

fuzz-regression-build:
	$(call fuzz_build,fuzz-regression)

## Remove fuzzing artifacts and the recorded crash log.
fuzz-clean:
	rm -rf $(FUZZ_BUILD_DIR)
	$(RM) $(FUZZ_LOG)
