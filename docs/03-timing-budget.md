# Timing budget

Computed by `tools/timing_budget.py` for the as-built two-board stack
(`fastprom-lower` + `fastprom-upper`). Re-run it after changing any
assumption rather than editing this file; re-run
`tools/ag32/run_matrix.sh` (`make pnr`) after any pin or HDL change.

All figures are **maxima**. There are two sources for the fabric's
share, side by side:

* **model**: datasheet primitives (AGRV2K Datasheet Rev 3.0 section 4,
  see `docs/00-ag32-reference.md`) times the LUT depth that
  `tools/synth_matrix.py` measures;
* **P&R**: AGM Supra's static timing after placing and routing the real
  design on the QFN-32 with the board's actual pin assignment
  (`tools/ag32/run_flow.sh`, recipe in `docs/06-build-and-flash.md`).

The P&R figures supersede the model wherever both exist. Neither is
silicon: the primitives are AGM boilerplate with no stated PVT corner,
and Supra's delay model is AGM's too. Both want confirming on hardware.

## The path through the stack

```
socket pin -> [lower] U2/U3 LVT245 -> DF40 -> [upper] AG32 pad
   ... fabric ...
AG32 pad -> DF40 -> [lower] U4 LVT245 -> 33 R (RN1/RN2) -> socket pin
AG32 PIN_14 (buf_noe) -> DF40 -> [lower] U4 /OE (R21 pull-up)
```

| External term | Value | Basis |
|---|---:|---|
| Receive buffer tpd (SN74LVT245B) | 3.5 ns | datasheet max, 3.3 V +/- 0.3 V, CL 50 pF |
| Drive buffer tpd | 3.5 ns | same part |
| Drive buffer enable, max(tPZH, tPZL) | 5.5 ns | |
| Drive buffer disable, max(tPHZ, tPLZ) | 5.9 ns | |
| Wiring, each way | 0.47 ns | 60 mm at 7.0 ps/mm + DF40 0.05 ns; longest routed signal is 38.3 mm |
| 33 R terminator into the host | 1.14 ns | R x 50 pF x ln 2, on top of a tpd already rated into 50 pF |
| **Socket to AG32 pad** | **3.97 ns** | |
| **AG32 pad to socket** | **5.11 ns** | |

I/O rated to 5.5 V, IOL 64 mA, IOH -32 mA, no bus hold. The two buffer crossings and the terminator are
8.1 ns of every access, and on the enable path
the drive buffer's 5.5 ns enable replaces its tpd.

The enable path has one LUT of decode and leaves the AG32 through
`buf_noe`, an ordinary always-driven pad, so the pad's own
output-enable time (T_ZX) is not in it: on the stack the AG32's data
pads drive unconditionally and the drive buffer alone gates the bus.

## Place and route

Source: `tools/ag32/pnr-results.json`, generated 2026-11-05 with AGM Supra 2024.11.b0(2d866f99)
by tools/ag32/run_matrix.sh (AgRV_pio native flow, device AGRV2KQ32). Pin map checked against:

| Socket pin | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | /OE |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| AG32 | 8 | 3 | 1 | 5 | 7 | 19 | 18 | 15 | 31 | 29 | 26 | 27 | 28 | 9 | 10 | 11 | 12 | 13 | 14 |

### Against each part's grades

Fabric delay is pad to pad as Supra reports it (worst data pin);
socket delay adds the external path above. `yes` is 20 % or more
margin, `tight` under that, **NO** does not fit.

#### 512x8

