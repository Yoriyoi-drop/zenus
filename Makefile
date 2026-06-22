ARCH ?= x86_64
TARGET = $(ARCH)-unknown-none
CARGO := cargo
CARGO_FLAGS ?=
BUILD_DIR := build
PROFILE_DIR := $(if $(filter --release,$(CARGO_FLAGS)),release,debug)
LIMINE_DIR := limine
ISO_DIR := iso_root

KERNEL := $(BUILD_DIR)/zenus
INITRD := initrd.tar
ISO := $(BUILD_DIR)/zenus.iso
IMG := $(BUILD_DIR)/zenus.hdd
LD := ld.lld

.PHONY: all clean run run-qemu run-qemu-gdb iso img

all: $(KERNEL)

# Build user-space binary
apps/user.bin: apps/src/user.s
	nasm -f bin -o $@ $<

# Build kernel staticlib (depends on user binary)
target/$(TARGET)/$(PROFILE_DIR)/libzenus.a: apps/src/lib.rs apps/user.bin $(shell find crates -name '*.rs')
	RUSTFLAGS="-C target-cpu=x86-64" \
	$(CARGO) build --package zenus --target $(TARGET) $(CARGO_FLAGS)

# Link kernel with custom linker script
$(KERNEL): target/$(TARGET)/$(PROFILE_DIR)/libzenus.a apps/src/linker.ld
	mkdir -p $(BUILD_DIR)
	$(LD) -T apps/src/linker.ld -o $@ \
		--nmagic -n --gc-sections \
		--whole-archive \
		target/$(TARGET)/$(PROFILE_DIR)/libzenus.a \
		--no-whole-archive

# Build initrd
$(INITRD): mkinitrd.sh
	bash mkinitrd.sh $(INITRD)

# ISO image (BIOS + UEFI) — ISO depends on kernel + initrd
$(ISO): $(KERNEL) $(INITRD)
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot/limine
	cp $(KERNEL) $(ISO_DIR)/boot/
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
img: $(KERNEL)
	dd if=/dev/zero bs=1M count=0 seek=64 of=$(IMG)
	parted -s $(IMG) mklabel gpt
	parted -s $(IMG) mkpart ESP fat32 2048s 100%
	parted -s $(IMG) set 1 esp on
	$(eval LOOP := $(shell losetup -Pf --show $(IMG)))
	mkfs.fat -F 32 $(LOOP)p1
	mount $(LOOP)p1 /mnt
	mkdir -p /mnt/EFI/BOOT /mnt/boot/limine
	cp $(KERNEL) /mnt/boot/
	cp limine.conf /mnt/boot/limine/
	cp $(LIMINE_DIR)/BOOTX64.EFI /mnt/EFI/BOOT/
	cp $(LIMINE_DIR)/limine-bios.sys /mnt/boot/limine/
	umount /mnt
	losetup -d $(LOOP)
	$(LIMINE_DIR)/limine bios-install $(IMG)

run-qemu: $(ISO)
	qemu-system-x86_64 -serial stdio -m 4G -drive file=$(ISO),format=raw -no-reboot \
		-netdev user,id=net0 -device rtl8139,netdev=net0

run-qemu-gdb: $(ISO)
	qemu-system-x86_64 -serial stdio -m 2G -drive file=$(ISO),format=raw -s -S -no-reboot \
		-netdev user,id=net0 -device rtl8139,netdev=net0

# Test build — enables testing feature for unit tests
$(BUILD_DIR)/zenus-test: apps/src/lib.rs $(shell find crates -name '*.rs') apps/src/linker.ld
	RUSTFLAGS="-C target-cpu=x86-64" \
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

test: test-iso
	qemu-system-x86_64 -serial stdio -m 2G -cdrom $(BUILD_DIR)/zenus-test.iso -no-reboot \
		-drive file=ext2_test.img,format=raw,if=ide 2>&1

test-quiet: test-iso
	qemu-system-x86_64 -serial stdio -m 2G -cdrom $(BUILD_DIR)/zenus-test.iso -no-reboot \
		-drive file=ext2_test.img,format=raw,if=ide 2>&1 | grep -a "\[TEST\]"

clean:
	rm -rf $(BUILD_DIR) $(ISO_DIR)
	$(CARGO) clean
