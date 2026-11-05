#!/usr/bin/env python3
"""Pack ROM images into the image table the RISC-V core loads into block RAM.

BRAM mode keeps the contents in the microcontroller's flash and copies them
into the fabric's M9K block at boot. This builds that table: a small header
the firmware can sanity-check, followed by the image data.

Each image is checked with CRC-32 so the firmware can refuse to serve a
corrupt image rather than driving a wrong value onto a host's bus - the
outputs stay tri-stated instead, which is the safe failure.

Two encodings of the same table (format version 1), chosen by --out:

  *.bin  the binary table the Rust firmware reads from flash at TABLE_ADDR
         (0x80020000, fw/fastprom-ag32/memory.x); layout below
  *.c    a C source table (fastprom_image.h), as the retired C firmware
         linked it

Binary layout, all integers little-endian u32:

  0x000  magic        0x4650524d ('FPRM')
  0x004  version      1
  0x008  depth        words per image, a power of two, 16..65536
  0x00c  width        bits per word, 4 or 8
  0x010  image_count  1, 2 or 4 (two image-select switches)
  0x014  part[28]     target part name, ASCII, NUL padded, NUL terminated
  0x030  4 entries of 48 bytes, one per image (unused ones all zero):
           +0x00 name[40]  source filename, ASCII, NUL padded and terminated
           +0x28 crc32     zlib.crc32 over exactly depth bytes of data
           +0x2c reserved  0
  0x0f0  reserved[12] 0
  0x0fc  header_crc   zlib.crc32 of bytes 0x000..0x0fb
  0x100  image data: image i at 0x100 + i * depth, one word per byte

fw/fastprom-core/src/table.rs parses exactly this; change both together.
"""
import argparse
import os
import struct
import sys
import zlib

MAGIC = 0x4650524D  # 'FPRM'
FORMAT_VERSION = 1

# Binary table geometry (see the docstring).
BIN_HEADER_LEN = 0x100
BIN_MAX_IMAGES = 4
BIN_PART_LEN = 28
BIN_NAME_LEN = 40
BIN_MIN_DEPTH = 16
BIN_MAX_DEPTH = 1 << 16


def ascii_field(text, length, what):
    """NUL-padded ASCII, always NUL terminated; truncates with a warning."""
    raw = text.encode("ascii", errors="replace")
    if len(raw) > length - 1:
        print(f"warning: {what} {text!r} truncated to {length - 1} bytes",
              file=sys.stderr)
        raw = raw[:length - 1]
    return raw + b"\0" * (length - len(raw))


def binary_table(part, depth, width, rows):
    """The binary encoding of the table: header, directory, image data."""
    if not (BIN_MIN_DEPTH <= depth <= BIN_MAX_DEPTH) or depth & (depth - 1):
        sys.exit(f"depth {depth}: the binary table needs a power of two "
                 f"from {BIN_MIN_DEPTH} to {BIN_MAX_DEPTH}")
    if len(rows) not in (1, 2, 4):
        sys.exit(f"{len(rows)} images: the image-select switch has two bits, "
                 "so give 1, 2 or 4")
    head = struct.pack("<5I", MAGIC, FORMAT_VERSION, depth, width, len(rows))
    head += ascii_field(part, BIN_PART_LEN, "part name")
    for src, crc, _words in rows:
        head += ascii_field(src, BIN_NAME_LEN, "image name")
        head += struct.pack("<2I", crc, 0)
    head += b"\0" * (48 * (BIN_MAX_IMAGES - len(rows)))
    head += b"\0" * 12
    assert len(head) == BIN_HEADER_LEN - 4, len(head)
    head += struct.pack("<I", zlib.crc32(head) & 0xFFFFFFFF)
    return head + b"".join(bytes(words) for _src, _crc, words in rows)


