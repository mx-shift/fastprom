# dfu_usb_post.tcl - read by gen_logic.tcl (LOGIC_POST) after it has written
# the compressed bitstream top.bin (tools/ag32/build_dfu_fabric.sh).
#
# Write the same routed design again, uncompressed, as top_raw.bin, so that
# tools/ag32/check_dfu_fabric.py can check that top.bin expands to exactly
# Supra's plain bitstream. It must be this run's: Supra's router does not
# pick the same clock routing in every run (two runs of the same inputs can
# differ in a few routing bits), so a second run is no reference.
set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION "OFF"
bitgen normal -bin top_raw.bin
set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION "ON"
