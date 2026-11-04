# fastprom_bram.sdc - appended after fastprom.sdc in BRAM mode only.
#
# BRAM mode: the address pads are asynchronous to sys_clock and are sampled by
# the M9K's address register; the data pads are driven from its output. A zero
# external delay on both sides makes Supra report the pad->register and
# clock->pad legs on their own; run_flow.sh combines them into
#   tAA(fabric) = 2*T - slack(pad->reg) + arrival(reg->pad)
# (the clock insertion delay cancels). Not used in LOGIC mode, where it would
# turn the combinational a_pin -> d_pin path into a one-period clocked one.
set_input_delay  0 -clock pll_inst|clkout0 [get_ports {a_pin[*]}]
set_output_delay 0 -clock pll_inst|clkout0 [get_ports {d_pin[*]}]
#
# The data pads have no clocked receiver: the host samples them at its own
# tAA, and the requirement on them is the socket budget, not one sys_clock
# period. Relax the launch leg so it is measured but does not score as a
# clocked failure (arrival times are unchanged by this).
set_multicycle_path -setup -end 4 -to [get_ports {d_pin[*]}]
set_multicycle_path -hold  -end 3 -to [get_ports {d_pin[*]}]
