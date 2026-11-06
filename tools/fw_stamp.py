#!/usr/bin/env python3
"""Stamp the application header, and write fabric manifests.

The two pages fw/fastprom-core/src/header.rs defines (its docs have the
layout; this file must agree with it, and the host tests check that it does
against fixtures this module writes, fw/tools/gen_fixtures.py):

  app ELF    fill in the application header that fastprom-ag32 links at
             0x80008000: the image length (every byte the ELF loads after
             the header), its CRC-32, and the header CRC. The firmware links
             the header with its version, board revision and fabric ID, and
             zero length and CRCs, which the DFU stage refuses to boot, so
             an unstamped build cannot run. Patches the ELF in place, before
             objcopy turns it into the .bin; running it again changes
             nothing.
  manifest   write the 256-byte manifest page for a fabric bitstream: its
             length and CRC-32, the part, the board revision and the fabric
             ID.
  loader ELF fill in the recovery loader's 32-byte header (fastprom-core
             recovery/mod.rs LoaderHeader), which fastprom-recovery links at
             0x20010000: the length and CRC-32 of every byte the ELF loads
             after it. In place, like app.

option_bytes() is the 128 option bytes UART recovery writes (fastprom-core
recovery/mod.rs OPTION_BYTES; gen_batch's for --logic-address 0x80014000),
for tools/mkbundle.py.

All CRCs are zlib.crc32 (fastprom-core's crc32). Both pages are written with
their commit word set, as the UART loader programs them; over USB the
device holds the commit word back until COMMIT has checked the payload.

usage: fw_stamp.py app --elf E
       fw_stamp.py loader --elf E
       fw_stamp.py manifest --fabric top.bin --part 512x8 --out manifest.bin
                   [--board-rev A] [--fabric-id 1]
"""
import argparse
import os
import struct
import sys
import zlib

from fw_layout import Elf

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

PAGE = 256
APP_BASE = 0x80008000
APP_LEN = 48 * 1024
FABRIC_LEN = 100 * 1024
APP_MAGIC = 0x46504150       # 'FPAP'
APP_VERSION = 1
MANIFEST_MAGIC = 0x46504D46  # 'FPMF'
MANIFEST_VERSION = 1
FABRIC_ID = 1                # fastprom-core regmap::FABRIC_ID
ERASED = 0xFFFFFFFF
HEADER_SECTION = ".fastprom_header"
# The recovery loader (fastprom-core memmap.rs RECOVERY_*, recovery/mod.rs).
LOADER_BASE = 0x20010000
LOADER_LEN = 5296
LOADER_HEADER_LEN = 32
LOADER_MAGIC = 0x46504C44    # 'FPLD'
LOADER_VERSION = 1
LOADER_SECTION = ".fastprom_loader_header"
# The option bytes: RDP unprotected, the fabric's address (memmap.rs
# FABRIC_BASE) and its complement at 0x30.
FABRIC_BASE = 0x80014000
OPTION_LEN = 128

# Offsets (fastprom-core header.rs).
OFF_COMMIT, OFF_VERSION, OFF_LEN, OFF_CRC, OFF_PAGE_CRC = 0x00, 0x04, 0x08, 0x0C, 0xFC
APP_OFF_FW_VERSION, APP_OFF_BOARD_REV, APP_OFF_FABRIC_ID = 0x10, 0x18, 0x20
MAN_OFF_PART, MAN_OFF_BOARD_REV, MAN_OFF_FABRIC_ID = 0x10, 0x20, 0x28
BOARD_REV_LEN, PART_LEN = 8, 16


def crc32(data):
    return zlib.crc32(data) & 0xFFFFFFFF


def text(s, n, what):
    b = s.encode("ascii")
    if not 1 <= len(b) <= n or not all(0x21 <= c <= 0x7E for c in b):
        sys.exit(f"{what} {s!r}: 1 to {n} printable characters, no spaces")
    return b.ljust(n, b"\0")


def board_rev():
    """hw/REVISION, as scripts/release_hw.sh reads it."""
    with open(os.path.join(REPO, "hw", "REVISION")) as fh:
        return "".join(fh.read().split())


def seal(page):
    page[OFF_PAGE_CRC:OFF_PAGE_CRC + 4] = struct.pack(
        "<I", crc32(bytes(page[OFF_VERSION:OFF_PAGE_CRC])))
    return bytes(page)


def app_header(image, version, rev, fabric_id, commit=True):
    """The whole header page for `image` (the bytes after it)."""
    p = bytearray(PAGE)
    struct.pack_into("<IIII", p, 0, APP_MAGIC if commit else ERASED, APP_VERSION,
                     len(image), crc32(image))
    struct.pack_into("<HHHH", p, APP_OFF_FW_VERSION, *version, 0)
    p[APP_OFF_BOARD_REV:APP_OFF_BOARD_REV + BOARD_REV_LEN] = text(rev, BOARD_REV_LEN,
                                                                  "board revision")
    struct.pack_into("<I", p, APP_OFF_FABRIC_ID, fabric_id)
    return seal(p)


