#!/usr/bin/env python3
"""Test fixtures for the firmware's host tests, from independent producers.

The firmware must agree exactly with the tools that write its inputs, or it
rejects every image at boot and the socket stays dead. So the expected values
here never come from the Rust code under test:

  crc_vectors.txt  CRC-32 vectors from Python's zlib.crc32. The first nine
                   are the C firmware's vectors (same seed, same cases, from
                   the retired fw/test/gen_vectors.py); the rest add longer
                   and structured inputs.
  tables/*.bin     image tables written by tools/mkimagetable.py itself, from
                   images written by tools/mkimage.py, so the parser is
                   tested against the real tool's output.
  tables.txt       what each table must contain, worked out here from the
                   source images (padding with blank words, zlib CRC), not
                   by reading the table back.
  tables/*.img     each image's expected contents, depth bytes.

Usage: gen_fixtures.py <output dir>   (make fw-fixtures)
"""
import os
import random
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
MKIMAGE = os.path.join(REPO, "tools", "mkimage.py")
MKTABLE = os.path.join(REPO, "tools", "mkimagetable.py")


def crc32(data):
    return zlib.crc32(data) & 0xFFFFFFFF


def crc_vectors():
    # The C firmware's vectors, unchanged (fw/test/gen_vectors.py).
    rng = random.Random(20260912)
    cases = [
        ("empty", b""),
        ("one_zero", b"\x00"),
        ("one_ff", b"\xff"),
        ("ascii", b"FastPROM"),
    ]
    for n in (16, 256, 512, 2048):
        cases.append((f"rand{n}", bytes(rng.getrandbits(8) for _ in range(n))))
    # An unprogrammed bipolar PROM reads as all ones.
    cases.append(("all_ff_512", b"\xff" * 512))

    # Additions: the largest image the table holds, every byte value, the
    # standard check string, an all-zero image and 4-bit data.
    rng = random.Random(20261104)
    cases.append(("rand65536", bytes(rng.getrandbits(8) for _ in range(65536))))
    cases.append(("bytes_0_255", bytes(range(256))))
    cases.append(("check_123456789", b"123456789"))
    cases.append(("all_zero_4096", b"\x00" * 4096))
    cases.append(("nibbles_4096", bytes(rng.getrandbits(4) for _ in range(4096))))
    for n in range(1, 9):  # every length that does not fill a word
        cases.append((f"short{n}", bytes(rng.getrandbits(8) for _ in range(n))))
    return cases


def run(*args):
    subprocess.run([sys.executable, "-I", *args], check=True,
                   stdout=subprocess.DEVNULL)


def read_words(path):
    """The words in an image file, the way mkimagetable.py reads them."""
    if path.endswith(".bin"):
        with open(path, "rb") as fh:
            return list(fh.read())
    words = []
    with open(path) as fh:
        for raw in fh:
            line = raw.split("//")[0].strip()
            if line:
                words.extend(int(t, 16) for t in line.split())
    return words


# name, part, depth, width, [(kind, seed, words or None for full depth)]
TABLES = [
    ("t512x8_1", "82S147", 512, 8, [("palette", 1, None)]),
    ("t256x8_4", "82S129", 256, 8, [("random", 2, None), ("palette", 3, None),
                                     ("decode", 4, None), ("random", 5, 100)]),
    ("t4096x4_2", "82S185", 4096, 4, [("random", 6, None), ("decode", 7, None)]),
    ("t2048x4_1", "82S137", 2048, 4, [("palette", 8, 2000)]),
    ("t16x8_2", "TINY", 16, 8, [("random", 9, None), ("decode", 10, None)]),
]


def tables(outdir):
    tdir = os.path.join(outdir, "tables")
    os.makedirs(tdir, exist_ok=True)
    manifest = []
    for name, part, depth, width, images in TABLES:
        paths = []
        expect = []
        mask = (1 << width) - 1
        for i, (kind, seed, nwords) in enumerate(images):
            src = os.path.join(tdir, f"{name}_src{i}.hex")
            run(MKIMAGE, "--kind", kind, "--depth", str(nwords or depth),
                "--width", str(width), "--seed", str(seed), "--out", src)
            words = read_words(src)
            # Unprogrammed bipolar PROM cells read as 1s.
            words += [mask] * (depth - len(words))
            data = bytes(words)
            with open(os.path.join(tdir, f"{name}_{i}.img"), "wb") as fh:
                fh.write(data)
            paths.append(src)
            expect.append((os.path.basename(src), crc32(data)))
        out = os.path.join(tdir, f"{name}.bin")
        cmd = [MKTABLE, "--depth", str(depth), "--width", str(width),
               "--part", part, "--out", out]
        for p in paths:
            cmd += ["--image", p]
        run(*cmd)
        manifest.append(f"table {name} {part} {depth} {width} {len(images)}")
        for i, (src, crc) in enumerate(expect):
            manifest.append(f"image {i} {src} {crc:08x}")
    with open(os.path.join(outdir, "tables.txt"), "w") as fh:
        fh.write("\n".join(manifest) + "\n")
    return len(TABLES)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    outdir = sys.argv[1]
    os.makedirs(outdir, exist_ok=True)

    cases = crc_vectors()
    lines = ["# name length crc32 data(hex) -- from zlib.crc32, gen_fixtures.py"]
    for name, data in cases:
        lines.append(f"{name} {len(data)} {crc32(data):08x} {data.hex() or '-'}")
    with open(os.path.join(outdir, "crc_vectors.txt"), "w") as fh:
        fh.write("\n".join(lines) + "\n")

    n = tables(outdir)
    print(f"{outdir}: {len(cases)} CRC vectors from zlib.crc32, "
          f"{n} image tables from tools/mkimagetable.py")


if __name__ == "__main__":
    main()
