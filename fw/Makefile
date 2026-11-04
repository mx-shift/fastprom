# Firmware build.
#
# Two targets:
#   test  - build and run the host tests (needs only gcc)
#   fw    - cross-compile for the AG32 (needs a RISC-V toolchain)
#
# The cross build deliberately stops at objects. Startup code, the linker
# script and the flash layout come from the vendor AG32 SDK or from the
# AGaMEMnon open SDK, and inventing our own here would only be guessing at
# something those already define. See docs/06-build-and-flash.md.

CC      ?= gcc
CROSS   ?= riscv-none-elf-
CFLAGS  := -Wall -Wextra -Werror -O2 -I include
TESTFLAGS := $(CFLAGS) -I test -DFASTPROM_FABRIC_MOCK

SRC := src/crc32.c src/loader.c

.PHONY: all test fw clean

all: test

test: test/vectors.h
	@mkdir -p ../build
	$(CC) $(TESTFLAGS) -o ../build/test_fw test/test_fw.c $(SRC)
	@../build/test_fw

test/vectors.h: test/gen_vectors.py
	python3 test/gen_vectors.py

fw:
	@mkdir -p ../build/fw
	$(CROSS)gcc $(CFLAGS) -march=rv32imafc -mabi=ilp32f -c $(SRC) \
		-o /dev/null 2>/dev/null || \
		$(CROSS)gcc $(CFLAGS) -march=rv32imac -mabi=ilp32 \
			$(patsubst %,-c %,$(SRC))
	@echo "objects built; link with the SDK's startup and linker script"

clean:
	rm -rf ../build/test_fw ../build/fw test/vectors.h