def manifest(fabric, part, rev, fabric_id, commit=True):
    """The manifest page for a bitstream."""
    if not 1 <= len(fabric) <= FABRIC_LEN:
        sys.exit(f"bitstream is {len(fabric)} bytes, not 1 to {FABRIC_LEN}")
    p = bytearray(PAGE)
    struct.pack_into("<IIII", p, 0, MANIFEST_MAGIC if commit else ERASED,
                     MANIFEST_VERSION, len(fabric), crc32(fabric))
    p[MAN_OFF_PART:MAN_OFF_PART + PART_LEN] = text(part, PART_LEN, "part")
    p[MAN_OFF_BOARD_REV:MAN_OFF_BOARD_REV + BOARD_REV_LEN] = text(rev, BOARD_REV_LEN,
                                                                  "board revision")
    struct.pack_into("<I", p, MAN_OFF_FABRIC_ID, fabric_id)
    return seal(p)


def manifest_fields(page):
    """(part, board revision, fabric ID) of a manifest page, to rebuild it."""
    part = bytes(page[MAN_OFF_PART:MAN_OFF_PART + PART_LEN]).rstrip(b"\0").decode("ascii")
    rev = bytes(page[MAN_OFF_BOARD_REV:MAN_OFF_BOARD_REV + BOARD_REV_LEN]).rstrip(b"\0")
    return part, rev.decode("ascii"), struct.unpack_from("<I", page, MAN_OFF_FABRIC_ID)[0]


def decode_app_header(page):
    """The header's fields, as a dict; raises ValueError if malformed."""
    commit, version, length, crc = struct.unpack_from("<IIII", page, 0)
    if commit not in (APP_MAGIC, ERASED):
        raise ValueError(f"commit word {commit:#010x}")
    if version != APP_VERSION:
        raise ValueError(f"header version {version}")
    if not 1 <= length <= APP_LEN - PAGE:
        raise ValueError(f"image length {length}")
    stored = struct.unpack_from("<I", page, OFF_PAGE_CRC)[0]
    if stored != crc32(bytes(page[OFF_VERSION:OFF_PAGE_CRC])):
        raise ValueError("header CRC")
    major, minor, patch, pad = struct.unpack_from("<HHHH", page, APP_OFF_FW_VERSION)
    rev = bytes(page[APP_OFF_BOARD_REV:APP_OFF_BOARD_REV + BOARD_REV_LEN]).rstrip(b"\0")
    return {
        "committed": commit == APP_MAGIC,
        "length": length,
        "crc": crc,
        "version": f"{major}.{minor}.{patch}",
        "board_rev": rev.decode("ascii", "replace"),
        "fabric_id": struct.unpack_from("<I", page, APP_OFF_FABRIC_ID)[0],
    }


def option_bytes():
    """The option bytes recovery writes, as gen_batch lays them out
    (gen_option_bytes: RDP 0xA5 and its complement, then erased bytes, the
    logic address and its complement at 0x30, then erased to 128)."""
    o = bytearray(b"\xff" * OPTION_LEN)
    o[0:2] = b"\xa5\x5a"
    struct.pack_into("<II", o, 0x30, FABRIC_BASE, 0xFFFFFFFF - FABRIC_BASE)
    return bytes(o)


def loader_header(image, version, rev):
    """The recovery loader's header for `image` (the bytes after it)."""
    h = bytearray(LOADER_HEADER_LEN)
    struct.pack_into("<IIII", h, 0, LOADER_MAGIC, LOADER_VERSION, len(image), crc32(image))
    struct.pack_into("<HHHH", h, 0x10, *version, 0)
    h[0x18:0x18 + BOARD_REV_LEN] = text(rev, BOARD_REV_LEN, "board revision")
    return bytes(h)


def decode_loader(data):
    """A whole loader file's header fields; raises ValueError if it is not one."""
    if len(data) <= LOADER_HEADER_LEN or len(data) > LOADER_LEN:
        raise ValueError(f"{len(data)} bytes: not 1 to {LOADER_LEN - LOADER_HEADER_LEN} "
                         f"after a {LOADER_HEADER_LEN}-byte header")
    magic, version, length, crc = struct.unpack_from("<IIII", data, 0)
    if magic != LOADER_MAGIC or version != LOADER_VERSION:
        raise ValueError(f"no version {LOADER_VERSION} loader header")
    if length != len(data) - LOADER_HEADER_LEN or crc != crc32(data[LOADER_HEADER_LEN:]):
        raise ValueError("the header does not describe the image after it")
    major, minor, patch, _ = struct.unpack_from("<HHHH", data, 0x10)
    rev = bytes(data[0x18:0x18 + BOARD_REV_LEN]).rstrip(b"\0").decode("ascii", "replace")
    return {"length": length, "crc": crc, "version": f"{major}.{minor}.{patch}",
            "board_rev": rev}


