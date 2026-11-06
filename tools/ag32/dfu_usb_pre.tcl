# dfu_usb_pre.tcl - read by gen_logic.tcl (LOGIC_PRE) before Supra loads the
# DFU stage's USB-only fabric (tools/ag32/build_dfu_fabric.sh).
#
# Supra warns when its timing constraints reach fewer than 95 % of the
# design's connections (ta_coverage_limit, default "0.95 0.90"). This fabric
# has no logic at all: its ten connections are the clock network from the
# oscillator pins through the PLL to the CPU, the bus and the USB
# transceiver, and five of them (PLL outputs into their global-clock
# buffers, the PLL feedback, the clock switch's output) end at no register,
# so no constraint can reach them and coverage is 50 % whatever the SDC
# says. The ratio means nothing here, so the limit is off for this design
# only, and tools/ag32/check_dfu_fabric.py checks instead that every
# uncovered connection is one of those clock-network connections: anything
# else uncovered fails the build.
set ta_coverage_limit "0.0 0.0"
