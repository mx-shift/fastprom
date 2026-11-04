// AHB-Lite slave protocol and image-load sequence.
//
// Checks the three things the firmware depends on: that the outputs come up
// released after reset, that a run of DATA writes lands consecutive words
// without the firmware tracking addresses, and that reads return valid data
// in the data phase rather than a cycle late.
`timescale 1ns/1ps
`default_nettype none

module tb_ahb;

    localparam integer AW = 11;
    localparam integer DW = 8;

    localparam [31:0] BASE     = 32'h6000_0000;
    localparam [3:0]  REG_CTRL = 4'h0;
    localparam [3:0]  REG_ADDR = 4'h4;
    localparam [3:0]  REG_DATA = 4'h8;
    localparam [3:0]  REG_ID   = 4'hC;

    reg hclk = 1'b0;
    always #5 hclk = ~hclk;

    reg         hresetn = 1'b0;
    reg         hsel    = 1'b0;
    reg  [31:0] haddr   = 32'h0;
    reg         hwrite  = 1'b0;
    reg  [1:0]  htrans  = 2'b00;
    reg  [31:0] hwdata  = 32'h0;
    wire [31:0] hrdata;
    wire        hreadyout;
    wire [1:0]  hresp;

    wire          ld_en;
    wire [AW-1:0] ld_addr;
    wire [DW-1:0] ld_data;
    wire          core_ready;

    fastprom_ahb_slave #(.AW (AW), .DW (DW)) dut (
        .hclk (hclk), .hresetn (hresetn),
        .hsel (hsel), .haddr (haddr), .hwrite (hwrite), .htrans (htrans),
        .hwdata (hwdata), .hrdata (hrdata),
        .hreadyout (hreadyout), .hresp (hresp),
        .ld_en (ld_en), .ld_addr (ld_addr), .ld_data (ld_data),
        .core_ready (core_ready)
    );

    // Shadow of what the load port actually wrote, to check ordering.
    reg [DW-1:0] shadow [0:(1<<AW)-1];
    reg          written [0:(1<<AW)-1];

    always @(posedge hclk) begin
        if (ld_en) begin
            shadow[ld_addr]  <= ld_data;
            written[ld_addr] <= 1'b1;
        end
    end

    integer errors = 0;
    integer i;
    reg [31:0] got;

    task ahb_write(input [3:0] reg_off, input [31:0] data);
        begin
            // Address phase.
            @(negedge hclk);
            hsel   = 1'b1;
            haddr  = BASE | {28'h0, reg_off};
            hwrite = 1'b1;
            htrans = 2'b10;
            // Data phase.
            @(negedge hclk);
            htrans = 2'b00;
            hsel   = 1'b0;
            hwdata = data;
        end
    endtask

    task ahb_read(input [3:0] reg_off, output [31:0] data);
        begin
            @(negedge hclk);
            hsel   = 1'b1;
            haddr  = BASE | {28'h0, reg_off};
            hwrite = 1'b0;
            htrans = 2'b10;
            @(negedge hclk);
            htrans = 2'b00;
            hsel   = 1'b0;
            // Data phase: a zero-wait-state slave must have it ready now.
            data   = hrdata;
        end
    endtask

    initial begin
        for (i = 0; i < (1 << AW); i = i + 1) begin
            shadow[i]  = {DW{1'b0}};
            written[i] = 1'b0;
        end

        repeat (3) @(negedge hclk);
        hresetn = 1'b1;
        @(negedge hclk);

        if (core_ready !== 1'b0) begin
            errors = errors + 1;
            $display("FAIL: core_ready set after reset - would drive before load");
        end
        if (hreadyout !== 1'b1) begin
            errors = errors + 1;
            $display("FAIL: hreadyout not high");
        end

        // Identify the slave.
        ahb_read(REG_ID, got);
        if (got !== 32'h4650524D) begin
            errors = errors + 1;
            $display("FAIL: ID read 0x%08x, expected 0x4650524d", got);
        end

        // Load a whole image the way the firmware does: set the pointer once,
        // then store words back to back.
        ahb_write(REG_ADDR, 32'h0);
        for (i = 0; i < (1 << AW); i = i + 1)
            ahb_write(REG_DATA, (i * 7 + 3) & 8'hFF);

        // The load port is two cycles behind the data phase, so let the last
        // write drain before inspecting what landed.
        repeat (4) @(negedge hclk);

        for (i = 0; i < (1 << AW); i = i + 1) begin
            if (!written[i]) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL: word %0d never written", i);
            end else if (shadow[i] !== ((i * 7 + 3) & 8'hFF)) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL: word %0d is %h, expected %h",
                             i, shadow[i], (i * 7 + 3) & 8'hFF);
            end
        end

        // Pointer should have wrapped exactly back to zero.
        ahb_read(REG_ADDR, got);
        if (got[AW-1:0] !== {AW{1'b0}}) begin
            errors = errors + 1;
            $display("FAIL: pointer is %0d after a full image, expected 0",
                     got[AW-1:0]);
        end

        // Now release the outputs.
        ahb_write(REG_CTRL, 32'h1);
        @(negedge hclk);
        if (core_ready !== 1'b1) begin
            errors = errors + 1;
            $display("FAIL: core_ready did not set");
        end

        ahb_read(REG_CTRL, got);
        if (got[0] !== 1'b1) begin
            errors = errors + 1;
            $display("FAIL: CTRL read back %0d", got[0]);
        end

        if (errors == 0)
            $display("tb_ahb: PASS (%0d words loaded, protocol and interlock ok)",
                     1 << AW);
        else begin
            $display("tb_ahb: FAIL (%0d errors)", errors);
            $fatal(1);
        end
        $finish;
    end

endmodule

`default_nettype wire
