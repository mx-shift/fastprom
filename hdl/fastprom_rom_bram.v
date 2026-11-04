// -----------------------------------------------------------------------------
// fastprom_rom_bram - ROM contents in fabric block RAM, loaded at boot
//
// The flexible mode: contents live in an M9K block that the RISC-V core fills
// from its own flash at start-up, so swapping images (or serving several
// selectable images) needs no resynthesis. The cost is that M9K blocks are
// synchronous-read only, so an asynchronous ROM has to be emulated by
// oversampling the address with a free-running fabric clock.
//
// Timing, worst case, from the last address transition to valid data:
//
//     tPD = tPAD_IN + tROUTE + tSU + SYNC_STAGES * tCLK + tRAM + tPAD_OUT
//
// SYNC_STAGES * tCLK is the sampling uncertainty plus the pipeline: the last
// sampling stage IS the M9K's own address register, so with SYNC_STAGES = 1
// the address pins feed the block RAM's address register directly and the
// array output is unregistered - the lowest-latency M9K pattern (registered
// address, combinational output). sim/tb_bram_timing.v measures the
// SYNC_STAGES * tCLK term; tRAM (M9K clock to data out) is the vendor timing
// model's, reported by tools/ag32/run_flow.sh.
//
// SYNC_STAGES = 1 is the default and the fast choice. A host address change
// that lands on the sampling edge may be captured half-old/half-new, so the
// outputs can show a wrong value for one clock period. That is not a defect:
// the part being replaced also has undefined outputs during its tAA window,
// and the guarantee we must meet is only that data is correct and stable by
// tAA after the last address transition. SYNC_STAGES = 2 removes the
// metastability exposure at the cost of one more clock period of delay.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_rom_bram #(
    parameter integer AW = 9,
    parameter integer DW = 8,
    parameter integer SYNC_STAGES = 1,
    parameter         INIT_HEX = ""     // simulation convenience only
) (
    input  wire                 clk,      // read side, free-running

    input  wire [AW-1:0]        a,
    output wire [DW-1:0]        d,

    // Load port, driven by the RISC-V core through the fabric AHB slave.
    // M9K blocks are true dual-port with independent clocks per port, so the
    // write side runs on the core's bus clock and needs no clock crossing.
    // Loading always completes before the outputs are released, so there is
    // no concurrent read and write to worry about.
    input  wire                 ld_clk,
    input  wire                 ld_en,
    input  wire [AW-1:0]        ld_addr,
    input  wire [DW-1:0]        ld_data
);
    localparam integer DEPTH = 1 << AW;

