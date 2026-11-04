#!/usr/bin/env python3
"""patch_af_map.py - fix the af_map.tcl that AGM's pre_logic.tcl generates
(AgRV_pio 1.6.6, yosys 0.47) so the native (no-Quartus) flow builds real designs.
Copied unchanged in effect from cosmic-toad (tools/ag32/patch_af_map.py), where
these fixes were found; see docs/06-build-and-flash.md for why each is needed.

1. Carry chains: the template runs yosys' generic techmap.v before AGM's
   arith_map.v, so every $alu is already lowered to LUT ripple logic and the
   AGRV2K carry chains are never used. Run the arithmetic map first.
2. The arithmetic map itself takes every requested carry-out from the middle of
   the chain (an extra cell on cout). Signed compares use CO[n-2] as well as
   CO[n-1], so a cout would drive two cins, which Supra cannot place or route.
   Use a copy (logic/arith_map_fp.v) in which only the last carry leaves the
   chain; intermediate carries are rebuilt in a LUT: CO[i] = X[i+1] ^ Y[i+1].
3. AGM's RAM models contain $display checks that yosys keeps as $print cells
   (and $scopeinfo cells); Supra's netlist reader cannot parse them.
4. A few $not/$mux/$logic_not cells survive the last LUT mapping and are
   written as Verilog expressions that Supra rejects; lower and LUT-map them.

usage: patch_af_map.py <AgRV_pio dir> logic/af_map.tcl
"""
import os
import sys

pio, p = sys.argv[1], sys.argv[2]
s = open(p).read()

# --- 2. arithmetic map with a single carry exit per chain
src = os.path.join(pio, "packages/tool-agrv_logic/map/share/agm/rodina/arith_map.v")
a = open(src).read()
old = """    cycloneive_lcell_comb #(.lut_mask(16'hF0F0), .sum_lutc_input("cin")) carry_output (
      .dataa  (1'b1    ),
      .datab  (1'b1    ),
      .datac  (1'b1    ),
      .datad  (1'b1    ),
      .cin    (COx[i+1]),
      .combout(CO[i]   ),
      .cout   (        )
    );"""
new = """    if (i == Y_WIDTH - 1) begin
      cycloneive_lcell_comb #(.lut_mask(16'hF0F0), .sum_lutc_input("cin")) carry_output (
        .dataa  (1'b1    ),
        .datab  (1'b1    ),
        .datac  (1'b1    ),
        .datad  (1'b1    ),
        .cin    (COx[i+1]),
        .combout(CO[i]   ),
        .cout   (        )
      );
    end else begin
      assign CO[i] = X[i+1] ^ Y[i+1];   // carry into bit i+1, without tapping the chain
    end"""
assert old in a, "arith_map.v: carry_output cell not as expected"
open(os.path.join(os.path.dirname(p), "arith_map_fp.v"), "w").write(a.replace(old, new))

# --- 1. arithmetic map before the generic one
generic = "    techmap -autoproc -map +/techmap.v\n"
arith = "    techmap -autoproc -map +/agm/rodina/arith_map.v\n"
assert generic + arith in s, "af_map.tcl: techmap order not as expected"
s = s.replace(generic + arith, "    techmap -autoproc -map arith_map_fp.v\n" + generic)

# --- 3, 4. before the final checks / netlist write
fix = ("    delete t:\\$print t:\\$scopeinfo\n"
       "    techmap\n"
       "    abc -lut 4\n"
       "    techmap -autoproc -map +/agm/rodina/cells_map.v\n"
       "    clean -purge\n")
assert "\n#check:" in s, "af_map.tcl: #check: section missing"
s = s.replace("\n#check:", "\n" + fix + "#check:", 1)
open(p, "w").write(s)
