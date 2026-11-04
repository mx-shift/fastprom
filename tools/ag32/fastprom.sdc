# fastprom.sdc - appended to the generated logic/top.sdc by tools/ag32/run_flow.sh
# (after AGM's pio section, which creates PIN_HSE/PIN_HSI and the PLL clocks;
# sys_clock is pll_inst|clkout0).
#
# The fabric's share of the socket timing. The socket-side figures (tAA, tEA)
# are the fabric delays plus the external path on the stack - two LVT245
# crossings, the DF40, the traces and the 33 R terminators - which
# tools/timing_budget.py adds; see docs/03-timing-budget.md. The limits here
# are the fabric allowances that leave the fastest grades met:
#
#   address -> data  20 ns   (30 ns tAA grade less ~9 ns of external path)
#   enable  -> buf_noe 10 ns (20 ns tEA target less ~9.5 ns: drive buffer
#                             tPZH/tPZL 5.5 ns, receive tpd 3.5 ns, wiring)
#
# Both are combinational, pad to pad; the slack is what is reported.

set_max_delay 20 -from [get_ports {a_pin[*]}] -to [get_ports {d_pin[*]}]
set_max_delay 10 -from [get_ports {ce_pin*}] -to [get_ports {buf_noe}]
# The data pads drive unconditionally on this build, so ce reaches the bus
# only through buf_noe. A multi-image build's bank register is static.
