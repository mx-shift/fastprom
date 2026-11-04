// Exhaustive functional check of LOGIC mode: every address in every bank,
// chip-enable behaviour, and the core_ready bus-safety interlock.
`timescale 1ns/1ps
`default_nettype none

module tb_functional;

    localparam integer PROM_AW = 9;
    localparam integer DW      = 8;
    localparam integer BANKW   = 2;
    localparam integer AW      = PROM_AW + BANKW;
    localparam         IMAGE   = "build/img_rand.hex";

    reg  [PROM_AW-1:0] a_pin      = 0;
    reg  [BANKW-1:0]   bank_sel   = 0;
    reg                ce_n       = 1'b1;
    reg                core_ready = 1'b0;
    wire [DW-1:0]      d_pin;
    wire               selected;

    fastprom_top #(
        .PROM_AW        (PROM_AW),
        .DW             (DW),
        .NCE            (1),
        .CE_ACTIVE_LOW  (4'b0001),
        .OPEN_COLLECTOR (0),
        .BANKW          (BANKW),
        .MODE_BRAM      (0),
        .INIT_HEX       (IMAGE)
    ) dut (
        .a_pin      (a_pin),
        .ce_pin     (ce_n),
        .d_pin      (d_pin),
        .bank_sel   (bank_sel),
        .fabric_clk (1'b0),
        .core_ready (core_ready),
        .ld_clk     (1'b0),
        .ld_en      (1'b0),
        .ld_addr    ({AW{1'b0}}),
        .ld_data    ({DW{1'b0}}),
        .buf_noe    (),
        .od_data    (),
        .selected   (selected)
    );

    // Golden model: the same image, read independently of the DUT.
    reg [DW-1:0] golden [0:(1<<AW)-1];

    integer errors = 0;
    integer bank, addr;
    reg [DW-1:0] expect_d;

    task check(input [8*24:1] what, input [DW-1:0] got, input [DW-1:0] want);
        begin
            if (got !== want) begin
                errors = errors + 1;
                if (errors <= 10)
                    $display("FAIL %0s: bank=%0d addr=%0d got=%b want=%b",
                             what, bank, addr, got, want);
            end
        end
    endtask

    initial begin
        $readmemh(IMAGE, golden);

        // --- bus must stay off until the image is declared present ---------
        core_ready = 1'b0;
        ce_n       = 1'b0;
        a_pin      = 0;
        bank_sel   = 0;
        #10;
        if (d_pin !== {DW{1'bz}}) begin
            errors = errors + 1;
            $display("FAIL: drove the bus with core_ready low: %b", d_pin);
        end

        core_ready = 1'b1;

        // --- deselected part must float ------------------------------------
        ce_n = 1'b1;
        #10;
        if (d_pin !== {DW{1'bz}}) begin
            errors = errors + 1;
            $display("FAIL: drove the bus while deselected: %b", d_pin);
        end
        if (selected !== 1'b0) begin
            errors = errors + 1;
            $display("FAIL: selected asserted while CE inactive");
        end

        // --- exhaustive sweep ----------------------------------------------
        ce_n = 1'b0;
        #1;
        for (bank = 0; bank < (1 << BANKW); bank = bank + 1) begin
            bank_sel = bank[BANKW-1:0];
            for (addr = 0; addr < (1 << PROM_AW); addr = addr + 1) begin
                a_pin = addr[PROM_AW-1:0];
                #1;
                expect_d = golden[(bank << PROM_AW) + addr];
                check("data", d_pin, expect_d);
            end
        end

        if (selected !== 1'b1) begin
            errors = errors + 1;
            $display("FAIL: selected deasserted while CE active");
        end

        $display("tb_functional: swept %0d addresses across %0d banks",
                 1 << PROM_AW, 1 << BANKW);
        if (errors == 0)
            $display("tb_functional: PASS");
        else begin
            $display("tb_functional: FAIL (%0d errors)", errors);
            $fatal(1);
        end
        $finish;
    end

endmodule

`default_nettype wire