def stamp_loader(path):
    elf = Elf(path)
    sec = next((s for s in elf.sections if s["name"] == LOADER_SECTION), None)
    if sec is None:
        sys.exit(f"no {LOADER_SECTION} section: not a fastprom-recovery ELF")
    if sec["addr"] != LOADER_BASE or sec["size"] != LOADER_HEADER_LEN:
        sys.exit(f"{LOADER_SECTION} is {sec['size']} bytes at {sec['addr']:#010x}, "
                 f"not {LOADER_HEADER_LEN} at {LOADER_BASE:#010x}")
    start, image = elf.flash_image()
    if start != LOADER_BASE:
        sys.exit(f"the loader's image starts at {start:#010x}, not {LOADER_BASE:#010x}")
    payload = image[LOADER_HEADER_LEN:]
    if not 1 <= len(payload) <= LOADER_LEN - LOADER_HEADER_LEN:
        sys.exit(f"the loader is {len(payload)} bytes after its header, more than the "
                 f"{LOADER_LEN - LOADER_HEADER_LEN} the mask ROM upload may be")
    old = bytearray(elf.raw[sec["offset"]:sec["offset"] + LOADER_HEADER_LEN])
    magic, version = struct.unpack_from("<II", old, 0)
    if magic != LOADER_MAGIC or version != LOADER_VERSION:
        sys.exit(f"{LOADER_SECTION} does not hold a version {LOADER_VERSION} loader header")
    struct.pack_into("<II", old, 8, len(payload), crc32(payload))
    raw = bytearray(elf.raw)
    raw[sec["offset"]:sec["offset"] + LOADER_HEADER_LEN] = old
    with open(path, "wb") as fh:
        fh.write(raw)
    h = decode_loader(bytes(old) + payload)
    print(f"{path}: loader header stamped, {h['length']} bytes, crc32 {h['crc']:#010x}, "
          f"firmware {h['version']}, board rev {h['board_rev']}")


def header_section(elf):
    for s in elf.sections:
        if s["name"] == HEADER_SECTION:
            return s
    sys.exit(f"no {HEADER_SECTION} section: not a fastprom-ag32 ELF")


def stamp(path):
    elf = Elf(path)
    sec = header_section(elf)
    if sec["addr"] != APP_BASE or sec["size"] != PAGE:
        sys.exit(f"{HEADER_SECTION} is {sec['size']} bytes at {sec['addr']:#010x}, "
                 f"not {PAGE} at {APP_BASE:#010x}")
    start, image = elf.flash_image()
    if start != APP_BASE:
        sys.exit(f"the flash image starts at {start:#010x}, not {APP_BASE:#010x}")
    payload = image[PAGE:]
    old = bytearray(elf.raw[sec["offset"]:sec["offset"] + PAGE])
    commit, version = struct.unpack_from("<II", old, 0)
    if commit != APP_MAGIC or version != APP_VERSION:
        sys.exit(f"{HEADER_SECTION} does not hold a version {APP_VERSION} header")
    if not 1 <= len(payload) <= APP_LEN - PAGE:
        sys.exit(f"the image is {len(payload)} bytes, more than the "
                 f"{APP_LEN - PAGE} after the header")
    struct.pack_into("<II", old, OFF_LEN, len(payload), crc32(payload))
    page = seal(old)
    raw = bytearray(elf.raw)
    raw[sec["offset"]:sec["offset"] + PAGE] = page
    with open(path, "wb") as fh:
        fh.write(raw)
    h = decode_app_header(page)
    print(f"{path}: header stamped, {h['length']} bytes, crc32 {h['crc']:#010x}, "
          f"firmware {h['version']}, board rev {h['board_rev']}, fabric ID {h['fabric_id']}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("app")
    a.add_argument("--elf", required=True)
    lo = sub.add_parser("loader")
    lo.add_argument("--elf", required=True)
    m = sub.add_parser("manifest")
    m.add_argument("--fabric", required=True)
    m.add_argument("--part", required=True)
    m.add_argument("--out", required=True)
    m.add_argument("--board-rev", default=None, help="default: hw/REVISION")
    m.add_argument("--fabric-id", type=int, default=FABRIC_ID)
    args = ap.parse_args()
    if args.cmd == "app":
        stamp(args.elf)
    elif args.cmd == "loader":
        stamp_loader(args.elf)
    else:
        with open(args.fabric, "rb") as fh:
            fabric = fh.read()
        rev = args.board_rev or board_rev()
        page = manifest(fabric, args.part, rev, args.fabric_id)
        with open(args.out, "wb") as fh:
            fh.write(page)
        print(f"{args.out}: manifest for {args.fabric}, {len(fabric)} bytes, crc32 "
              f"{crc32(fabric):#010x}, part {args.part}, board rev {rev}, "
              f"fabric ID {args.fabric_id}")


if __name__ == "__main__":
    main()
