// -----------------------------------------------------------------------------
// fastprom_ip - FastPROM's fabric as the AG32 "macro IP" for AGM's native flow
//
// tools/ag32/run_flow.sh builds this with AGM's gen_vlog / af_map / Supra. The
// port list is the one gen_vlog's template (logic/fastprom_ip_tmpl.v) demands
// for the .ve that tools/gen_pins.py writes: every socket and board pin by its
// fastprom_top name, then the clocks and the CPU's AHB ports.
//
// The configuration comes from macros that run_flow.sh prepends:
//   FP_PROM_AW, FP_DW, FP_NCE, FP_CE_ACTIVE_LOW   from <part>_params.vh
//   FP_MODE_BRAM, FP_SYNC_STAGES, FP_BANKW
//
// Two choices here are about the as-built stack, not about the core:
//   * d_pin is driven unconditionally (pad_data). The .ve declares the data
//     pins OUTPUT, which gen_vlog makes always-enabled pads; on the stack the
//     host only sees them through the drive buffer, whose enable is buf_noe.
//   * bank_sel, for a multi-image LOGIC build (FP_BANKW > 0), comes from a
//     register the core writes at offset 0x10. The stack reads its image
//     switch through the UART pins in firmware, so the bits have no fabric
//     pad; the register is what makes them real logic for timing (constants
//     would be optimised away). Not yet in fastprom_ahb_slave.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_ip (
  input       [`FP_PROM_AW-1:0] a_pin,
  output tri0        buf_noe,
  input       [`FP_NCE-1:0] ce_pin,
`ifdef FP_RDCLK
  input              rd_clk_out_data,   // BRAM read clock, PLL output 1 (RDCLK)
  input              rd_clk_out_en,
`endif
  output tri0 [`FP_DW-1:0]  d_pin,
  input              sys_clock,
  input              bus_clock,
  input              resetn,
  input              stop,
  input       [1:0]  mem_ahb_htrans,
  input              mem_ahb_hready,
  input              mem_ahb_hwrite,
  input       [31:0] mem_ahb_haddr,
  input       [2:0]  mem_ahb_hsize,
  input       [2:0]  mem_ahb_hburst,
  input       [31:0] mem_ahb_hwdata,
  output tri1        mem_ahb_hreadyout,
  output tri0        mem_ahb_hresp,
  output tri0 [31:0] mem_ahb_hrdata,
  output tri0        slave_ahb_hsel,
  output tri1        slave_ahb_hready,
  input              slave_ahb_hreadyout,
  output tri0 [1:0]  slave_ahb_htrans,
  output tri0 [2:0]  slave_ahb_hsize,
  output tri0 [2:0]  slave_ahb_hburst,
  output tri0        slave_ahb_hwrite,
  output tri0 [31:0] slave_ahb_haddr,
  output tri0 [31:0] slave_ahb_hwdata,
  input              slave_ahb_hresp,
  input       [31:0] slave_ahb_hrdata,
  output tri0 [3:0]  ext_dma_DMACBREQ,
  output tri0 [3:0]  ext_dma_DMACLBREQ,
  output tri0 [3:0]  ext_dma_DMACSREQ,
  output tri0 [3:0]  ext_dma_DMACLSREQ,
  input       [3:0]  ext_dma_DMACCLR,
  input       [3:0]  ext_dma_DMACTC,
  output tri0 [3:0]  local_int
);
    localparam integer BANKW      = `FP_BANKW;
    localparam integer BANKW_PINS = (BANKW > 0) ? BANKW : 1;
    localparam integer AW         = `FP_PROM_AW + BANKW;

    // The CPU's AHB window (mem_ahb) runs on sys_clock.
    wire          ld_en;
    wire [AW-1:0] ld_addr;
    wire [`FP_DW-1:0] ld_data;
    wire          core_ready;
    wire [1:0]    hresp2;

    fastprom_ahb_slave #(.AW(AW), .DW(`FP_DW)) u_ahb (
        .hclk      (sys_clock),
        .hresetn   (resetn),
        .hsel      (mem_ahb_hready),
        .haddr     (mem_ahb_haddr),
        .hwrite    (mem_ahb_hwrite),
        .htrans    (mem_ahb_htrans),
        .hwdata    (mem_ahb_hwdata),
        .hrdata    (mem_ahb_hrdata),
        .hreadyout (mem_ahb_hreadyout),
        .hresp     (hresp2),
        .ld_en     (ld_en),
        .ld_addr   (ld_addr),
        .ld_data   (ld_data),
        .core_ready(core_ready)
    );
    assign mem_ahb_hresp = hresp2[0];

    // Image-select register (multi-image LOGIC timing builds only).
    reg [BANKW_PINS-1:0] bank_r;
    reg                  bank_wr;
    always @(posedge sys_clock or negedge resetn) begin
        if (!resetn) begin
            bank_wr <= 1'b0;
            bank_r  <= {BANKW_PINS{1'b0}};
        end else begin
            bank_wr <= mem_ahb_hready && mem_ahb_htrans[1] && mem_ahb_hwrite
                       && mem_ahb_haddr[4];
            if (bank_wr)
                bank_r <= mem_ahb_hwdata[BANKW_PINS-1:0];
        end
    end

    wire [`FP_DW-1:0] d_unused;
    wire [`FP_DW-1:0] od_unused;
    wire              sel_unused;

    fastprom_top #(
        .PROM_AW        (`FP_PROM_AW),
        .DW             (`FP_DW),
        .NCE            (`FP_NCE),
        .CE_ACTIVE_LOW  (`FP_CE_ACTIVE_LOW),
        .OPEN_COLLECTOR (0),
        .BANKW          (BANKW),
        .MODE_BRAM      (`FP_MODE_BRAM),
        .SYNC_STAGES    (`FP_SYNC_STAGES)
    ) u_top (
        .a_pin      (a_pin),
        .ce_pin     (ce_pin),
        .d_pin      (d_unused),
        .bank_sel   (bank_r),
`ifdef FP_RDCLK
        .fabric_clk (rd_clk_out_data),
`else
        .fabric_clk (sys_clock),
`endif
        .core_ready (core_ready),
        .ld_clk     (sys_clock),
        .ld_en      (ld_en),
        .ld_addr    (ld_addr),
        .ld_data    (ld_data),
        .buf_noe    (buf_noe),
        .od_data    (od_unused),
        .selected   (sel_unused),
        .pad_data   (d_pin)
    );

    assign slave_ahb_hready  = 1'b1;
endmodule

`default_nettype wire
