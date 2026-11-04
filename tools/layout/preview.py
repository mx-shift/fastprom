#!/usr/bin/env python3
"""Render a board's layout dump (build/layout/<board>.json, written by
gen_layout.py) as PNGs, one per side: copper on that side plus In2, pads,
courtyards and vias, so placement and routing can be looked at without the
GUI. Needs Pillow (present in the KiCad container):

    scripts/kicad.sh py tools/layout/preview.py build/layout/<board>.json
"""
import json
import os
import sys

from PIL import Image, ImageDraw, ImageFont

S = 60                      # pixels per mm
PAD = 1.0                   # mm margin
COL = {"F": (200, 40, 40), "B": (40, 90, 220), "I": (220, 170, 0)}


def render(d, side, path, crop=None, scale=S, layers=None):
    """crop: (x0, y0, x1, y1) in mm to draw just that window, larger.
    layers: which track layers to draw ("F", "I", "B"); default all."""
    rects = d["outline_rects"]
    if crop:
        x0, y0, x1, y1 = crop
    else:
        x0 = min(r[0] for r in rects) - PAD
        y0 = min(r[1] for r in rects) - PAD
        x1 = max(r[2] for r in rects) + PAD
        y1 = max(r[3] for r in rects) + PAD
    S = scale
    mirror = side == "B"           # seen from below, as the board is held

    def P(x, y):
        if mirror:
            x = x0 + x1 - x
        return ((x - x0) * S, (y - y0) * S)

    W, H = int((x1 - x0) * S), int((y1 - y0) * S)
    im = Image.new("RGB", (W, H), (250, 250, 245))
    dr = ImageDraw.Draw(im, "RGBA")
    try:
        font = ImageFont.truetype("DejaVuSans.ttf", 14)
        small = ImageFont.truetype("DejaVuSans.ttf", 11)
    except OSError:
        font = small = ImageFont.load_default()
    # a millimetre grid, labelled every mm when zoomed in
    step = 1.0
    gx = int(x0)
    while gx <= x1:
        q = P(gx, y0)
        dr.line([(q[0], 0), (q[0], H)], fill=(225, 225, 225), width=1)
        if crop:
            dr.text((q[0] + 2, 2), f"{gx:g}", fill=(120, 120, 120), font=font)
        gx += step
    gy = int(y0)
    while gy <= y1:
        q = P(x0, gy)
        dr.line([(0, q[1]), (W, q[1])], fill=(225, 225, 225), width=1)
        if crop:
            dr.text((2, q[1] + 2), f"{gy:g}", fill=(120, 120, 120), font=font)
        gy += step
    for r in rects:
        a, b = P(r[0], r[1]), P(r[2], r[3])
        dr.rectangle([min(a[0], b[0]), a[1], max(a[0], b[0]), b[1]],
                     outline=(0, 0, 0), width=2)
    # the other side's copper faintly, then In2, then this side
    order = [("B" if side == "F" else "F", 40), ("I", 110), (side, 230)]
    for layer, alpha in order:
        if layers and layer not in layers:
            continue
        for (ax, ay, bx, by, w, L, net) in d["tracks"]:
            if L != layer:
                continue
            c = COL[L] + (alpha,)
            dr.line([P(ax, ay), P(bx, by)], fill=c, width=max(1, int(w * S)))
            r = w * S / 2
            for (qx, qy) in ((ax, ay), (bx, by)):
                q = P(qx, qy)
                dr.ellipse([q[0] - r, q[1] - r, q[0] + r, q[1] + r], fill=c)
    for p in d["pads"]:
        if side not in p["layers"]:
            continue
        a, b = P(p["bbox"][0], p["bbox"][1]), P(p["bbox"][2], p["bbox"][3])
        box = [min(a[0], b[0]), a[1], max(a[0], b[0]), b[1]]
        c = (90, 90, 90, 170) if p["net"] != "GND" else (60, 140, 60, 170)
        if p["round"]:
            dr.ellipse(box, fill=c)
        else:
            dr.rectangle(box, fill=c)
        if crop and p["net"]:
            nm = p["net"].lstrip("/").replace("Net-(", "").rstrip(")")
            dr.text((box[0] + 1, box[1] + 1), nm, fill=(0, 0, 0), font=small)
    for (vx, vy, net) in d["vias"]:
        q = P(vx, vy)
        r = 0.225 * S
        c = (60, 140, 60) if net == "GND" else (0, 0, 0)
        dr.ellipse([q[0] - r, q[1] - r, q[0] + r, q[1] + r], outline=c, width=3)
    for f in d["fps"]:
        if f["side"] != side:
            continue
        a, b = P(f["bbox"][0], f["bbox"][1]), P(f["bbox"][2], f["bbox"][3])
        dr.rectangle([min(a[0], b[0]), a[1], max(a[0], b[0]), b[1]],
                     outline=(150, 0, 150, 200), width=1)
        q = P(*f["at"])
        dr.text((q[0] + 3, q[1] + 3), f["ref"], fill=(150, 0, 150), font=font)
    im.save(path)


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("--crop", help="x0,y0,x1,y1 in mm")
    ap.add_argument("--scale", type=float, default=S)
    ap.add_argument("--side", choices=("F", "B"))
    ap.add_argument("--layers", help="track layers to draw, e.g. FI")
    args = ap.parse_args()
    with open(args.dump) as fh:
        d = json.load(fh)
    base = os.path.splitext(args.dump)[0]
    crop = tuple(float(v) for v in args.crop.split(",")) if args.crop else None
    for side in ((args.side,) if args.side else ("F", "B")):
        tag = f"-{side}" + ("-crop" if crop else "") + \
            (f"-{args.layers}" if args.layers else "")
        out = f"{base}{tag}.png"
        render(d, side, out, crop, args.scale, args.layers)
        print(out)


if __name__ == "__main__":
    main()
