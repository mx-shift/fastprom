#!/usr/bin/env python3
"""Check a DFU-fabric build (tools/ag32/build_dfu_fabric.sh) and write its
evidence and manifest.

The DFU stage's fabric must drive nothing but USB: in DFU mode the socket's
data pins and the bus-buffer enable (PIN_14, buf_noe, which R21 holds high
while nothing drives it) must be left alone. This proves it from Supra's own
output, not from the .ve we asked for:

  * the routed netlist (top_routed.v) has exactly the ports PIN_HSE, PIN_HSI
    and PIN_OSC, all inputs (the oscillator input on PIN_2 and the two
    internal oscillators), and its only pad cells (alta_rio) are theirs,
    with output enable and data tied to ground;
  * Supra's IO assignments (logic_db/io.asf) place only those three;
  * the packer reports 3 pins used, and the run has no fatal, error or
    warning;
  * every connection the timing constraints do not reach is clock network
    (dfu_usb_pre.tcl says why the coverage limit is off for this design).

USB's D+ and D- (PIN_23, PIN_22) are the USB block's own pads, which the
fabric enables (USB0 in the .ve) but does not route: they appear in neither
list. Every other package pin has no pad cell in the design at all: unused,
undriven, as before configuration.

Then the bitstreams: the compressed one (logic/top.bin) must start with the
AGRV2K's IDCODE and expand - by the same algorithm as AGM's fcb.c, in
Python here, in Rust in fastprom-core's fcb.rs - to exactly the plain one
Supra wrote from the same routing (logic/top_raw.bin, dfu_usb_post.tcl).

usage: check_dfu_fabric.py <work dir> [--sdk <AgRV_pio dir>]
Prints the evidence; writes <work>/manifest; exits non-zero on any failure.
"""
import argparse
import gzip
import os
import re
import sys
import zlib

# fcb.h FCB_AUTO_WORDS * 4: every AGRV2K bitstream.
CONFIG_LEN = 99_944
# The AGRV2K device ID (boards/agrv2k_*.json "device_id"), which leads every
# bitstream as its IDCODE, big-endian in the file as agrv2k.cfg checks it.
IDCODE = 0x40200001
PORTS = {"PIN_HSE", "PIN_HSI", "PIN_OSC"}
# Clock-network cells: the PLL, its global-clock generators and buffers,
# and the HSI/HSE clock switch.
CLOCK_NET = re.compile(r"^(pll_inst|PLL_CLKOUT\[\d\]_gclkgen|gclksw_inst|gclksw_gen|"
                       r"usb0_xcvr_clk_gclk|PIN_HS[EI]_iobuf)\|")
INPUTS = ["dfu_usb.ve", "usb0.asf", "dfu_usb.sdc", "dfu_usb_pre.tcl", "dfu_usb_post.tcl"]