| Build | Parameter | Fabric (P&R) | At the socket | fastest (30/20) | common (45/30) | slowest (75/40) |
|---|---|---:|---:|---|---|---|
| **BRAM, read port 200 MHz** | tAA | 18.4 ns | 27.5 ns | tight +2.5 ns | yes +17.5 ns | yes +47.5 ns |
| | tEA | 7.1 ns | 18.2 ns | tight +1.8 ns | yes +11.8 ns | yes +21.8 ns |
| | tER | 7.1 ns | 17.5 ns | tight +2.5 ns | yes +12.5 ns | yes +17.5 ns |
| BRAM, read on sys_clock 150 MHz | tAA | 20.0 ns | 29.1 ns | tight +0.9 ns | yes +15.9 ns | yes +45.9 ns |
| | tEA | 7.1 ns | 18.2 ns | tight +1.8 ns | yes +11.8 ns | yes +21.8 ns |
| | tER | 7.1 ns | 17.5 ns | tight +2.5 ns | yes +12.5 ns | yes +17.5 ns |
| BRAM, read on sys_clock 100 MHz | tAA | 23.4 ns | 32.5 ns | **NO** -2.5 ns | yes +12.5 ns | yes +42.5 ns |
| | tEA | 7.1 ns | 18.2 ns | tight +1.8 ns | yes +11.8 ns | yes +21.8 ns |
| | tER | 7.1 ns | 17.5 ns | tight +2.5 ns | yes +12.5 ns | yes +17.5 ns |
| LOGIC (fallback), incompressible | tAA | 16.0 ns | 25.1 ns | tight +4.9 ns | yes +19.9 ns | yes +49.9 ns |
| | tEA | 7.3 ns | 18.4 ns | tight +1.6 ns | yes +11.6 ns | yes +21.6 ns |
| | tER | 7.3 ns | 17.7 ns | tight +2.3 ns | yes +12.3 ns | yes +17.3 ns |
| LOGIC, palette | tAA | 11.6 ns | 20.7 ns | yes +9.3 ns | yes +24.3 ns | yes +54.3 ns |
| | tEA | 7.4 ns | 18.5 ns | tight +1.5 ns | yes +11.5 ns | yes +21.5 ns |
| | tER | 7.4 ns | 17.8 ns | tight +2.2 ns | yes +12.2 ns | yes +17.2 ns |

#### 256x8

| Build | Parameter | Fabric (P&R) | At the socket | fastest (28/25) | common (45/25) | slowest (70/35) |
|---|---|---:|---:|---|---|---|
| **BRAM, read port 200 MHz** | tAA | 18.4 ns | 27.5 ns | tight +0.5 ns | yes +17.5 ns | yes +42.5 ns |
| | tEA | 7.5 ns | 18.6 ns | yes +6.4 ns | yes +6.4 ns | yes +16.4 ns |
| | tER | 7.5 ns | 17.8 ns | yes +7.2 ns | yes +7.2 ns | yes +12.2 ns |
| BRAM, read on sys_clock 150 MHz | tAA | 20.0 ns | 29.1 ns | **NO** -1.1 ns | yes +15.9 ns | yes +40.9 ns |
| | tEA | 7.4 ns | 18.5 ns | yes +6.5 ns | yes +6.5 ns | yes +16.5 ns |
| | tER | 7.4 ns | 17.8 ns | yes +7.2 ns | yes +7.2 ns | yes +12.2 ns |
| BRAM, read on sys_clock 100 MHz | tAA | 23.4 ns | 32.5 ns | **NO** -4.5 ns | yes +12.5 ns | yes +37.5 ns |
| | tEA | 7.6 ns | 18.7 ns | yes +6.3 ns | yes +6.3 ns | yes +16.3 ns |
| | tER | 7.6 ns | 18.0 ns | yes +7.0 ns | yes +7.0 ns | yes +12.0 ns |
| LOGIC (fallback), incompressible | tAA | 14.6 ns | 23.7 ns | tight +4.3 ns | yes +21.3 ns | yes +46.3 ns |
| | tEA | 7.7 ns | 18.7 ns | yes +6.3 ns | yes +6.3 ns | yes +16.3 ns |
| | tER | 7.7 ns | 18.0 ns | yes +7.0 ns | yes +7.0 ns | yes +12.0 ns |

**Not placed and routed: a four-image incompressible 512x8 LOGIC
build.** It is 1883 of 2112 LUTs (89 %), and on 2026-11-04 Supra had
not converged after an hour of routing (about 800 overflowing nodes
throughout), with a post-placement estimate of 20.1 ns of fabric
address delay - about 29 ns at the socket before routing congestion.
Treat multi-image incompressible LOGIC builds as unverified; real
multi-image contents are usually structured and far smaller, and
the stack's image switch is read by firmware, which has no path
into the fabric's bank bits (`fastprom_ahb_slave` has no bank
register). `X4=1 make pnr` retries it.

