// -----------------------------------------------------------------------------
// fastprom_ahb_slave - the RISC-V core's window into the fabric
//
// The AG32's hard core exposes an AHB-Lite master to the fabric, mapped at
// 0x60000000. This is the slave on the other end of it: three registers, which
// is all the firmware needs.
//
//   +0x00  CTRL   W/R  bit 0  READY - allow the socket outputs to drive
//   +0x04  ADDR   W/R         load pointer into the image store
//   +0x08  DATA   W           write one word at ADDR, then ADDR++
//   +0x0C  ID     R           magic and format version, for a sanity check
//
// DATA auto-increments the pointer so loading an image is one store per word
// with no address bookkeeping, which matters because the core does this at
// every power-on before it can let the outputs go live.
//
// Zero wait states: HREADYOUT is tied high. The store is a single register
// write and there is nothing to stall for.
//
// Deliberately small. The vendor's own AHB-to-RAM bridge example consumes most
// of the fabric, and the fabric here is needed for the ROM.
// -----------------------------------------------------------------------------
`default_nettype none

module fastprom_ahb_slave #(
    parameter integer AW = 11,
    parameter integer DW = 8
) (
    input  wire        hclk,
    input  wire        hresetn,

    // ---- AHB-Lite slave ----------------------------------------------------
    input  wire        hsel,
    input  wire [31:0] haddr,
    input  wire        hwrite,
    input  wire [1:0]  htrans,
    input  wire [31:0] hwdata,
    output wire [31:0] hrdata,
    output wire        hreadyout,
    output wire [1:0]  hresp,

    // ---- to the ROM store --------------------------------------------------
    output wire          ld_en,
    output wire [AW-1:0] ld_addr,
    output wire [DW-1:0] ld_data,

    // ---- to the output stage ----------------------------------------------
    output wire          core_ready
);
    localparam [1:0] HTRANS_IDLE = 2'b00;
    localparam [1:0] HRESP_OKAY  = 2'b00;

    localparam [3:0] REG_CTRL = 4'h0;
    localparam [3:0] REG_ADDR = 4'h4;
    localparam [3:0] REG_DATA = 4'h8;
    localparam [3:0] REG_ID   = 4'hC;

    // 'FPRM' and format version 1, matching tools/mkimagetable.py.
    localparam [31:0] ID_VALUE = 32'h4650524D;

    assign hreadyout = 1'b1;
    assign hresp     = HRESP_OKAY;

    // AHB splits a transfer into an address phase and a data phase, so the
    // address-phase controls have to be held for use alongside HWDATA.
    reg        ap_valid;
    reg        ap_write;
    reg [3:0]  ap_reg;

    wire transfer = hsel && (htrans != HTRANS_IDLE) && htrans[1];

    always @(posedge hclk or negedge hresetn) begin
        if (!hresetn) begin
            ap_valid <= 1'b0;
            ap_write <= 1'b0;
            ap_reg   <= 4'h0;
        end else begin
            ap_valid <= transfer;
            ap_write <= hwrite;
            ap_reg   <= {haddr[3:2], 2'b00};
        end
    end

    wire wr = ap_valid && ap_write;

    reg          ready_r;
    reg [AW-1:0] ptr_r;
    reg          ld_en_r;
    reg [DW-1:0] ld_data_r;

    always @(posedge hclk or negedge hresetn) begin
        if (!hresetn) begin
            // Outputs must come up released: at power-on the image is not
            // loaded yet, and driving a host's bus with whatever the block RAM
            // happens to contain is the one thing this must never do.
            ready_r   <= 1'b0;
            ptr_r     <= {AW{1'b0}};
            ld_en_r   <= 1'b0;
            ld_data_r <= {DW{1'b0}};
        end else begin
            ld_en_r <= 1'b0;

            if (wr) begin
                case (ap_reg)
                    REG_CTRL: ready_r <= hwdata[0];
                    REG_ADDR: ptr_r   <= hwdata[AW-1:0];
                    REG_DATA: begin
                        ld_data_r <= hwdata[DW-1:0];
                        ld_en_r   <= 1'b1;
                        ptr_r     <= ptr_r + 1'b1;
                    end
                    default: ;
                endcase
            end
        end
    end

    // ld_addr must be the pointer as it was when DATA was written, not the
    // already-incremented value.
    reg [AW-1:0] ld_addr_r;
    always @(posedge hclk or negedge hresetn) begin
        if (!hresetn)
            ld_addr_r <= {AW{1'b0}};
        else if (wr && ap_reg == REG_DATA)
            ld_addr_r <= ptr_r;
    end

    assign ld_en      = ld_en_r;
    assign ld_addr    = ld_addr_r;
    assign ld_data    = ld_data_r;
    assign core_ready = ready_r;

    // Read data is captured in the ADDRESS phase, from haddr, so that it is
    // already valid when the data phase begins. Capturing it in the data
    // phase instead would present it a cycle late, which a zero-wait-state
    // slave has no way to signal.
    reg [31:0] rdata_r;
    always @(posedge hclk or negedge hresetn) begin
        if (!hresetn)
            rdata_r <= 32'h0;
        else if (transfer && !hwrite) begin
            case ({haddr[3:2], 2'b00})
                REG_CTRL: rdata_r <= {31'h0, ready_r};
                REG_ADDR: rdata_r <= {{(32-AW){1'b0}}, ptr_r};
                REG_ID:   rdata_r <= ID_VALUE;
                default:  rdata_r <= 32'h0;
            endcase
        end
    end

    assign hrdata = rdata_r;

endmodule

`default_nettype wire
