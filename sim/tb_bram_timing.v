// BRAM mode: MCU load path, plus a measurement of the address-sampling
// latency bound that the oversampling architecture relies on.
//
// This is an RTL measurement, so it captures only the architectural part of
// the delay - the sampling uncertainty plus the address register. It does NOT
// include pad delay or M9K array access time; those are added in
// docs/03-timing-budget.md. What it does prove is that the worst case over
// address changes landing at arbitrary phases really is SYNC_STAGES clock
// periods and not more, which is the claim the budget is built on.
`timescale 1ps/1ps
`default_nettype none

module tb_bram_timing;

    parameter integer TCLK_PS     = 6667;  // 150 MHz fabric clock
    parameter integer SYNC_STAGES = 1;
    parameter integer TRIALS      = 400;
    // Extra latency allowed on top of the architectural bound. 0 for the
    // behavioural model. AGM's alta_bram9k simulation model (the ALTA_SYN
    // build, see scripts/run_sim.sh) adds a fixed read-pulse delay of about
    // 1 ns that is a modelling artefact, not timing; its run allows for it.
    parameter integer MODEL_SLACK_PS = 0;

    localparam integer PROM_AW = 9;
    localparam integer DW      = 8;
    localparam integer BANKW   = 2;
    localparam integer AW      = PROM_AW + BANKW;
    localparam         IMAGE   = "build/img_rand.hex";

    reg clk = 1'b0;
    always #(TCLK_PS/2) clk = ~clk;

    // The core's bus clock is unrelated to and slower than the read clock;
    // driving the load port from it exercises the dual-clock M9K arrangement
    // rather than pretending both ports share a clock.
    reg ld_clk = 1'b0;
    always #(2 * TCLK_PS) ld_clk = ~ld_clk;

    reg  [PROM_AW-1:0] a_pin      = 0;
    reg  [BANKW-1:0]   bank_sel   = 0;
    reg                ce_n       = 1'b1;
    reg                core_ready = 1'b0;
    wire [DW-1:0]      d_pin;

    reg                ld_en   = 1'b0;
    reg  [AW-1:0]      ld_addr = 0;
    reg  [DW-1:0]      ld_data = 0;

    fastprom_top #(
        .PROM_AW        (PROM_AW),
        .DW             (DW),
        .NCE            (1),
        .CE_ACTIVE_LOW  (4'b0001),
        .OPEN_COLLECTOR (0),
        .BANKW          (BANKW),
        .MODE_BRAM      (1),
        .SYNC_STAGES    (SYNC_STAGES),
        .INIT_HEX       ("")            // deliberately empty: the core loads it
    ) dut (
        .a_pin      (a_pin),
        .ce_pin     (ce_n),
        .d_pin      (d_pin),
        .bank_sel   (bank_sel),
        .fabric_clk (clk),
        .core_ready (core_ready),
        .ld_clk     (ld_clk),
        .ld_en      (ld_en),
        .ld_addr    (ld_addr),
        .ld_data    (ld_data),
        .buf_noe    (),
        .od_data    (),
        .selected   ()
    );

    reg [DW-1:0] golden [0:(1<<AW)-1];

    integer errors = 0;
    integer trial, i;
    integer max_latency = 0;
    integer bound;
    integer t0;
    integer latency;

    // Exact settle detection: remember when the outputs last moved, rather
    // than polling, so the measurement has no quantisation error.
    integer last_change = 0;
    always @(d_pin) last_change = $time;
    reg [AW-1:0] target;
    reg [DW-1:0] want;
    integer seed = 32'h5EED;

    initial begin
        $readmemh(IMAGE, golden);

        // ---- the core fills the block RAM from its flash -------------------
        // Bus stays off throughout; a half-loaded image must never be served.
        ce_n = 1'b0;
        @(negedge ld_clk);
        for (i = 0; i < (1 << AW); i = i + 1) begin
            ld_en   = 1'b1;
            ld_addr = i[AW-1:0];
            ld_data = golden[i];
            @(negedge ld_clk);
            if (d_pin !== {DW{1'bz}}) begin
                errors = errors + 1;
                if (errors <= 3)
                    $display("FAIL: bus driven during load at word %0d", i);
            end
        end
        ld_en = 1'b0;
        @(negedge ld_clk);
        core_ready = 1'b1;

        // ---- readback of everything the core wrote -------------------------
        for (i = 0; i < (1 << AW); i = i + 1) begin
            a_pin    = i[PROM_AW-1:0];
            bank_sel = i[AW-1:PROM_AW];
            #(4*TCLK_PS);
            if (d_pin !== golden[i]) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL readback: addr=%0d got=%b want=%b",
                             i, d_pin, golden[i]);
            end
        end

        // ---- latency measurement at arbitrary clock phases -----------------
        bound = SYNC_STAGES * TCLK_PS + MODEL_SLACK_PS;

        for (trial = 0; trial < TRIALS; trial = trial + 1) begin
            // Land the address change at a phase unrelated to the clock.
            #(1 + ({$random(seed)} % (TCLK_PS - 1)));

            target   = {$random(seed)} % (1 << AW);
            want     = golden[target];
            a_pin    = target[PROM_AW-1:0];
            bank_sel = target[AW-1:PROM_AW];

            t0 = $time;
            #(4 * TCLK_PS);

            // If the new word happens to equal the old one the pins never
            // move, which counts as zero latency.
            latency = (last_change > t0) ? (last_change - t0) : 0;
            if (latency > max_latency)
                max_latency = latency;

            if (d_pin !== want) begin
                errors = errors + 1;
                if (errors <= 5)
                    $display("FAIL trial %0d: never settled, addr=%0d got=%b want=%b",
                             trial, target, d_pin, want);
            end
        end

        $display("tb_bram_timing: fclk=%0d MHz (%0d ps), SYNC_STAGES=%0d",
                 1000000 / TCLK_PS, TCLK_PS, SYNC_STAGES);
        $display("tb_bram_timing: measured worst-case sampling latency %0d ps over %0d trials",
                 max_latency, TRIALS);
        $display("tb_bram_timing: architectural bound %0d ps", bound);

        if (max_latency > bound) begin
            errors = errors + 1;
            $display("FAIL: measured latency exceeds the bound the budget assumes");
        end

        if (errors == 0)
            $display("tb_bram_timing: PASS");
        else begin
            $display("tb_bram_timing: FAIL (%0d errors)", errors);
            $fatal(1);
        end
        $finish;
    end

endmodule

`default_nettype wire
