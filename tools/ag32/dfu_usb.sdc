# dfu_usb.sdc - timing constraints for the DFU stage's USB-only fabric
# (tools/ag32/build_dfu_fabric.sh). gen_logic.tcl sets ::HSI_PERIOD and
# ::HSE_PERIOD from the top.hx gen_vlog writes (HSI 10 MHz, HSECLK 12) before
# reading this, as it does for an IP's SDC. The same clocks as AGM's pio
# section of the ROM fabric's top.sdc: the two clock inputs, asynchronous to
# each other, and the PLL's outputs derived from HSE.
create_clock -name PIN_HSI -period $::HSI_PERIOD [get_ports PIN_HSI]
set_clock_groups -asynchronous -group PIN_HSI
create_clock -name PIN_HSE -period $::HSE_PERIOD [get_ports PIN_HSE]
set_clock_groups -asynchronous -group PIN_HSE
derive_pll_clocks -create_base_clocks
