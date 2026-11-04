#!/usr/bin/env python3
"""Generate ROM images for simulation and synthesis experiments.

Three flavours, because how compressible the contents are decides how many
LUT4s LOGIC mode needs:

  random     worst case for logic minimisation - an incompressible truth table
  palette    a plausible arcade colour-palette PROM (structured, repetitive)
  decode     a plausible address-decode PROM (very few distinct output words)
"""
import argparse
import random
import sys


def gen_random(depth, width, seed):
    rng = random.Random(seed)
    mask = (1 << width) - 1
    return [rng.getrandbits(width) & mask for _ in range(depth)]


def gen_palette(depth, width, seed):
    """RGB-ish palette: a handful of intensity levels replicated in blocks."""
    rng = random.Random(seed)
    levels = [0x00, 0x11, 0x33, 0x55, 0x77, 0x99, 0xBB, 0xDD, 0xFF]
    base = [rng.choice(levels) for _ in range(16)]
    mask = (1 << width) - 1
    return [base[i % 16] & mask for i in range(depth)]


def gen_decode(depth, width, seed):
    """Address decode: output is a function of the top few address bits only."""
    rng = random.Random(seed)
    mask = (1 << width) - 1
    regions = [rng.getrandbits(width) & mask for _ in range(8)]
    shift = max(depth.bit_length() - 4, 0)
    return [regions[(i >> shift) % 8] for i in range(depth)]


GENERATORS = {"random": gen_random, "palette": gen_palette, "decode": gen_decode}


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--kind", choices=sorted(GENERATORS), default="random")
    ap.add_argument("--depth", type=int, default=512)
    ap.add_argument("--width", type=int, default=8)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", required=True, help="output path (.hex or .bin)")
    args = ap.parse_args()

    if args.width not in (4, 8):
        sys.exit("width must be 4 or 8 (bipolar PROMs are x4 or x8)")

    words = GENERATORS[args.kind](args.depth, args.width, args.seed)
    digits = 1 if args.width == 4 else 2

    if args.out.endswith(".bin"):
        with open(args.out, "wb") as fh:
            fh.write(bytes(words))
    else:
        with open(args.out, "w") as fh:
            for w in words:
                fh.write(f"{w:0{digits}x}\n")

    print(f"{args.out}: {args.kind} {args.depth}x{args.width}, seed {args.seed}")


if __name__ == "__main__":
    main()