def read_image(path, width, depth):
    if path.endswith(".bin"):
        with open(path, "rb") as fh:
            words = list(fh.read())
    else:
        words = []
        with open(path) as fh:
            for raw in fh:
                line = raw.split("//")[0].strip()
                if line:
                    words.extend(int(t, 16) for t in line.split())

    mask = (1 << width) - 1
    if any(w > mask for w in words):
        sys.exit(f"{path}: contains a word wider than {width} bits")
    if len(words) > depth:
        sys.exit(f"{path}: {len(words)} words exceeds the {depth}-word organisation")

    # Unprogrammed bipolar PROM cells read as 1s.
    words += [mask] * (depth - len(words))
    return words


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", action="append", required=True, metavar="PATH",
                    help="an image; repeat for a multi-image build (bank order)")
    ap.add_argument("--depth", type=int, required=True,
                    help="words per image, e.g. 512 for a 512x8 part")
    ap.add_argument("--width", type=int, default=8, choices=(4, 8))
    ap.add_argument("--part", default="unknown", help="target part name, for the header")
    ap.add_argument("--out", required=True,
                    help="output path: .bin (binary table, for the firmware) or .c")
    args = ap.parse_args()

    n = len(args.image)
    if n & (n - 1):
        sys.exit(f"{n} images: the bank select is binary, so give a power of two")

    images = [read_image(p, args.width, args.depth) for p in args.image]

    rows = []
    for path, words in zip(args.image, images):
        blob = bytes(words)
        rows.append((os.path.basename(path), zlib.crc32(blob) & 0xFFFFFFFF, words))

    if args.out.endswith(".bin"):
        blob = binary_table(args.part, args.depth, args.width, rows)
        with open(args.out, "wb") as fh:
            fh.write(blob)
        print(f"{args.out}: {n} image(s) of {args.depth}x{args.width}, "
              f"{len(blob)} bytes, part {args.part}")
        for idx, (src, crc, _) in enumerate(rows):
            print(f"  bank {idx}: {src} crc32 0x{crc:08x}")
        return

    name = os.path.splitext(os.path.basename(args.out))[0]
    out = [
        "/* Generated by tools/mkimagetable.py -- do not edit. */",
        f"/* target part: {args.part}   organisation: {args.depth}x{args.width}"
        f"   images: {n} */",
        "",
        '#include "fastprom_image.h"',
        "",
    ]

    for idx, (src, crc, words) in enumerate(rows):
        out.append(f"/* image {idx}: {src}  crc32 0x{crc:08x} */")
        out.append(f"static const uint8_t {name}_data_{idx}[{args.depth}] = {{")
        for start in range(0, args.depth, 16):
            chunk = words[start:start + 16]
            out.append("    " + " ".join(f"0x{w:02x}," for w in chunk))
        out.append("};")
        out.append("")

    out += [
        f"static const fastprom_image_t {name}_images[{n}] = {{",
    ]
    for idx, (src, crc, _) in enumerate(rows):
        out += [
            "    {",
            f'        .name  = "{src}",',
            f"        .crc32 = 0x{crc:08x}u,",
            f"        .data  = {name}_data_{idx},",
            "    },",
        ]
    out += ["};", ""]

    out += [
        f"const fastprom_image_table_t {name}_table = {{",
        f"    .magic      = 0x{MAGIC:08x}u,",
        f"    .version    = {FORMAT_VERSION}u,",
        f"    .part       = \"{args.part}\",",
        f"    .depth      = {args.depth}u,",
        f"    .width      = {args.width}u,",
        f"    .image_count = {n}u,",
        f"    .images     = {name}_images,",
        "};",
        "",
    ]

    with open(args.out, "w") as fh:
        fh.write("\n".join(out) + "\n")

    total = n * args.depth
    print(f"{args.out}: {n} image(s) of {args.depth}x{args.width}, "
          f"{total} bytes of flash, part {args.part}")
    for idx, (src, crc, _) in enumerate(rows):
        print(f"  bank {idx}: {src} crc32 0x{crc:08x}")


if __name__ == "__main__":
    main()
