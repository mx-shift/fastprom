// Replays the Rust firmware's load sequence on the AHB slave itself.
//
// fw/fastprom-core/tests/hdl_trace.rs runs the real loader (fastprom-core's
// Released::claim and load) against its model of this slave and writes
// every bus transfer it made, with the value each read returned. This
// testbench drives those transfers into hdl/fastprom_ahb_slave.v and checks:
//
//   - every read returns what the model returned (the model is the HDL),
//   - the load port wrote exactly the image, every word,
//   - no word was written while READY was set, and READY ends as the trace
//     left it.
//
// GAP is the number of idle cycles after each transfer: 0 drives them back
// to back, a pipelined master's worst case (the address phase of each
// transfer in the data phase of the one before), 1 or more gives every
// transfer its own data phase. The firmware's sequence must pass both. The
// hazard trace checks the model's read-after-write behaviour, and only
// makes sense back to back.
//
// Trace format: 40-bit words, kind[39:36] (1 write, 2 read, f end),
// register[35:32] (byte offset / 4), data or expected read value[31:0].
`timescale 1ns/1ps
`default_nettype none

module tb_fw_load;
    parameter integer AW          = 9;
    parameter integer DW          = 8;
    parameter         TRACE       = "build/fw/trace/load_aw9_dw8.trace";
    parameter         IMAGE       = "build/fw/trace/load_aw9_dw8.hex";
    parameter integer CHECK_IMAGE = 1;
    parameter integer GAP         = 0;

    localparam integer    MAXOPS = 20000;
    localparam [31:0]     BASE   = 32'h6000_0000;
    localparam [3:0]      K_WR   = 4'h1;
    localparam [3:0]      K_RD   = 4'h2;
    localparam [3:0]      K_END  = 4'hf;

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

    reg [39:0]   ops [0:MAXOPS-1];
    reg [DW-1:0] img [0:(1<<AW)-1];
    reg [DW-1:0] bram    [0:(1<<AW)-1];
    reg          written [0:(1<<AW)-1];

    integer errors = 0;
    integer reads  = 0;
    integer n, i, g, fd;
    reg     want_ready;
    reg [39:0] word;

    // The block RAM as the load port fills it; a word written while READY
    // is set would be the socket serving a half-loaded image.
    always @(posedge hclk) begin
        if (ld_en) begin
            bram[ld_addr]    <= ld_data;
            written[ld_addr] <= 1'b1;
            if (core_ready) begin
                errors = errors + 1;
                $display("FAIL word %0d written with READY set", ld_addr);
            end
        end
    end

    // The transfer whose data phase is the current cycle.
    reg        dp_valid = 1'b0;
    reg        dp_read  = 1'b0;
    reg [31:0] dp_data  = 32'h0;
    integer    dp_index = 0;

    // One bus cycle: finish the previous transfer's data phase and drive
    // this one's address phase (valid = 0 for an idle cycle).
    task cycle(input valid, input [39:0] op, input integer index);
        begin
            @(negedge hclk);
            if (dp_valid && dp_read) begin
                reads = reads + 1;
                if (hrdata !== dp_data) begin
                    errors = errors + 1;
                    if (errors <= 10)
                        $display("FAIL transfer %0d: HDL read %h, firmware model read %h",
                                 dp_index, hrdata, dp_data);
                end
            end
            hwdata = (dp_valid && !dp_read) ? dp_data : 32'h0;
            if (valid) begin
                hsel   = 1'b1;
                htrans = 2'b10;
                hwrite = (op[39:36] == K_WR);
                haddr  = BASE | {26'h0, op[35:32], 2'b00};
            end else begin
                hsel   = 1'b0;
                htrans = 2'b00;
                hwrite = 1'b0;
                haddr  = BASE;
            end
            dp_valid = valid;
            dp_read  = valid && (op[39:36] == K_RD);
            dp_data  = op[31:0];
            dp_index = index;
        end
    endtask

    initial begin
        // $fscanf rather than $readmemh: a trace is shorter than MAXOPS.
        for (i = 0; i < MAXOPS; i = i + 1) ops[i] = {K_END, 36'h0};
        fd = $fopen(TRACE, "r");
        if (fd == 0) begin
            $display("tb_fw_load: FAIL (cannot open %s)", TRACE);
            $finish;
        end
        i = 0;
        while (i < MAXOPS && $fscanf(fd, "%h\n", word) == 1) begin
            ops[i] = word;
            i = i + 1;
        end
        $fclose(fd);
        if (CHECK_IMAGE) $readmemh(IMAGE, img);
        for (i = 0; i < (1 << AW); i = i + 1) written[i] = 1'b0;

        repeat (3) @(posedge hclk);
        hresetn = 1'b1;

        // The traces start as after a warm reset, with READY still set
        // from the last boot: the firmware must clear it before any word.
        cycle(1'b1, {K_WR, 4'h0, 32'h1}, -1);
        cycle(1'b0, 40'h0, -1);
        cycle(1'b0, 40'h0, -1);
        if (core_ready !== 1'b1) begin
            errors = errors + 1;
            $display("FAIL could not preset READY");
        end

        want_ready = 1'b1;
        n = 0;
        while (n < MAXOPS && ops[n][39:36] != K_END) begin
            if (ops[n][39:36] == K_WR && ops[n][35:32] == 4'h0)
                want_ready = ops[n][0];
            cycle(1'b1, ops[n], n);
            for (g = 0; g < GAP; g = g + 1) cycle(1'b0, 40'h0, -1);
            n = n + 1;
        end
        cycle(1'b0, 40'h0, -1);
        cycle(1'b0, 40'h0, -1);
        if (n == 0) begin
            errors = errors + 1;
            $display("FAIL empty trace %s", TRACE);
        end

        if (CHECK_IMAGE) begin
            for (i = 0; i < (1 << AW); i = i + 1) begin
                if (written[i] !== 1'b1 || bram[i] !== img[i]) begin
                    errors = errors + 1;
                    if (errors <= 10)
                        $display("FAIL word %0d: block RAM %h, image %h (written %b)",
                                 i, bram[i], img[i], written[i]);
                end
            end
        end
        if (core_ready !== want_ready) begin
            errors = errors + 1;
            $display("FAIL READY is %b, the trace left it %b", core_ready, want_ready);
        end

        if (errors == 0)
            $display("tb_fw_load AW=%0d DW=%0d GAP=%0d: PASS (%0d transfers, %0d reads checked)",
                     AW, DW, GAP, n, reads);
        else
            $display("tb_fw_load AW=%0d DW=%0d GAP=%0d: FAIL (%0d errors)", AW, DW, GAP, errors);
        $finish;
    end
endmodule

`default_nettype wire
