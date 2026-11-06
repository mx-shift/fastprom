#!/usr/bin/env python3
"""Check the firmware against the AG32's memory map and report the flash layout.

Two board binaries, the DFU stage at the reset vector and the application
behind its header (docs/10-updates.md, "Flash map"), and the recovery
loader the page uploads into SRAM. Reads each linked ELF directly (section
and program headers, symbol table), so it needs no target binutils, and for
each of the two in flash:

  - every allocated section must sit in that binary's flash window; RAM
    sections in SRAM, and their flash load images (.data, .ramfunc) in the
    window too;
  - the entry point must be the one the map gives it: the reset vector for
    the DFU stage, the first byte after the header for the application;
  - the raw .bin must be exactly the flash image of the ELF;
  - no panic machinery may be linked (the firmware is built so that nothing
    can panic; a `core::panicking` symbol means a path crept in).

For the recovery loader: every section in its upload window at 0x20010000
(at most 5,296 bytes, the most AGM's tools upload through the mask ROM) or
in its RAM at the bottom of SRAM, .data's load image in the window; its
entry the instruction after its 32-byte header; the .bin exactly the bytes
to upload, its header stamped (tools/fw_stamp.py loader) with their length
and CRC, the version and board revision; no panic machinery.

For the application, also its header (fastprom-core header.rs): committed,
stamped by tools/fw_stamp.py with the length and CRC-32 of exactly the bytes
after it, the version in fw/Cargo.toml, the board revision in hw/REVISION
and the fabric ID the firmware expects. With --manifest, the manifest must
describe --fabric. With --batch, the combined image AGM's gen_batch wrote
must place the DFU stage, the application, the fabric bitstream, the
manifest and the image table where the ELFs' layout symbols say, and point
the option bytes at the bitstream.

The region boundaries come from the ELFs: fw/fastprom-hal/build_support.rs
records __fastprom_*_{start,end} from fw/fastprom-core/src/memmap.rs, and
both ELFs must agree on them.

usage: fw_layout.py --elf APP.elf --bin APP.bin --dfu-elf DFU.elf --dfu-bin DFU.bin
                    --recovery-elf REC.elf --recovery-bin REC.bin
                    [--fabric top.bin] [--manifest manifest.bin]
                    [--table images.bin] [--batch batch.bin]
"""
import argparse
import os
import re
import struct
import sys
import zlib

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SRAM_BASE = 0x20000000
SRAM_SIZE = 128 * 1024
SHF_ALLOC = 0x2
SHT_NOBITS = 8
PT_LOAD = 1
REGIONS = ("dfu", "app", "fabric", "manifest", "table", "option")


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


def regions(elf, path):
    """The flash map the ELF was linked against."""
    try:
        m = {r: (elf.symbols[f"__fastprom_{r}_start"], elf.symbols[f"__fastprom_{r}_end"])
             for r in REGIONS}
        m["image"] = (elf.symbols["__fastprom_image_start"], elf.symbols["__fastprom_image_end"])
        m["entry"] = elf.symbols["__fastprom_entry"]
        m["flash_end"] = elf.symbols["__fastprom_flash_end"]
        m["page"] = elf.symbols["__fastprom_page"]
    except KeyError as e:
        sys.exit(f"{path}: no layout symbol {e} (built without build_support.rs's memory.x?)")
    return m


