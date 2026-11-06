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
                   images written by tools/mkimage.py (and one from binary
                   files with awkward names, written here), so the parser
                   is tested against the real tool's output, and
                   fastprom-host's builder against its bytes.
  tables.txt       what each table must contain, worked out here from the
                   source images (padding with blank words, zlib CRC, the
                   name as the tool stores it), not by reading the table
                   back; and each image's source file.
  tables/*.img     each image's expected contents, depth bytes.
  (refusals)       and the inputs mkimagetable.py must refuse - a part that
                   is not a tools/partdb.yaml key, a depth that is not the
                   part's, an image name with a control character, Intel
                   HEX - are run through it, and must fail with its message.
  headers.txt      application headers and fabric manifests written by
  headers/*.bin    tools/fw_stamp.py itself, with the fields they were
                   given, so the Rust decoder and encoder are tested against
                   the tool that stamps the firmware.

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
sys.path.insert(0, os.path.join(REPO, "tools"))
import fw_stamp  # noqa: E402  (tools/ is not a package)


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
    """The words in an image file, the way mkimagetable.py (and the page) read
    them: .hex, .txt and .mem in any case are text, anything else binary."""
    if not path.lower().endswith((".hex", ".txt", ".mem")):
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
# The part is a tools/partdb.yaml key, as the tool requires. A kind of "bin"
# is a binary source file written here, one word per byte (mkimage.py
# writes only text), under the file name given as its seed: names the table
# must store as mkimagetable.py does (non-ASCII as "?", cut to 39 bytes),
# and extensions it must read as binary as the page does (.BIN, .rom), for
# fastprom-host's byte-identity test.
TABLES = [
    ("t512x8_1", "512x8", 512, 8, [("palette", 1, None)]),
    ("t256x8_4", "256x8", 256, 8, [("random", 2, None), ("palette", 3, None),
                                    ("decode", 4, None), ("random", 5, 100)]),
    ("t4096x4_2", "4096x4", 4096, 4, [("random", 6, None), ("decode", 7, None)]),
    ("t2048x4_1", "2048x4", 2048, 4, [("palette", 8, 2000)]),
    ("tcase256x8_2", "256x8", 256, 8, [("bin", "DUMP.BIN", None), ("bin", "game.rom", 200)]),
    ("tbin512x8_2", "512x8", 512, 8,
     [("bin", "caf\u00e9-\u00fcber.bin", 500),
      ("bin", "a-very-long-rom-file-name-that-the-table-cuts-short.bin", None)]),
]


def refusals(outdir):
    """Inputs mkimagetable.py must refuse, each with its reason in the message."""
    rdir = os.path.join(outdir, "refusals")
    os.makedirs(rdir, exist_ok=True)
    good = os.path.join(rdir, "ok.bin")
    with open(good, "wb") as fh:
        fh.write(bytes(256))
    ctrl = os.path.join(rdir, "bad\x01name.bin")
    with open(ctrl, "wb") as fh:
        fh.write(bytes(256))
    ihex = os.path.join(rdir, "intel.hex")
    with open(ihex, "w") as fh:
        fh.write(":10000000000102030405060708090A0B0C0D0E0F78\n:00000001FF\n")
    cases = [
        (["--part", "82S129", "--image", good], "not in tools/partdb.yaml"),
        (["--part", "256x8", "--depth", "512", "--image", good], "part 256x8 is 256x8"),
        (["--part", "256x8", "--image", ctrl], "control characters"),
        (["--part", "256x8", "--image", ihex], "Intel HEX"),
    ]
    for args, why in cases:
        r = subprocess.run([sys.executable, "-I", MKTABLE, *args,
                            "--out", os.path.join(rdir, "t.bin")],
                           capture_output=True, text=True)
        if r.returncode == 0 or why not in r.stderr:
            sys.exit(f"mkimagetable.py {' '.join(args)}: expected a refusal saying "
                     f"{why!r}, got exit {r.returncode}: {r.stderr.strip()}")
    return len(cases)


def stored_name(name):
    """What mkimagetable.py's ascii_field keeps of a name."""
    return name.encode("ascii", errors="replace")[:39].decode("ascii")


def tables(outdir):
    tdir = os.path.join(outdir, "tables")
    os.makedirs(tdir, exist_ok=True)
    manifest = []
    for name, part, depth, width, images in TABLES:
        paths = []
        expect = []
        mask = (1 << width) - 1
        for i, (kind, seed, nwords) in enumerate(images):
            if kind == "bin":
                src = os.path.join(tdir, seed)
                rng = random.Random(f"{name}-{i}")
                with open(src, "wb") as fh:
                    fh.write(bytes(rng.getrandbits(width) for _ in range(nwords or depth)))
            else:
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
        subprocess.run([sys.executable, "-I", *cmd], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # The part as stored, and each image: its name as stored, its CRC,
        # and the source file it came from.
        manifest.append(f"table {name} {stored_name(part)[:27]} "
                        f"{depth} {width} {len(images)}")
        for i, (src, crc) in enumerate(expect):
            manifest.append(f"image {i} {stored_name(src)} {crc:08x} {src}")
    with open(os.path.join(outdir, "tables.txt"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(manifest) + "\n")
    return len(TABLES)


# kind, name, fields: (payload length, seed, version or part, board rev,
# fabric ID, committed)
HEADERS = [
    ("app", "app_small", (1, 20, (0, 1, 0), "A", 1, True)),
    ("app", "app_full", (48 * 1024 - 256, 21, (65535, 2, 300), "REV-B2", 7, True)),
    ("app", "app_pending", (5000, 22, (1, 0, 0), "ABCDEFGH", 1, False)),
    ("manifest", "man_512x8", (99944, 23, "512x8", "A", 1, True)),
    ("manifest", "man_pending", (100 * 1024, 24, "4096x4-logic-pal", "Z", 2, False)),
]


def headers(outdir):
    hdir = os.path.join(outdir, "headers")
    os.makedirs(hdir, exist_ok=True)
    lines = []
    for kind, name, (n, seed, what, rev, fid, commit) in HEADERS:
        rng = random.Random(seed)
        payload = bytes(rng.getrandbits(8) for _ in range(n))
        if kind == "app":
            page = fw_stamp.app_header(payload, what, rev, fid, commit)
            what = ".".join(str(v) for v in what)
        else:
            page = fw_stamp.manifest(payload, what, rev, fid, commit)
        with open(os.path.join(hdir, f"{name}.bin"), "wb") as fh:
            fh.write(page)
        lines.append(f"{kind} {name} {n} {crc32(payload):08x} {what} {rev} {fid} "
                     f"{int(commit)}")
    with open(os.path.join(outdir, "headers.txt"), "w") as fh:
        fh.write("\n".join(lines) + "\n")
    return len(HEADERS)


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
    r = refusals(outdir)
    h = headers(outdir)
    print(f"{outdir}: {len(cases)} CRC vectors from zlib.crc32, "
          f"{n} image tables from tools/mkimagetable.py and {r} inputs it refused, "
          f"{h} headers and manifests from tools/fw_stamp.py")


if __name__ == "__main__":
    main()
