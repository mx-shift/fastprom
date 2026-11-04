// -----------------------------------------------------------------------------
// fastprom_rom_logic - ROM contents as pure combinational logic
//
// This is the mode that makes the project work. A 20-pin bipolar PROM holds
// 2-8 Kbit, which is small enough to synthesise into the fabric's LUT4s as a
// plain boolean function of the address. There is no clock, no memory block
// and no sampling edge in the read path, so the behaviour is asynchronous in
// exactly the way the original part is, and the delay is just pad + a handful
// of LUT levels.
//
// Real PROM contents (palettes, address decodes, character generators) are
// highly structured, so the synthesiser's logic minimisation usually lands
// well under the worst-case LUT count for a random truth table.
//
// This file is the *reference* implementation: an asynchronously-read,
// initialised array. Because the read is asynchronous it cannot map to the
// fabric's synchronous M9K blocks, so the synthesiser is forced to implement
// it in logic. tools/gen_rom_logic.py emits an equivalent explicit-case
// version for production builds; sim/tb_equiv.v proves the two agree.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_rom_logic #(
    parameter integer AW = 9,          // total address width (incl. bank bits)
    parameter integer DW = 8,          // data width
    parameter         INIT_HEX = ""    // hex image, one byte per line
) (
    input  wire [AW-1:0] a,
    output wire [DW-1:0] d
);
    localparam integer DEPTH = 1 << AW;

    reg [DW-1:0] rom [0:DEPTH-1];

    integer i;
    initial begin
        // Unprogrammed bipolar PROM cells read as 1s, so that is the sane
        // default for any location the image does not cover.
        for (i = 0; i < DEPTH; i = i + 1)
            rom[i] = {DW{1'b1}};
        if (INIT_HEX != "")
            $readmemh(INIT_HEX, rom);
    end

    // Asynchronous read - no clock anywhere in this path.
    assign d = rom[a];

endmodule

`default_nettype wire