def check_binary(what, elf_path, bin_path, errors):
    """Sections, entry, .bin and panics of one board binary; its layout."""
    elf = Elf(elf_path)
    m = regions(elf, elf_path)
    lo, hi = m["image"]
    print(f"{what} {elf_path}")
    print(f"  {'section':<17} {'address':>10} {'size':>8}  region")
    flash_end = lo
    ram_used = 0
    for s in elf.sections:
        if not s["flags"] & SHF_ALLOC or (s["size"] == 0 and s["name"] != ".stack"):
            continue
        addr, size = s["addr"], s["size"]
        if SRAM_BASE <= addr < SRAM_BASE + SRAM_SIZE:
            region = "SRAM"
            if addr + size > SRAM_BASE + SRAM_SIZE:
                errors.append(f"{what}: {s['name']} runs past the end of SRAM")
            if s["name"] not in (".stack", ".ramfunc"):
                ram_used += size
            if s["type"] != SHT_NOBITS:  # .data, .ramfunc: also a flash load image
                lma = elf.lma(addr)
                region += f", loaded from {lma:#010x}"
                if not (lo <= lma and lma + size <= hi):
                    errors.append(f"{what}: {s['name']}'s load image is outside its window")
                flash_end = max(flash_end, lma + size)
        elif lo <= addr and addr + size <= hi:
            region = "flash"
            flash_end = max(flash_end, addr + size)
        else:
            region = "OUTSIDE ITS REGION"
            errors.append(f"{what}: {s['name']} at {addr:#010x}+{size:#x} is outside "
                          f"its window {lo:#010x}-{hi:#010x}")
        print(f"  {s['name']:<17} {addr:#010x} {size:>8}  {region}")
    if elf.entry != m["entry"]:
        errors.append(f"{what}: entry point {elf.entry:#010x}, expected {m['entry']:#010x}")

    start, image = elf.flash_image()
    with open(bin_path, "rb") as fh:
        binary = fh.read()
    if start != lo or binary != image:
        errors.append(f"{what}: {bin_path} is not the ELF's flash image "
                      f"({len(binary)} bytes vs {len(image)} at {start:#010x})")

    panics = sorted(n for n in elf.symbols if "panicking" in n or "panic_bounds_check" in n)
    if panics:
        errors.append(f"{what}: panic machinery is linked: {', '.join(panics[:3])}")
    ramfunc = next((s for s in elf.sections if s["name"] == ".ramfunc"), None)
    rf = f", {ramfunc['size']} bytes of .ramfunc" if ramfunc else ""
    print(f"  entry {elf.entry:#010x}; SRAM: {ram_used} bytes of .data/.bss{rf}, the rest stack")
    print(f"  panic paths linked: {'yes' if panics else 'none'}")
    print()
    return elf, m, binary, flash_end


def check_recovery(elf_path, bin_path, errors):
    """The recovery loader: SRAM only, the upload window, its header."""
    import fw_stamp
    elf = Elf(elf_path)
    try:
        lo, hi = elf.symbols["__fastprom_image_start"], elf.symbols["__fastprom_image_end"]
        rlo, rhi = elf.symbols["__fastprom_ram_start"], elf.symbols["__fastprom_ram_end"]
        entry = elf.symbols["__fastprom_entry"]
    except KeyError as e:
        sys.exit(f"{elf_path}: no layout symbol {e} (not the recovery loader's memory.x?)")
    what = "recovery loader"
    print(f"{what} {elf_path}")
    print(f"  {'section':<23} {'address':>10} {'size':>8}  region")
    for s in elf.sections:
        if not s["flags"] & SHF_ALLOC or (s["size"] == 0 and s["name"] != ".stack"):
            continue
        addr, size = s["addr"], s["size"]
        if lo <= addr and addr + size <= hi:
            region = "upload window"
        elif rlo <= addr and addr + size <= rhi:
            region = "loader RAM"
            if s["type"] != SHT_NOBITS:
                lma = elf.lma(addr)
                region += f", loaded from {lma:#010x}"
                if not (lo <= lma and lma + size <= hi):
                    errors.append(f"{what}: {s['name']}'s load image is outside the window")
        else:
            region = "OUTSIDE ITS REGION"
            errors.append(f"{what}: {s['name']} at {addr:#010x}+{size:#x} is outside its "
                          f"window {lo:#010x}-{hi:#010x} and its RAM {rlo:#010x}-{rhi:#010x}")
        print(f"  {s['name']:<23} {addr:#010x} {size:>8}  {region}")
    if elf.entry != entry:
        errors.append(f"{what}: entry point {elf.entry:#010x}, expected {entry:#010x}")
    start, image = elf.flash_image()
    with open(bin_path, "rb") as fh:
        binary = fh.read()
    if start != lo or binary != image:
        errors.append(f"{what}: {bin_path} is not the ELF's upload image "
                      f"({len(binary)} bytes vs {len(image)} at {start:#010x})")
    if len(binary) > hi - lo:
        errors.append(f"{what}: {len(binary)} bytes, more than the {hi - lo} the window holds")
    try:
        h = fw_stamp.decode_loader(binary)
        want = {"version": cargo_version(), "board_rev": fw_stamp.board_rev()}
        for k, v in want.items():
            if h[k] != v:
                errors.append(f"{what} header: {k} is {h[k]!r}, expected {v!r}")
        print(f"  header: {h['length']} bytes, crc32 {h['crc']:#010x}, firmware "
              f"{h['version']}, board rev {h['board_rev']}")
    except ValueError as e:
        errors.append(f"{what} header: {e} (not stamped by tools/fw_stamp.py loader?)")
    panics = sorted(n for n in elf.symbols if "panicking" in n or "panic_bounds_check" in n)
    if panics:
        errors.append(f"{what}: panic machinery is linked: {', '.join(panics[:3])}")
    print(f"  entry {elf.entry:#010x}; {len(binary)} bytes to upload, "
          f"{100 * len(binary) / (hi - lo):.0f} % of {hi - lo}")
    print(f"  panic paths linked: {'yes' if panics else 'none'}")
    print()


