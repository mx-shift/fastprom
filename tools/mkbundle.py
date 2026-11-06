#!/usr/bin/env python3
"""Write the update bundle the web page offers (docs/12-web.md).

A directory beside the page:

  manifest.json             versions, board revision, part, sizes, CRCs
  fastprom-fw-<ver>.bin     the application, header stamped (make fw-build)
  fabric-<part>.bin         the BRAM-mode fabric bitstream for each part
  fabric-<part>.manifest.bin  its manifest page (tools/fw_stamp.py manifest)
  fastprom-dfu-<ver>.bin    the DFU stage (make fw-build), for UART recovery
  fastprom-recovery-<ver>.bin  the recovery loader (make fw-build), which the
                            page uploads into SRAM through the mask ROM
  option-bytes.bin          the 128 option bytes recovery writes
                            (fw_stamp.option_bytes, gen_batch's layout)

USB never writes the DFU stage or the option bytes (docs/10-updates.md, "Flash
map"): those three are the page's UART recovery's only ("Recovery"). A part
whose bitstream is not in build/ag32 (it needs AGM's SDK,
tools/ag32/run_flow.sh) is left out, with a note in the manifest saying so.
fastprom-host's bundle.rs reads the manifest and checks every file against it
and against its own header before anything is written.

usage: mkbundle.py --app build/fw/fastprom-ag32.bin --out build/web/bundle
                   [--fabric-dir build/ag32]
                   [--dfu build/fw/fastprom-dfu.bin --loader build/fw/fastprom-recovery.bin]
       mkbundle.py --empty --out DIR   (no firmware built: a manifest saying so)
       mkbundle.py --synthetic --version 0.2.0 --board-rev A --out DIR
                   (a bundle of made-up payloads, for the page's tests)
"""
import argparse
import json
import os
import random
import shutil
import sys

import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fw_stamp  # noqa: E402  (tools/ is not a package)

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
FORMAT = 1


def hex32(v):
    return f"0x{v:08x}"


def firmware_entry(name, data):
    h = fw_stamp.decode_app_header(data[:fw_stamp.PAGE])
    if not h["committed"]:
        sys.exit(f"{name}: the header's commit word is not set: not a stamped build")
    if len(data) < fw_stamp.PAGE + h["length"] or len(data) > fw_stamp.APP_LEN:
        sys.exit(f"{name}: {len(data)} bytes for a header of {h['length']}")
    if fw_stamp.crc32(data[fw_stamp.PAGE:fw_stamp.PAGE + h["length"]]) != h["crc"]:
        sys.exit(f"{name}: the image does not match its header's CRC: run make fw-build")
    return h, {
        "file": name,
        "version": h["version"],
        "board_rev": h["board_rev"],
        "fabric_id": h["fabric_id"],
        "size": len(data),
        "crc32": hex32(fw_stamp.crc32(data)),
        "image_len": h["length"],
        "image_crc32": hex32(h["crc"]),
    }


def fabric_entry(out, part, bitstream, rev, fabric_id):
    name = f"fabric-{part}.bin"
    page = fw_stamp.manifest(bitstream, part, rev, fabric_id)
    with open(os.path.join(out, name), "wb") as fh:
        fh.write(bitstream)
    with open(os.path.join(out, f"fabric-{part}.manifest.bin"), "wb") as fh:
        fh.write(page)
    return {
        "part": part,
        "file": name,
        "manifest": f"fabric-{part}.manifest.bin",
        "board_rev": rev,
        "fabric_id": fabric_id,
        "size": len(bitstream),
        "crc32": hex32(fw_stamp.crc32(bitstream)),
    }


def recovery_entries(out, dfu, loader, rev):
    """The recovery set: the DFU stage, the loader, the option bytes."""
    h = fw_stamp.decode_loader(loader)
    if h["board_rev"] != rev:
        sys.exit(f"the recovery loader is for board {h['board_rev']}, the firmware for {rev}")
    if not 1 <= len(dfu) <= 32 * 1024:
        sys.exit(f"the DFU stage is {len(dfu)} bytes, not 1 to 32 KiB")
    names = {
        "loader": (f"fastprom-recovery-{h['version']}.bin", loader),
        "dfu": (f"fastprom-dfu-{h['version']}.bin", dfu),
        "option_bytes": ("option-bytes.bin", fw_stamp.option_bytes()),
    }
    entries = {}
    for key, (name, data) in names.items():
        with open(os.path.join(out, name), "wb") as fh:
            fh.write(data)
        e = {"file": name, "size": len(data), "crc32": hex32(fw_stamp.crc32(data))}
        if key != "option_bytes":
            e.update(version=h["version"], board_rev=rev)
        entries[key] = e
    return entries