### Per data pin, 512x8 LOGIC incompressible

| Data bit | AG32 pin | Socket pin | Fabric tAA |
|---|---|---:|---:|
| d_pin[0] | PIN_19 | 6 | 16.02 ns |
| d_pin[1] | PIN_18 | 7 | 15.81 ns |
| d_pin[2] | PIN_15 | 8 | 14.72 ns |
| d_pin[3] | PIN_31 | 9 | 14.33 ns |
| d_pin[4] | PIN_29 | 11 | 14.89 ns |
| d_pin[5] | PIN_26 | 12 | 15.88 ns |
| d_pin[6] | PIN_27 | 13 | 15.51 ns |
| d_pin[7] | PIN_28 | 14 | 15.59 ns |

Spread across the eight outputs: 1.69 ns. The two slowest,
d_pin[0] and d_pin[5], are on PIN_19 and PIN_26; the rest of the spread is
the logic cone each bit happens to get. See the pad positions below.

### Does the pin assignment matter?

On the enable path, yes. Supra's routed netlist gives each pad's tile
on the die (23 x 14 tiles). The QFN-32 bonds its pads to tiles on
three sides, and PIN_18 and PIN_19 (with PIN_26 near them) sit at the
opposite edge from the rest:

| Signal | AG32 pin | Die tile (x, y) |
|---|---|---|
| d_pin[0] | PIN_19 | 0, 4 |
| d_pin[1] | PIN_18 | 0, 4 |
| d_pin[5] | PIN_26 | 1, 0 |
| d_pin[4] | PIN_29 | 17, 0 |
| d_pin[6] | PIN_27 | 17, 0 |
| d_pin[7] | PIN_28 | 17, 0 |
| d_pin[2] | PIN_15 | 17, 13 |
| buf_noe | PIN_14 | 18, 13 |
| d_pin[3] | PIN_31 | 19, 0 |
| a_pin[5] | PIN_10 | 19, 13 |
| a_pin[6] | PIN_11 | 19, 13 |
| a_pin[7] | PIN_12 | 19, 13 |
| a_pin[8] | PIN_13 | 19, 13 |
| a_pin[0] | PIN_8 | 20, 13 |
| a_pin[4] | PIN_7 | 20, 13 |
| ce_pin | PIN_9 | 20, 13 |
| a_pin[2] | PIN_1 | 22, 2 |
| a_pin[1] | PIN_3 | 22, 3 |
| a_pin[3] | PIN_5 | 22, 3 |

The chip enable comes in at tile 20, 13 and `buf_noe` leaves at
18, 13. The one-LUT enable path is mostly the two pads and the
route into the output pad, so its length is set by that distance.
With `buf_noe` on PIN_18, across the die, the fabric enable is
9.5-9.9 ns and tEA misses 20 ns by 0.6-0.9 ns; on PIN_14 runs on
2026-11-04 measured 7.1-7.9 ns (three seeds, three designs). Socket 8
is on PIN_15 and socket 7 on PIN_18 to make room; the data bit on
PIN_18 is up to about 1.5 ns slower than it would be on PIN_15, which
tAA absorbs.

### Run-to-run variation

The enable path is the same logic in every build, so its spread across
the 9 as-built runs is placement noise: 7.13 to
7.65 ns of fabric. Margins under about
0.5 ns are within it; a real build should be
checked from its own report (`timing.json`), not from this table.

### Clocked paths and fit

| Build | LUTs | M9K | sys_clock (Fmax) | BRAM read clock (Fmax) | Worst clocked slack |
|---|---:|---:|---:|---:|---:|
| 256x8-bram | 25/2112 | 1/4 | 100 MHz (233) | 200 MHz (342) | 3.29 ns |
| 256x8-bram-sys100 | 25/2112 | 1/4 | 100 MHz (216) | sys_clock | 5.37 ns |
| 256x8-bram-sys150 | 25/2112 | 1/4 | 150 MHz (247) | sys_clock | 2.61 ns |
| 256x8-logic | 305/2112 | 0/4 | 100 MHz (233) | - | 5.71 ns |
| 512x8-bram | 27/2112 | 1/4 | 100 MHz (216) | 200 MHz (342) | 3.29 ns |
| 512x8-bram-sys100 | 27/2112 | 1/4 | 100 MHz (216) | sys_clock | 5.38 ns |
| 512x8-bram-sys150 | 27/2112 | 1/4 | 150 MHz (216) | sys_clock | 2.04 ns |
| 512x8-logic | 590/2112 | 0/4 | 100 MHz (224) | - | 5.54 ns |
| 512x8-logic-pal | 29/2112 | 0/4 | 100 MHz (215) | - | 5.36 ns |