def lzw_expand(src):
    """fcb.c FCB_AutoDecompressEncrypt, unencrypted: the configuration bytes."""
    out = bytearray(src[:8])
    width, slot, prev, prev_first, acc, bits = 9, 3, None, 0, 0, 0
    prefix, last = [0] * 1024, [0] * 1024
    for b in src[8:]:
        acc = ((acc << 8) | b) & 0xFFFFFFFF
        bits += 8
        while bits >= width:
            bits -= width
            code = (acc >> bits) & ((1 << width) - 1)
            if code == 256:
                width, slot, prev = 9, 3, None
                continue
            if code == 257:
                raise ValueError(f"STOP after {len(out)} bytes")
            if code > slot + 255 or (code > 255 and prev is None):
                raise ValueError(f"undefined code {code}")
            if code == slot + 255:
                prefix[slot], last[slot] = prev, prev_first
            s, c = [], code
            while c > 255:
                s.append(last[c - 255])
                c = prefix[c - 255]
            s.append(c)
            prev_first = c
            if prev is not None:
                prefix[slot], last[slot] = prev, c
                slot += 1
                if slot + 255 > (1 << width) - 1:
                    width += 1
            prev = code
            for x in reversed(s):
                out.append(x)
                if len(out) == CONFIG_LEN:
                    return bytes(out)
    raise ValueError(f"truncated after {len(out)} bytes")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("work")
    ap.add_argument("--sdk", help="the AgRV_pio dir, recorded in the evidence")
    args = ap.parse_args()
    w = args.work
    logic = os.path.join(w, "logic")
    bad = []
    say = print

    def read(*p):
        with open(os.path.join(*p), errors="replace") as fh:
            return fh.read()

    say("DFU fabric: pin and bitstream evidence (tools/ag32/check_dfu_fabric.py)")
    if args.sdk:
        say(f"  SDK {args.sdk}")

    # ---- what we asked for: no pin lines -----------------------------------
    ve = [ln.split("#")[0].split() for ln in read(w, "dfu_usb.ve").splitlines()]
    ve = [ln for ln in ve if ln]
    keys = sorted({ln[0] for ln in ve})
    say(f"  .ve keys: {' '.join(keys)}")
    if set(keys) - {"SYSCLK", "BUSCLK", "HSECLK", "USB0"}:
        bad.append(f".ve has more than clocks and USB0: {keys}")

    for d in ("logic",):
        # ---- Supra's routed netlist: the ports and the pad cells ----------
        net = read(w, d, "top_routed.v")
        m = re.search(r"^module top\((.*?)\);", net, re.S | re.M)
        ports = {p.strip() for p in m[1].split(",")} if m else set()
        decl = dict((n, k) for k, n in re.findall(r"^(input|output|inout)\s+(\S+);", net, re.M))
        rio = re.findall(r"^alta_rio (\S+)\((.*?)\);", net, re.S | re.M)
        if ports != PORTS:
            bad.append(f"{d}: netlist ports {sorted(ports)}, want {sorted(PORTS)}")
        if any(decl.get(p) != "input" for p in ports):
            bad.append(f"{d}: a port is not an input: {decl}")
        names = sorted(n for n, _ in rio)
        if names != sorted(f"{p}_iobuf" for p in PORTS):
            bad.append(f"{d}: pad cells {names}")
        for n, body in rio:
            if ".oe(gnd)" not in body or ".datain(gnd)" not in body:
                bad.append(f"{d}: {n} may drive its pad")
        # ---- Supra's IO placement ------------------------------------------
        io = re.findall(r"set_location_assignment -to (\S+) (\S+)", read(w, d, "logic_db", "io.asf"))
        if {p for p, _ in io} != PORTS or any(p != loc for p, loc in io):
            bad.append(f"{d}: io.asf places {io}")
        # ---- the run itself --------------------------------------------------
        con = read(w, d, "supra_console.log")
        pins = re.search(r"Total\s+Pins : (\d+)/\s*(\d+)", con)
        total = re.search(r"Total (\d+) fatals, (\d+) errors, (\d+) warnings", con)
        if not pins or pins[1] != "3":
            bad.append(f"{d}: packer pin count {pins[0] if pins else 'missing'}")
        if not total or total.groups() != ("0", "0", "0"):
            bad.append(f"{d}: Supra: {total[0] if total else 'no summary'}")
        with gzip.open(os.path.join(w, d, "logic_db", "coverage.rpt.gz"), "rt") as fh:
            unc = re.findall(r"Uncovered connection: (\S+) => (\S+)", fh.read())
        stray = [c for c in unc if not (CLOCK_NET.match(c[0]) and CLOCK_NET.match(c[1]))]
        if stray:
            bad.append(f"{d}: uncovered connections outside the clock network: {stray}")
        say(f"  {d}: ports {', '.join(sorted(ports))} ({', '.join(decl.get(p, '?') for p in sorted(ports))}); "
            f"pad cells {len(rio)}, oe and data tied low; io.asf places {len(io)}, each on itself")
        say(f"  {d}: packer '{pins[0] if pins else '?'}'; Supra '{total[0] if total else '?'}'; "
            f"{len(unc)} unconstrained connections, all clock network: "
            + "; ".join(f"{a} => {b}" for a, b in unc))

    # ---- the bitstreams --------------------------------------------------------
    comp = open(os.path.join(logic, "top.bin"), "rb").read()
    raw = open(os.path.join(logic, "top_raw.bin"), "rb").read()
    if len(raw) != CONFIG_LEN:
        bad.append(f"plain bitstream is {len(raw)} bytes, not {CONFIG_LEN}")
    if int.from_bytes(comp[:4], "big") != IDCODE:
        bad.append(f"compressed bitstream IDCODE {comp[:4].hex()}")
    try:
        expanded = lzw_expand(comp)
    except ValueError as e:
        bad.append(f"compressed bitstream does not expand: {e}")
        expanded = b""
    if expanded != raw:
        bad.append("compressed bitstream does not expand to the plain one")
    say(f"  bitstream: {len(comp)} bytes compressed, expands to {len(expanded)} bytes, "
        f"{'identical to' if expanded == raw else 'DIFFERENT from'} Supra's plain {len(raw)}-byte one "
        f"(CRC-32 {zlib.crc32(raw):#010x})")

    inputs = b"".join(open(os.path.join(w, "logic", n) if n != "dfu_usb.ve"
                           else os.path.join(w, n), "rb").read() for n in INPUTS)
    with open(os.path.join(w, "manifest"), "w") as fh:
        fh.write("# The DFU stage's USB-only fabric, dfu_usb.bin beside this file: written by\n"
                 "# tools/ag32/build_dfu_fabric.sh --install, checked by fastprom-dfu's build.rs\n"
                 "# and fastprom-core's tests/fcb.rs. Do not edit.\n")
        fh.write(f"bin_len {len(comp)}\n")
        fh.write(f"bin_crc32 {zlib.crc32(comp):#010x}\n")
        fh.write(f"raw_len {len(raw)}\n")
        fh.write(f"raw_crc32 {zlib.crc32(raw):#010x}\n")
        fh.write(f"# CRC-32 of {', '.join(INPUTS)}, concatenated\n")
        fh.write(f"inputs_crc32 {zlib.crc32(inputs):#010x}\n")

    if bad:
        for b in bad:
            say(f"FAIL: {b}")
        sys.exit(1)
    say("  PASS: no package pin but the clock input has a pad cell; nothing drives the socket "
        "or buf_noe")


if __name__ == "__main__":
    main()
