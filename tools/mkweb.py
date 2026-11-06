#!/usr/bin/env python3
"""Assemble the update page's static files (docs/12-web.md).

Copies web/ (less its tests) into the output directory and writes
parts.json from tools/partdb.yaml: each part's organisation, which the page
checks ROM files against, and the chips it stands in for. The WebAssembly
(pkg/) and the update bundle (bundle/) are written beside them by `make web`.

usage: mkweb.py --out build/web
"""
import argparse
import json
import os
import shutil

import yaml

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SKIP = {"tests"}


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    src = os.path.join(REPO, "web")
    os.makedirs(args.out, exist_ok=True)
    for name in sorted(os.listdir(src)):
        if name in SKIP:
            continue
        s, d = os.path.join(src, name), os.path.join(args.out, name)
        if os.path.isdir(s):
            shutil.rmtree(d, ignore_errors=True)
            shutil.copytree(s, d)
        else:
            shutil.copyfile(s, d)
    with open(os.path.join(REPO, "tools", "partdb.yaml")) as fh:
        db = yaml.safe_load(fh)["parts"]
    parts = {
        key: {
            "depth": p["depth"],
            "width": p["width"],
            "chips": [str(c) for c in p.get("tristate_parts", []) + p.get("open_collector_parts", [])],
        }
        for key, p in db.items()
    }
    with open(os.path.join(args.out, "parts.json"), "w") as fh:
        json.dump(parts, fh, indent=1)
        fh.write("\n")
    print(f"{args.out}: page from web/, {len(parts)} parts from tools/partdb.yaml")


if __name__ == "__main__":
    main()