The clocked paths are the AHB slave (and, in BRAM mode, the load
port and the M9K): all meet their clocks with margin. Fmax is
Supra's path-based figure; its restricted Fmax, the clock-network
limit, is 200.08 MHz throughout.

### BRAM mode, decomposed

```
tAA(fabric) = T + capture + launch
  T        sampling uncertainty, one sys_clock period
           (measured in sim/tb_bram_timing.v)
  capture  pad -> M9K address register, setup, less clock latency
  launch   clock latency + M9K clock-to-out + route + output pad
```

From the 200 MHz read-clock run: capture 1.71 ns, launch
11.69 ns. Item by item, from Supra's worst paths:

| Stage | Delay | Kind |
|---|---:|---|
| Sampling: wait for the next read-clock edge | T = 1 period | quantised (SYNC_STAGES = 1) |
| Input pad | 1.31 ns | combinational |
| Route, pad to the M9K's address pins | 2.48 ns | combinational |
| M9K address-register setup | -0.09 ns | combinational |
| Clock insertion, launch less capture | 0.05 ns | cancels except this |
| M9K clock to DataOut (unregistered output) | 3.24 ns | combinational |
| Route, M9K to the data pad | 2.9-3.1 ns | combinational |
| Output pad | 3.48 ns | combinational |

There is no fabric synchroniser: the address pads feed the M9K's own
address register, the only sampling stage, and there is no output
mux for a single-block image. So the only quantised term is one
period, which `sim/tb_bram_timing.v` measures (its behavioural RAM has
tRAM = 0); everything else, about 13.4 ns, is fixed.

The read port takes its own PLL output (`PLLCLK2` in the `.ve` that
`tools/gen_pins.py` writes; `RDCLK` overrides it in
`tools/ag32/run_flow.sh`), so it can run faster than sys_clock, which
carries the CPU's AHB. Supra reports a restricted Fmax of 200.08 MHz
for any clock in this design (it times 250 and 300 MHz read clocks at
path level but flags them over that limit), so 200 MHz - 400 MHz
VCO / 2 from the 12 MHz HSE - is the ceiling.

## Model from datasheet primitives

```
tAA = ext_in + T_IN + T_LOCAL + N x (T_LUT + route) + T_DO + ext_out
tEA = ext_in + T_IN + T_LOCAL + T_LUT + route + T_DO + wire + tEN + tTERM
```

Routing per level two ways, because the datasheet quotes interconnect
at "average loading" while the recovered vendor mux tables show a
worst case nearly twice as large: 0.679 ns and 1.175 ns.
N is the depth `tools/synth_matrix.py` measures (docs/synth-results.md).

| Case | LUT depth | tAA avg routing | tAA worst routing |
|---|---:|---:|---:|
| structured contents, any organisation | 1 | 14.8 ns | 15.3 ns |
| 256x8 incompressible | 5 | 21.9 ns | 24.4 ns |
| 512x8 incompressible | 6 | 23.7 ns | 26.7 ns |
| 512x8 x2 images, incompressible | 7 | 25.5 ns | 29.0 ns |
| 512x8 x4 images, incompressible | 8 | 27.3 ns | 31.3 ns |
| 2048x4 incompressible | 8 | 27.3 ns | 31.3 ns |
| 4096x4 incompressible | 8 | 27.3 ns | 31.3 ns |

| Parameter | avg routing | worst routing |
|---|---:|---:|
| tEA, enable to outputs driving | 16.8 ns | 17.3 ns |
| tER, enable to outputs released | 16.0 ns | 16.5 ns |

