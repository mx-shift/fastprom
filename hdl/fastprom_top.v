// -----------------------------------------------------------------------------
// fastprom_top - fabric top level for a 20-pin bipolar PROM replacement
//
// Everything in the socket-facing read path lives in the AGM fabric. The
// RISC-V core is not in the path at all: in LOGIC mode it has nothing to do
// after boot, and in BRAM mode it only fills the block RAM at start-up. This
// is the whole reason the module can hit bipolar PROM access times, where a
// GPIO-polling microcontroller cannot.
//
// Parameterised so one source tree covers the 20-pin catalogue:
//   PROM_AW / DW      organisation (512x8, 256x8, 1024x4, ...)
//   NCE / CE_ACTIVE_LOW  number and polarity of chip-enable pins
//   OPEN_COLLECTOR    output stage personality
//   BANKW             extra high-order address bits that select between
//                     several images held in the fabric at once (a
//                     multi-image LOGIC build). The stack's product build is
//                     BRAM mode with BANKW = 0: firmware reads the
//                     image-select switch at boot and loads only that image.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_top #(
    parameter integer PROM_AW        = 9,       // address pins on the socket
    parameter integer DW             = 8,       // data pins on the socket
    parameter integer NCE            = 1,       // chip-enable pins
    parameter [3:0]   CE_ACTIVE_LOW  = 4'b1111,
    parameter integer OPEN_COLLECTOR = 0,
    parameter integer BANKW          = 2,       // image-select bits (0 = single image)
    parameter integer MODE_BRAM      = 0,       // 0 = LOGIC, 1 = BRAM
    parameter integer SYNC_STAGES    = 1,       // BRAM mode only
    parameter         INIT_HEX       = "",

    // Derived; not intended to be overridden. A zero-width port is illegal,
    // so a single-image build still presents one (unused) bank_sel bit.
    parameter integer BANKW_PINS     = (BANKW > 0) ? BANKW : 1
) (
    // ---- socket pins -------------------------------------------------------
    input  wire [PROM_AW-1:0] a_pin,
    input  wire [NCE-1:0]     ce_pin,
    inout  wire [DW-1:0]      d_pin,

    // ---- board-local pins (not connected to the socket) --------------------
    input  wire [BANKW_PINS-1:0] bank_sel, // image select, active high; in the
                                           // build wrapper, a register the
                                           // core writes (tools/ag32/fastprom_ip.v)
    input  wire               fabric_clk, // free-running; BRAM mode only
    input  wire               core_ready, // MCU has loaded the image

    // ---- image load port, from the RISC-V core -----------------------------
    input  wire                     ld_clk,
    input  wire                     ld_en,
    input  wire [PROM_AW+BANKW-1:0] ld_addr,
    input  wire [DW-1:0]            ld_data,

    // ---- external bus buffer ----------------------------------------------
    // The data lines leave through a 5 V-tolerant transceiver whose output
    // enable is active low, so the fabric drives the inverted sense directly
    // rather than making the board add a gate to the enable path - which is
    // the tightest path in the budget.
    output wire               buf_noe,

    // ---- open-collector build variant --------------------------------------
    // A '245 cannot emulate an open-collector part: it drives both rails and
    // its output enable is common to all eight bits. That build substitutes
    // external open-drain buffers, which want a plain data signal that is low
    // only where the part should pull the bus down.
    //
    // That signal is exactly the inverse of the per-bit output enable:
    //   ~pad_oe[i] == selected ? rom_data[i] : 1'b1
    // so a deselected part feeds all ones and every buffer floats.
    output wire [DW-1:0]      od_data,

    // ---- diagnostics -------------------------------------------------------
    output wire               selected,

    // ---- unconditional pad data, for builds whose bus enable is external ----
    // On the two-board stack the data lines reach the host only through the
    // drive buffer, whose enable is buf_noe, so the AG32's own data pads can
    // drive all the time (the vendor flow's OUTPUT pins are always enabled).
    // pad_data is what d_pin carries while driving; the enable path is then
    // buf_noe alone, and no pad output-enable (T_ZX) is in it.
    output wire [DW-1:0]      pad_data
);
    localparam integer AW = PROM_AW + BANKW;

    // Bank bits sit above the socket address bits, so image N occupies a
    // contiguous block of the stored data.
    wire [AW-1:0] rom_addr;
    generate
        if (BANKW == 0)
            assign rom_addr = a_pin;
        else
            assign rom_addr = {bank_sel[BANKW-1:0], a_pin};
    endgenerate

    wire [DW-1:0] rom_data;

    generate
        if (MODE_BRAM) begin : g_bram
            fastprom_rom_bram #(
                .AW          (AW),
                .DW          (DW),
                .SYNC_STAGES (SYNC_STAGES),
                .INIT_HEX    (INIT_HEX)
            ) u_rom (
                .clk     (fabric_clk),
                .a       (rom_addr),
                .d       (rom_data),
                .ld_clk  (ld_clk),
                .ld_en   (ld_en),
                .ld_addr (ld_addr),
                .ld_data (ld_data)
            );
        end else begin : g_logic
            // Contents are in the bitstream; the load port is unused and the
            // clock input is left dangling on purpose - LOGIC mode needs no
            // clock, so the core can be asleep or stopped.
            fastprom_rom_logic #(
                .AW       (AW),
                .DW       (DW),
                .INIT_HEX (INIT_HEX)
            ) u_rom (
                .a (rom_addr),
                .d (rom_data)
            );
        end
    endgenerate

    // ---- enable decode and output stage -----------------------------------
    wire [DW-1:0] pad_out;
    wire [DW-1:0] pad_oe;
    wire          ce_selected;

    fastprom_enable #(
        .DW             (DW),
        .NCE            (NCE),
        .CE_ACTIVE_LOW  (CE_ACTIVE_LOW),
        .OPEN_COLLECTOR (OPEN_COLLECTOR)
    ) u_en (
        .ce_pin   (ce_pin),
        .rom_data (rom_data),
        .pad_out  (pad_out),
        .pad_oe   (pad_oe),
        .selected (ce_selected)
    );

    // Hold the bus off until the image is actually present. core_ready is the
    // AHB slave's READY bit in every mode: it is clear from configuration
    // until the core sets it, so even a LOGIC build, whose data is valid as
    // soon as the fabric is configured, keeps its outputs off until firmware
    // writes READY. The product firmware sets it after loading and verifying
    // the image (BRAM mode).
    wire [DW-1:0] drive_oe = pad_oe & {DW{core_ready}};

    genvar b;
    generate
        for (b = 0; b < DW; b = b + 1) begin : g_pad
            assign d_pin[b] = drive_oe[b] ? pad_out[b] : 1'bz;
        end
    endgenerate

    assign selected = ce_selected;
    assign pad_data = pad_out;

    // Released whenever the part is deselected or the image is not yet
    // present, so a host never sees the bus driven before the data is real.
    assign buf_noe = ~(ce_selected & core_ready);

    assign od_data = ~(pad_oe & {DW{core_ready}});

endmodule

`default_nettype wire
