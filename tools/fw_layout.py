#!/usr/bin/env python3
"""Check the firmware against the AG32's memory map and report the flash layout.

Reads the linked ELF directly (section and program headers, symbol table), so
it needs no target binutils:

  - every allocated section must sit in its region: code and read-only data
    in the firmware's flash window, below the fabric bitstream; .data's load
    image too; RAM sections in SRAM;
  - the raw .bin must be exactly the flash image of the ELF;
  - no panic machinery may be linked (the firmware is built so that nothing
    can panic; a `core::panicking` symbol means a path crept in);
  - with --batch, the combined image AGM's gen_batch wrote must place the
    firmware, the image table and the fabric bitstream where the ELF's
    layout symbols say, and point the option bytes at the bitstream.

The region boundaries come from the ELF itself: fw/fastprom-ag32/build.rs
records __fastprom_{fabric,table}_{start,end} from src/memmap.rs.

usage: fw_layout.py --elf E --bin B [--fabric top.bin] [--table images.bin]
                    [--batch batch.bin]
"""
import argparse
import os
import struct
import sys
import zlib

FLASH_BASE = 0x80000000
FLASH_SIZE = 256 * 1024
SRAM_BASE = 0x20000000
SRAM_SIZE = 128 * 1024
OPTION_BYTES = 0x81000000
SHF_ALLOC = 0x2
SHT_NOBITS = 8
PT_LOAD = 1


class Elf:
    def __init__(self, path):
        with open(path, "rb") as fh:
            self.raw = fh.read()
        raw = self.raw
        if raw[:4] != b"\x7fELF" or raw[4] != 1 or raw[5] != 1:
            sys.exit(f"{path}: not a little-endian ELF32 file")
        (_type, machine, _ver, self.entry, phoff, shoff, _flags, _ehsize,
         phentsize, phnum, shentsize, shnum, shstrndx) = struct.unpack_from(
            "<HHIIIIIHHHHHH", raw, 16)
        if machine != 243:
            sys.exit(f"{path}: machine {machine}, not RISC-V")
        self.segments = []
        for i in range(phnum):
            (p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, _f,
             _a) = struct.unpack_from("<8I", raw, phoff + i * phentsize)
            if p_type == PT_LOAD:
                self.segments.append((p_offset, p_vaddr, p_paddr, p_filesz, p_memsz))
        shdrs = [struct.unpack_from("<10I", raw, shoff + i * shentsize)
                 for i in range(shnum)]
        strtab = shdrs[shstrndx]
        self.sections = []
        self.symbols = {}
        for sh in shdrs:
            name = self._str(strtab[4], sh[0])
            self.sections.append({"name": name, "type": sh[1], "flags": sh[2],
                                  "addr": sh[3], "offset": sh[4], "size": sh[5]})
            if sh[1] == 2:  # SHT_SYMTAB
                symstr = shdrs[sh[6]]
                for off in range(sh[4], sh[4] + sh[5], 16):
                    st_name, st_value, _sz, _info, _other, _shndx = struct.unpack_from(
                        "<IIIBBH", raw, off)
                    self.symbols[self._str(symstr[4], st_name)] = st_value

    def _str(self, table_off, off):
        end = self.raw.index(b"\0", table_off + off)
        return self.raw[table_off + off:end].decode()

    def lma(self, addr):
        """Load address of a VMA (where it lives in flash)."""
        for _off, vaddr, paddr, _filesz, memsz in self.segments:
            if vaddr <= addr < vaddr + max(memsz, 1):
                return paddr + (addr - vaddr)
        return addr

    def flash_image(self):
        """The bytes objcopy -O binary writes: every loaded byte, by LMA."""
        loads = [(paddr, self.raw[off:off + filesz])
                 for off, _v, paddr, filesz, _m in self.segments if filesz]
        start = min(p for p, _ in loads)
        end = max(p + len(d) for p, d in loads)
        image = bytearray(end - start)
        for paddr, data in loads:
            image[paddr - start:paddr - start + len(data)] = data
        return start, bytes(image)


def kib(n):
    return f"{n / 1024:.1f} KiB"


