// Prove the generated explicit-case ROM agrees with the reference model.
//
// hdl/fastprom_rom_logic.v is a readable reference: an initialised array with
// an asynchronous read. gen/*.v is what actually gets synthesised: the same
// contents flattened into a case statement so the mapper is unambiguously
// handed combinational logic. A build substitutes one for the other, so they
// must agree on every address, including the locations the generator folded
// into the default arm.
`timescale 1ns/1ps
`default_nettype none

module tb_equiv;

    parameter integer AW    = 11;
    parameter integer DW    = 8;
    parameter         IMAGE = "build/img_rand.hex";

    reg [AW-1:0] a = 0;

    wire [DW-1:0] d_ref;
    wire [DW-1:0] d_gen;

    fastprom_rom_logic #(
        .AW (AW), .DW (DW), .INIT_HEX (IMAGE)
    ) u_ref (.a (a), .d (d_ref));

    fastprom_rom_logic_gen #(
        .AW (AW), .DW (DW)
    ) u_gen (.a (a), .d (d_gen));

    integer errors = 0;
    integer i;

    initial begin
        for (i = 0; i < (1 << AW); i = i + 1) begin
            a = i[AW-1:0];
            #1;
            if (d_ref !== d_gen) begin
                errors = errors + 1;
                if (errors <= 10)
                    $display("FAIL addr=%0d reference=%b generated=%b",
                             i, d_ref, d_gen);
            end
        end

        if (errors == 0)
            $display("tb_equiv: PASS (%0d addresses, reference == generated)",
                     1 << AW);
        else begin
            $display("tb_equiv: FAIL (%0d mismatches)", errors);
            $fatal(1);
        end
        $finish;
    end

endmodule

`default_nettype wire
