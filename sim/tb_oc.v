// Open-collector personality check.
//
// An open-collector PROM must never source current: a '1' is a float that the
// host's pull-up resolves. Proving that from outside needs two copies of the
// part on two differently-terminated buses - with a pull-up the outputs read
// the image, and with a pull-down every bit reads 0, which is only possible if
// the part is genuinely not driving the high bits.
`timescale 1ns/1ps
`default_nettype none

module tb_oc;

    localparam integer PROM_AW = 8;
    localparam integer DW      = 8;
    localparam integer BANKW   = 0;
    localparam integer AW      = PROM_AW + BANKW;
    localparam         IMAGE   = "build/img_oc.hex";

    reg [PROM_AW-1:0] a_pin = 0;
    reg               ce_n  = 1'b0;

    tri1 [DW-1:0] d_pullup;    // host terminates high, as an OC bus must
    tri0 [DW-1:0] d_pulldown;  // pathological bus used only as a probe

    // The signal the open-collector board variant feeds to external
    // open-drain buffers. Low means pull the bus down; high means float.
    wire [DW-1:0] od_data;

    // Two copies of the part on two differently-terminated buses. Connected
    // directly rather than through an intermediate wire, because a
    // continuously-assigned wire would break the bidirectional path.
    fastprom_top #(
        .PROM_AW        (PROM_AW),
        .DW             (DW),
        .NCE            (1),
        .CE_ACTIVE_LOW  (4'b0001),
        .OPEN_COLLECTOR (1),
        .BANKW          (BANKW),
        .MODE_BRAM      (0),
        .INIT_HEX       (IMAGE)
    ) dut_pullup (
        .a_pin      (a_pin),
        .ce_pin     (ce_n),
        .d_pin      (d_pullup),
        .od_data    (od_data),
        .bank_sel   (1'b0),
        .fabric_clk (1'b0),
        .core_ready (1'b1),
        .ld_clk     (1'b0),
        .ld_en      (1'b0),
        .ld_addr    ({AW{1'b0}}),
        .ld_data    ({DW{1'b0}}),
        .buf_noe    (),
        .selected   ()
    );

    fastprom_top #(
        .PROM_AW        (PROM_AW),
        .DW             (DW),
        .NCE            (1),
        .CE_ACTIVE_LOW  (4'b0001),
        .OPEN_COLLECTOR (1),
        .BANKW          (BANKW),
        .MODE_BRAM      (0),
        .INIT_HEX       (IMAGE)
    ) dut_pulldown (
        .a_pin      (a_pin),
        .ce_pin     (ce_n),
        .d_pin      (d_pulldown),
        .od_data    (),
        .bank_sel   (1'b0),
        .fabric_clk (1'b0),
        .core_ready (1'b1),
        .ld_clk     (1'b0),
        .ld_en      (1'b0),
        .ld_addr    ({AW{1'b0}}),
        .ld_data    ({DW{1'b0}}),
        .buf_noe    (),
        .selected   ()
    );

    reg [DW-1:0] golden [0:(1<<PROM_AW)-1];
    integer errors = 0;
    integer addr;

    initial begin
        $readmemh(IMAGE, golden);
        #1;
        for (addr = 0; addr < (1 << PROM_AW); addr = addr + 1) begin
            a_pin = addr[PROM_AW-1:0];
            #1;
            // The external-buffer signal must agree with what the internal
            // open-collector emulation puts on the bus, or the two board
            // variants would not behave alike.
            if (od_data !== golden[addr]) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL od_data: addr=%0d got=%b want=%b",
                             addr, od_data, golden[addr]);
            end
            if (d_pullup !== golden[addr]) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL pull-up bus: addr=%0d got=%b want=%b",
                             addr, d_pullup, golden[addr]);
            end
            // Every high bit must be a float, so a pulled-down bus reads 0.
            if (d_pulldown !== {DW{1'b0}}) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL pull-down bus: addr=%0d got=%b (sourcing current)",
                             addr, d_pulldown);
            end
        end

        // Deselected: both buses must be left entirely to their terminations.
        ce_n = 1'b1;
        #2;
        if (d_pullup !== {DW{1'b1}} || d_pulldown !== {DW{1'b0}}) begin
            errors = errors + 1;
            $display("FAIL: OC part drove the bus while deselected (pu=%b pd=%b)",
                     d_pullup, d_pulldown);
        end
        // Deselected, every external buffer must be told to float.
        if (od_data !== {DW{1'b1}}) begin
            errors = errors + 1;
            $display("FAIL: od_data %b while deselected, should be all ones",
                     od_data);
        end

        if (errors == 0)
            $display("tb_oc: PASS (%0d addresses, never sourced current)",
                     1 << PROM_AW);
        else begin
            $display("tb_oc: FAIL (%0d errors)", errors);
            $fatal(1);
        end
        $finish;
    end

endmodule

`default_nettype wire