def write(out, manifest):
    with open(os.path.join(out, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=2)
        fh.write("\n")
    fw = manifest["firmware"]
    print(f"{out}: firmware {fw['version']} for board {manifest['board_rev']}, "
          f"fabric for {', '.join(f['part'] for f in manifest['fabrics']) or 'no part'}, "
          f"{'the recovery set' if manifest.get('recovery') else 'no recovery set'}")
    for n in manifest["notes"]:
        print(f"  note: {n}")


def real(args):
    with open(args.app, "rb") as fh:
        data = fh.read()
    h, _ = firmware_entry(os.path.basename(args.app), data)
    name = f"fastprom-fw-{h['version']}.bin"
    _, fw = firmware_entry(name, data)
    rev = h["board_rev"]
    if rev != fw_stamp.board_rev():
        sys.exit(f"{args.app} is for board {rev}, hw/REVISION says {fw_stamp.board_rev()}")
    if os.path.isdir(args.out):
        shutil.rmtree(args.out)
    os.makedirs(args.out)
    with open(os.path.join(args.out, name), "wb") as fh:
        fh.write(data)
    with open(os.path.join(REPO, "tools", "partdb.yaml")) as fh:
        parts = yaml.safe_load(fh)["parts"]
    fabrics, notes = [], ["The DFU stage is not updated over USB: only by UART recovery (J3)."]
    for part in parts:
        path = os.path.join(args.fabric_dir, f"{part}-bram", "logic", "top.bin")
        if not os.path.isfile(path):
            notes.append(f"No fabric for {part}: {os.path.relpath(path)} was not built "
                         "(tools/ag32/run_flow.sh, AGM's SDK).")
            continue
        with open(path, "rb") as fh:
            fabrics.append(fabric_entry(args.out, part, fh.read(), rev, h["fabric_id"]))
    manifest = {"format": FORMAT, "board_rev": rev, "firmware": fw, "fabrics": fabrics}
    if args.dfu and args.loader:
        with open(args.dfu, "rb") as fh:
            dfu = fh.read()
        with open(args.loader, "rb") as fh:
            loader = fh.read()
        manifest["recovery"] = recovery_entries(args.out, dfu, loader, rev)
    else:
        notes.append("No recovery set (--dfu and --loader): the page cannot recover a board.")
    manifest["notes"] = notes
    write(args.out, manifest)


def synthetic(args):
    """Made-up payloads, well formed, for the page's tests."""
    version = tuple(int(v) for v in args.version.split("."))
    rng = random.Random(f"{args.version}-{args.board_rev}")
    code = bytes(rng.getrandbits(8) for _ in range(9000))
    data = fw_stamp.app_header(code, version, args.board_rev, fw_stamp.FABRIC_ID) + code
    name = f"fastprom-fw-{args.version}.bin"
    _, fw = firmware_entry(name, data)
    if os.path.isdir(args.out):
        shutil.rmtree(args.out)
    os.makedirs(args.out)
    with open(os.path.join(args.out, name), "wb") as fh:
        fh.write(data)
    bitstream = bytes(rng.getrandbits(8) for _ in range(20000))
    fabrics = [fabric_entry(args.out, "512x8", bitstream, args.board_rev, fw_stamp.FABRIC_ID)]
    lcode = bytes(rng.getrandbits(8) for _ in range(3000))
    loader = fw_stamp.loader_header(lcode, version, args.board_rev) + lcode
    dfu = bytes(rng.getrandbits(8) for _ in range(12000))
    write(args.out, {"format": FORMAT, "board_rev": args.board_rev, "firmware": fw,
                     "fabrics": fabrics,
                     "recovery": recovery_entries(args.out, dfu, loader, args.board_rev),
                     "notes": ["A synthetic bundle for testing: it is not FastPROM firmware."]})


def empty(args):
    """No firmware built: a manifest that says so, so the page has one."""
    if os.path.isdir(args.out):
        shutil.rmtree(args.out)
    os.makedirs(args.out)
    with open(os.path.join(args.out, "manifest.json"), "w") as fh:
        json.dump({"format": FORMAT, "board_rev": fw_stamp.board_rev(), "firmware": None,
                   "fabrics": [], "notes": ["This build of the page carries no firmware: "
                                            "it was built without make fw-build."]}, fh, indent=2)
        fh.write("\n")
    print(f"{args.out}: an empty bundle (no build/fw/fastprom-ag32.bin)")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--app", help="the stamped application binary")
    ap.add_argument("--fabric-dir", default=os.path.join(REPO, "build", "ag32"))
    ap.add_argument("--dfu", help="the DFU stage binary, for the recovery set")
    ap.add_argument("--loader", help="the stamped recovery loader binary, for the recovery set")
    ap.add_argument("--synthetic", action="store_true")
    ap.add_argument("--empty", action="store_true", help="a bundle with nothing in it")
    ap.add_argument("--version", default="0.2.0")
    ap.add_argument("--board-rev", default="A")
    args = ap.parse_args()
    if args.empty:
        empty(args)
    elif args.synthetic:
        synthetic(args)
    elif args.app:
        real(args)
    else:
        sys.exit("give --app, or --synthetic")


if __name__ == "__main__":
    main()