**Where the model is wrong.** Against Supra for 512x8 incompressible,
the worst-routing model gives 17.6 ns of fabric against
16.0 ns placed and routed - close, but for opposite
reasons: LUT levels are cheaper than the model (0.1-0.6 ns
in Supra against 1.111 ns T_LUT), and pads are dearer. The output pad
is 3.48 ns against T_DO 2.14 ns, and the route from the last LUT into
any output pad is 3.1-3.4 ns. On the one-LUT enable path that fixed
pad overhead is almost everything: 7.3 ns placed and routed
against 6.2 ns modelled.

### BRAM mode, model

```
tAA = ext_in + SYNC_STAGES x tCLK + capture + launch + ext_out
```

The sampling term is measured, not estimated: `sim/tb_bram_timing.v`
moves the address at phases unrelated to the clock and finds the worst
case is one full clock period per sync stage, within a few
picoseconds, across every rate and depth tested.

## Conclusions

Socket figures from the P&R fabric delays plus the external path;
each against the part's fastest characterised grade.

1. **512x8, BRAM.** tAA 27.5 ns against 30 ns:
   meets it by 2.5 ns. tEA 18.2 ns against 20 ns: meets it by 1.8 ns.
   tER 17.5 ns against 20 ns: meets it by 2.5 ns. (Read on sys_clock at 100 MHz it would be 32.5 ns.)
2. **512x8, LOGIC (fallback, incompressible).** tAA 25.1 ns:
   meets it by 4.9 ns; tEA 18.4 ns: meets it by 1.6 ns; tER 17.7 ns: meets it by 2.3 ns.
3. **256x8, BRAM.** tAA 27.5 ns against 28 ns:
   meets it by 0.5 ns. tEA 18.6 ns against 25 ns: meets it by 6.4 ns.
   tER 17.8 ns against 25 ns: meets it by 7.2 ns. (Read on sys_clock at 100 MHz it would be 32.5 ns.)
4. **256x8, LOGIC (fallback, incompressible).** tAA 23.7 ns:
   meets it by 4.3 ns; tEA 18.7 ns: meets it by 6.3 ns; tER 18.0 ns: meets it by 7.0 ns.
5. **What limits BRAM mode.** One read-clock period (5 ns at
   200 MHz, the most Supra allows) plus about 13.4 ns of fixed fabric delay - pads,
   routing and the M9K's 3.24 ns access, which no pad setting or
   placement constraint changed - plus the 9.1 ns of bus interface.
   Measured on 2026-11-04: the M9K sits at the site nearest the
   pads (X13 Y4); a BRAM-optimised pin map gains about 0.2 ns; pad
   standard, drive and slew settings do not change Supra's 3.48 ns
   output pad; the RAM's output register would add a period. Backing
   off the clock limit costs little: 192 MHz (VCO 384 MHz, sys_clock
   96 MHz) is 0.2 ns slower. The 256x8's 28 ns grade is the thin one;
   against the PROMs' own 30 pF test load the terminator term is
   0.69 ns rather than 1.14 ns, which is
   worth about 0.45 ns but is not assumed here. LOGIC mode is the
   fallback where it matters.
6. **The enable path is the tightest.** One LUT of decode
   costs 7.3 ns of fabric, almost all of it two pads and the route
   into the output pad, and the stack adds 4.0 ns in and
   7.1 ns out (the drive buffer's 5.5 ns enable is the
   largest single item). tEA 18.4 ns is 0.73 of tAA, not the 1/2-1/3
   the period parts offer; a host relying on that ratio rather than on
   the datasheet limit is at risk.
7. **Pins and clocks.** The enable leaves on PIN_14,
   next to the chip enable; the address path is insensitive to the pin
   map; nothing needs a global-clock pin (LOGIC mode has no clock,
   BRAM mode clocks from the PLL fed by the HSE on PIN_2).

## Unresolved risk: the configuration window

The fabric is SRAM-based and configured from flash at power-on, and
AGM publishes no configuration time. Until the fabric is live the
socket is dead: a host that starts fetching before configuration
completes reads nothing meaningful. The design fails safe (R21 holds
the drive buffer disabled), but the time needs measuring against the
host's own reset delay.
