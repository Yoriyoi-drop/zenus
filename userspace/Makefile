ARCH ?= x86_64
TARGET = $(ARCH)-unknown-none
CARGO := cargo
CARGO_FLAGS ?=
BUILD_DIR := build
PROFILE_DIR := $(if $(filter --release,$(CARGO_FLAGS)),release,debug)
LD := ld.lld
PROGRAMS := hello echo cat exitonly

.PHONY: all clean $(PROGRAMS)

all: $(PROGRAMS)

# Build each userspace program as staticlib, then link into standalone ELF
target/$(TARGET)/$(PROFILE_DIR)/libsyscall_core.a: ../zutils/crates/common/src/lib.rs $(shell find syscall_core -name '*.rs') $(shell find syscall_core -name '*.toml')
	$(CARGO) build --package syscall_core --target $(TARGET) $(CARGO_FLAGS)

define PROGRAM_template
target/$(TARGET)/$(PROFILE_DIR)/lib$(1).a: $(1)/src/lib.rs $(1)/Cargo.toml target/$(TARGET)/$(PROFILE_DIR)/libsyscall_core.a
	$(CARGO) build --package $(1) --target $(TARGET) $(CARGO_FLAGS)

$(BUILD_DIR)/$(1): target/$(TARGET)/$(PROFILE_DIR)/lib$(1).a userspace.ld
	mkdir -p $(BUILD_DIR)
	$(LD) -T userspace.ld -o $$@ \
		--gc-sections --strip-all \
		target/$(TARGET)/$(PROFILE_DIR)/lib$(1).a

$(1): $(BUILD_DIR)/$(1)
endef

$(foreach prog,$(PROGRAMS),$(eval $(call PROGRAM_template,$(prog))))

clean:
	rm -rf $(BUILD_DIR)
	$(CARGO) clean