`ifdef ALTA_SYN
    // AGM's native flow (tools/ag32/run_flow.sh). Instantiate the AGRV2K M9K
    // primitive directly: a yosys-inferred memory goes through AGM's
    // alta_ram9k wrapper, which this yosys cannot evaluate, and the read data
    // then becomes undefined and everything downstream is optimised away.
    //
    // Each block is used as 512 x 18 (the mode cosmic-toad's ct_ram.v builds
    // with), DW <= 18 bits of it, so AW > 9 takes 2^(AW-9) blocks and a read
    // mux selected by a registered copy of the top address bits. Port A is the
    // load port on ld_clk, port B the read port on clk; the read address
    // register is the block's own, so the last sampling stage lives in the M9K.
    localparam integer NBLK = (AW > 9) ? (1 << (AW - 9)) : 1;
    localparam integer BW   = (AW > 9) ? (AW - 9) : 1;   // block-select width

    wire [AW-1:0] ra;           // address presented to the M9K address register
    generate
        if (SYNC_STAGES > 1) begin : g_pre
            reg [AW-1:0] a_pre [0:SYNC_STAGES-2];
            integer k;
            always @(posedge clk) begin
                a_pre[0] <= a;
                for (k = 1; k < SYNC_STAGES - 1; k = k + 1)
                    a_pre[k] <= a_pre[k-1];
            end
            assign ra = a_pre[SYNC_STAGES-2];
        end else begin : g_nopre
            assign ra = a;
        end
    endgenerate

    wire [17:0] dout [0:NBLK-1];
    genvar g;
    generate
        for (g = 0; g < NBLK; g = g + 1) begin : g_blk
            wire [8:0] wa = ld_addr[(AW > 9 ? 9 : AW)-1:0];
            wire [8:0] rd = ra[(AW > 9 ? 9 : AW)-1:0];
            wire       we = ld_en & ((AW > 9) ? (ld_addr[AW-1:9] == g) : 1'b1);
            wire [17:0] di = ld_data;
            alta_bram9k ram_inst (
                .DataInA(di), .DataInB(18'h3FFFF),
                .AddressA({wa, 4'b1111}), .AddressB({rd, 4'b1111}),   // x18 mode
                .DataOutA(), .DataOutB(dout[g]),
                .Clk0(ld_clk), .ClkEn0(1'b1), .AsyncReset0(1'b0),
                .Clk1(clk),    .ClkEn1(1'b1), .AsyncReset1(1'b0),
                .ByteEnA(2'b11), .ByteEnB(2'b11),
                .WeA(we), .WeB(1'b0), .ReA(1'b0), .ReB(1'b1),
                .AddressStallA(1'b0), .AddressStallB(1'b0));
            defparam ram_inst.PORTA_WIDTH     = 5'b00000;   // 512 x 18
            defparam ram_inst.PORTB_WIDTH     = 5'b00000;
            defparam ram_inst.CLKMODE         = 2'b00;      // independent ports
            defparam ram_inst.PACKEDMODE      = 1'b0;
            defparam ram_inst.PORTA_CLKIN_EN  = 1'b1;
            defparam ram_inst.PORTA_CLKOUT_EN = 1'b1;
            defparam ram_inst.PORTB_CLKIN_EN  = 1'b1;
            defparam ram_inst.PORTB_CLKOUT_EN = 1'b1;
            defparam ram_inst.PORTA_RSTIN_EN  = 1'b1;
            defparam ram_inst.PORTA_RSTOUT_EN = 1'b1;
            defparam ram_inst.PORTB_RSTIN_EN  = 1'b1;
            defparam ram_inst.PORTB_RSTOUT_EN = 1'b1;
            defparam ram_inst.PORTA_OUTREG    = 1'b0;
            defparam ram_inst.PORTB_OUTREG    = 1'b0;       // combinational read
            defparam ram_inst.PORTA_WRITETHRU = 1'b0;
            defparam ram_inst.PORTB_WRITETHRU = 1'b0;
            defparam ram_inst.INIT_VAL        = 9216'b0;
        end
        if (AW > 9) begin : g_sel
            reg [BW-1:0] blk_q;
            always @(posedge clk) blk_q <= ra[AW-1:9];
            assign d = dout[blk_q][DW-1:0];
        end else begin : g_one
            assign d = dout[0][DW-1:0];
        end
    endgenerate
`else
    reg [DW-1:0] mem [0:DEPTH-1];

    integer i;
    initial begin
        for (i = 0; i < DEPTH; i = i + 1)
            mem[i] = {DW{1'b1}};
        if (INIT_HEX != "")
            $readmemh(INIT_HEX, mem);
    end

    // Address sampling pipeline. Stage 0 captures the raw pads.
    reg [AW-1:0] a_pipe [0:SYNC_STAGES-1];

    integer k;
    always @(posedge clk) begin
        a_pipe[0] <= a;
        for (k = 1; k < SYNC_STAGES; k = k + 1)
            a_pipe[k] <= a_pipe[k-1];
    end

    always @(posedge ld_clk) begin
        if (ld_en)
            mem[ld_addr] <= ld_data;
    end

    // Unregistered array output: registered-address / combinational-read M9K.
    assign d = mem[a_pipe[SYNC_STAGES-1]];
`endif

endmodule

`default_nettype wire
