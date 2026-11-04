// -----------------------------------------------------------------------------
// fastprom_enable - chip-select decode and output-driver personality
//
// Bipolar PROMs differ in three ways that matter at the pad:
//   * how many chip-enable pins they have (1..4),
//   * whether each enable is active low or active high,
//   * whether the output stage is tri-state or open-collector.
//
// All three are parameters here so one bitstream family covers the whole
// 20-pin catalogue. This block is purely combinational: there is deliberately
// no clock in the enable path, because tEA/tER on the parts we replace are
// 15-25 ns and a synchroniser would eat the whole budget.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_enable #(
    parameter integer DW  = 8,   // data bus width
    parameter integer NCE = 1,   // number of chip-enable pins on the target part

    // Bit i set  => ce_pin[i] is active LOW.  Bit i clear => active HIGH.
    parameter [3:0] CE_ACTIVE_LOW = 4'b1111,

    // Output stage: 0 = tri-state (82S147, 74S472, ...)
    //               1 = open-collector (82S126, 74S470, ...)
    parameter integer OPEN_COLLECTOR = 0
) (
    input  wire [NCE-1:0]  ce_pin,   // raw socket enable pins
    input  wire [DW-1:0]   rom_data, // data from the ROM core

    output wire [DW-1:0]   pad_out,  // value to drive when pad_oe is set
    output wire [DW-1:0]   pad_oe,   // per-bit output enable (per-bit for OC)
    output wire            selected  // part is enabled (for diagnostics/LED)
);

    // ---- enable decode -----------------------------------------------------
    // The part drives the bus only when EVERY enable pin is in its active
    // state, which is how the real parts behave.
    wire [NCE-1:0] ce_active;
    genvar i;
    generate
        for (i = 0; i < NCE; i = i + 1) begin : g_ce
            assign ce_active[i] = CE_ACTIVE_LOW[i] ? ~ce_pin[i] : ce_pin[i];
        end
    endgenerate

    assign selected = &ce_active;

    // ---- output stage ------------------------------------------------------
    generate
        if (OPEN_COLLECTOR) begin : g_oc
            // Open-collector: never source current. A '1' is high-Z and the
            // host's pull-up does the work; a '0' pulls the line down. This
            // also means the outputs float when the part is deselected, which
            // is exactly the real part's behaviour.
            assign pad_out = {DW{1'b0}};
            assign pad_oe  = selected ? ~rom_data : {DW{1'b0}};
        end else begin : g_ts
            // Tri-state: drive both levels while selected, float otherwise.
            assign pad_out = rom_data;
            assign pad_oe  = selected ? {DW{1'b1}} : {DW{1'b0}};
        end
    endgenerate

endmodule

`default_nettype wire