def cargo_version():
    s = open(os.path.join(REPO, "fw", "Cargo.toml")).read()
    sect = s.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    return re.search(r'^version\s*=\s*"([^"]+)"', sect, re.M).group(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--elf", required=True, help="the application ELF")
    ap.add_argument("--bin", required=True, help="the application .bin")
    ap.add_argument("--dfu-elf", required=True, help="the DFU stage ELF")
    ap.add_argument("--dfu-bin", required=True, help="the DFU stage .bin")
    ap.add_argument("--recovery-elf", required=True, help="the recovery loader ELF")
    ap.add_argument("--recovery-bin", required=True, help="the recovery loader .bin")
    ap.add_argument("--fabric", help="the fabric bitstream (top.bin)")
    ap.add_argument("--manifest", help="the fabric manifest page (fw_stamp.py manifest)")
    ap.add_argument("--table", help="the image table (.bin)")
    ap.add_argument("--batch", help="the combined image from gen_batch")
    args = ap.parse_args()
    import fw_stamp  # here, not at the top: fw_stamp imports this module

    errors = []
    dfu, dm, dfu_bin, dfu_end = check_binary("DFU stage", args.dfu_elf, args.dfu_bin, errors)
    app, am, app_bin, app_end = check_binary("application", args.elf, args.bin, errors)
    check_recovery(args.recovery_elf, args.recovery_bin, errors)
    for r in REGIONS:
        if dm[r] != am[r]:
            errors.append(f"the two ELFs disagree on the {r} region: {dm[r]} vs {am[r]}")
    if dm["image"] != dm["dfu"] or am["image"] != am["app"]:
        errors.append("an ELF was linked into the other's region")
    m = am

    # The application header.
    page = m["page"]
    hdr = None
    try:
        hdr = fw_stamp.decode_app_header(app_bin[:page])
    except (ValueError, struct.error) as e:
        errors.append(f"application header: {e} (not stamped by tools/fw_stamp.py?)")
    if hdr:
        body = app_bin[page:]
        want = {
            "committed": True,
            "length": len(body),
            "crc": fw_stamp.crc32(body),
            "version": cargo_version(),
            "board_rev": fw_stamp.board_rev(),
            "fabric_id": fw_stamp.FABRIC_ID,
        }
        for k, v in want.items():
            if hdr[k] != v:
                errors.append(f"application header: {k} is {hdr[k]!r}, expected {v!r}")
        print(f"application header at {m['app'][0]:#010x}: {hdr['length']} bytes, crc32 "
              f"{hdr['crc']:#010x}, firmware {hdr['version']}, board rev {hdr['board_rev']}, "
              f"fabric ID {hdr['fabric_id']}, committed")
        print()

    flash_base, flash_end = m["dfu"][0], m["flash_end"]
    print(f"flash, {kib(flash_end - flash_base)} at {flash_base:#010x}:")

    def row(lo, hi, what):
        print(f"  {lo:#010x}-{hi:#010x}  {kib(hi - lo):>10}  {what}")

    def used(lo, hi, end, n, what):
        row(lo, end, f"{what}, {n} bytes ({100 * (end - lo) / (hi - lo):.0f} % of {kib(hi - lo)})")
        if end < hi:
            row(end, hi, "free")

    used(*m["dfu"], dfu_end, len(dfu_bin), "DFU stage")
    used(*m["app"], app_end, len(app_bin), "application, header and code")
    fabric_data = None
    if args.fabric:
        with open(args.fabric, "rb") as fh:
            fabric_data = fh.read()
        lo, hi = m["fabric"]
        n = len(fabric_data)
        row(lo, min(lo + n, hi), f"fabric bitstream {args.fabric}, {n} bytes")
        if lo + n < hi:
            row(lo + n, hi, "fabric reserve, unused")
        if lo + n > hi:
            errors.append(f"{args.fabric} is {n} bytes, larger than the fabric reserve")
    else:
        row(*m["fabric"], "fabric bitstream reserve")
    manifest_data = None
    if args.manifest:
        with open(args.manifest, "rb") as fh:
            manifest_data = fh.read()
        row(*m["manifest"], f"fabric manifest {args.manifest}, {len(manifest_data)} bytes")
        if fabric_data is None:
            errors.append("--manifest needs --fabric, to check it against")
        else:
            want = fw_stamp.manifest(fabric_data, *fw_stamp.manifest_fields(manifest_data))
            if manifest_data != want:
                errors.append(f"{args.manifest} does not describe {args.fabric}")
    else:
        row(*m["manifest"], "fabric manifest")
    if args.table:
        lo, hi = m["table"]
        n = os.path.getsize(args.table)
        row(lo, lo + n, f"image table {args.table}, {n} bytes")
        row(lo + n, hi, "free")
        if lo + n > hi:
            errors.append(f"{args.table} is {n} bytes, larger than the table region")
    else:
        row(*m["table"], "image table region")

    if args.batch:
        print()
        print(f"combined image {args.batch}:")
        secs = batch_sections(args.batch)
        for addr, data in secs:
            print(f"  {addr:#010x}  {len(data):>7} bytes")
        by_addr = {a: d for a, d in secs}
        want = {m["dfu"][0]: ("DFU stage", dfu_bin), m["app"][0]: ("application", app_bin)}
        if args.table:
            with open(args.table, "rb") as fh:
                want[m["table"][0]] = ("image table", fh.read())
        if fabric_data is not None:
            want[m["fabric"][0]] = ("fabric bitstream", fabric_data)
        if manifest_data is not None:
            want[m["manifest"][0]] = ("fabric manifest", manifest_data)
        for addr, (what, data) in want.items():
            if by_addr.get(addr) != data:
                errors.append(f"{args.batch}: the {what} is not at {addr:#010x}")
        opt = by_addr.get(m["option"][0])
        fabric_lo = m["fabric"][0]
        if opt is None:
            errors.append(f"{args.batch}: no option bytes")
        else:
            cfg, cfg_n = struct.unpack_from("<II", opt, 0x30)
            if cfg != fabric_lo or cfg_n != 0xFFFFFFFF - fabric_lo:
                errors.append(f"{args.batch}: option byte 0x30 points the fabric at {cfg:#010x}")
            else:
                print(f"  option bytes: fabric configuration address {cfg:#010x} "
                      f"({m['option'][0] + 0x30:#010x}, uncompressed)")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
    print()
    print("layout OK")


if __name__ == "__main__":
    main()