def batch_sections(path):
    """Sections of an AGM download batch (gen_batch's DownloadHeader)."""
    with open(path, "rb") as fh:
        raw = fh.read()
    out = []
    i = 0
    while i < len(raw):
        hdr = raw[i:i + 256]
        if len(hdr) < 256:
            sys.exit(f"{path}: truncated header at {i}")
        _dev, size, addr = struct.unpack_from("<III", hdr)
        if zlib.crc32(hdr[:252]) & 0xFFFFFFFF != struct.unpack_from("<I", hdr, 252)[0]:
            sys.exit(f"{path}: bad header checksum at {i}")
        out.append((addr, raw[i + 256:i + 256 + size]))
        i += 256 + size
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--elf", required=True)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--fabric", help="the fabric bitstream (top.bin)")
    ap.add_argument("--table", help="the image table (.bin)")
    ap.add_argument("--batch", help="the combined image from gen_batch")
    args = ap.parse_args()

    elf = Elf(args.elf)
    errors = []
    sym = elf.symbols
    try:
        fabric_lo, fabric_hi = sym["__fastprom_fabric_start"], sym["__fastprom_fabric_end"]
        table_lo, table_hi = sym["__fastprom_table_start"], sym["__fastprom_table_end"]
    except KeyError as e:
        sys.exit(f"{args.elf}: no layout symbol {e} (built without build.rs's memory.x?)")
    fw_lo, fw_hi = FLASH_BASE, fabric_lo

    print(f"firmware {args.elf}")
    print(f"  {'section':<14} {'address':>10} {'size':>8}  region")
    flash_end = fw_lo
    ram_used = 0
    for s in elf.sections:
        if not s["flags"] & SHF_ALLOC or (s["size"] == 0 and s["name"] != ".stack"):
            continue
        addr, size = s["addr"], s["size"]
        if SRAM_BASE <= addr < SRAM_BASE + SRAM_SIZE:
            region = "SRAM"
            if addr + size > SRAM_BASE + SRAM_SIZE:
                errors.append(f"{s['name']} runs past the end of SRAM")
            if s["name"] != ".stack":
                ram_used += size
            if s["type"] != SHT_NOBITS:  # .data: also has a flash load image
                lma = elf.lma(addr)
                region += f", loaded from {lma:#010x}"
                if not (fw_lo <= lma and lma + size <= fw_hi):
                    errors.append(f"{s['name']}'s load image is outside the firmware window")
                flash_end = max(flash_end, lma + size)
        elif fw_lo <= addr and addr + size <= fw_hi:
            region = "flash (firmware)"
            flash_end = max(flash_end, addr + size)
        else:
            region = "OUTSIDE ITS REGION"
            errors.append(f"{s['name']} at {addr:#010x}+{size:#x} is outside "
                          f"the firmware window {fw_lo:#010x}-{fw_hi:#010x}")
        print(f"  {s['name']:<14} {addr:#010x} {size:>8}  {region}")
    if elf.entry != FLASH_BASE:
        errors.append(f"entry point {elf.entry:#010x}, the mask ROM jumps to {FLASH_BASE:#010x}")

    start, image = elf.flash_image()
    with open(args.bin, "rb") as fh:
        binary = fh.read()
    if start != FLASH_BASE or binary != image:
        errors.append(f"{args.bin} is not the ELF's flash image "
                      f"({len(binary)} bytes vs {len(image)} at {start:#010x})")

    panics = sorted(n for n in sym if "panicking" in n or "panic_bounds_check" in n)
    if panics:
        errors.append(f"panic machinery is linked: {', '.join(panics[:3])}")

    used = flash_end - fw_lo
    window = fw_hi - fw_lo
    print(f"  entry {elf.entry:#010x}; SRAM: {ram_used} bytes of .data/.bss, the rest stack")
    print(f"  panic paths linked: {'yes' if panics else 'none'}")
    print()
    print(f"flash, {kib(FLASH_SIZE)} at {FLASH_BASE:#010x}:")

    def row(lo, hi, what):
        print(f"  {lo:#010x}-{hi:#010x}  {kib(hi - lo):>10}  {what}")

    row(fw_lo, flash_end, f"firmware, {len(binary)} bytes ({100 * used / window:.0f} % of "
        f"the {kib(window)} window)")
    row(flash_end, fw_hi, "free")
    if args.fabric:
        n = os.path.getsize(args.fabric)
        row(fabric_lo, fabric_lo + n, f"fabric bitstream {args.fabric}, {n} bytes")
        row(fabric_lo + n, fabric_hi, "fabric reserve, unused")
        if fabric_lo + n > fabric_hi:
            errors.append(f"{args.fabric} is {n} bytes, larger than the fabric reserve")
    else:
        row(fabric_lo, fabric_hi, "fabric bitstream reserve")
    if args.table:
        n = os.path.getsize(args.table)
        row(table_lo, table_lo + n, f"image table {args.table}, {n} bytes")
        row(table_lo + n, table_hi, "free")
        if table_lo + n > table_hi:
            errors.append(f"{args.table} is {n} bytes, larger than the table region")
    else:
        row(table_lo, table_hi, "image table region")

    if args.batch:
        print()
        print(f"combined image {args.batch}:")
        secs = batch_sections(args.batch)
        for addr, data in secs:
            print(f"  {addr:#010x}  {len(data):>7} bytes")
        by_addr = {a: d for a, d in secs}
        want = {FLASH_BASE: ("firmware", binary)}
        if args.table:
            with open(args.table, "rb") as fh:
                want[table_lo] = ("image table", fh.read())
        if args.fabric:
            with open(args.fabric, "rb") as fh:
                want[fabric_lo] = ("fabric bitstream", fh.read())
        for addr, (what, data) in want.items():
            if by_addr.get(addr) != data:
                errors.append(f"{args.batch}: the {what} is not at {addr:#010x}")
        opt = by_addr.get(OPTION_BYTES)
        if opt is None:
            errors.append(f"{args.batch}: no option bytes")
        else:
            cfg, cfg_n = struct.unpack_from("<II", opt, 0x30)
            if cfg != fabric_lo or cfg_n != 0xFFFFFFFF - fabric_lo:
                errors.append(f"{args.batch}: option byte 0x30 points the fabric at {cfg:#010x}")
            else:
                print(f"  option bytes: fabric configuration address {cfg:#010x} "
                      "(0x81000030, uncompressed)")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
    print()
    print("layout OK")


if __name__ == "__main__":
    main()
