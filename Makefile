# FastPROM - everything is regenerable from tools/partdb.yaml and
# tools/boardmap.yaml. Nothing downstream of them is hand-maintained.
#
# The hardware is the two-board stack: fastprom-lower (pins, bus buffers)
# under fastprom-upper (the AG32).

PART  ?= 512x8
BOARD ?= fastprom-upper
KICAD ?= flatpak run --command=kicad-cli org.kicad.KiCad

# The two stack boards, lower first.
STACK := fastprom-lower fastprom-upper

.PHONY: all test sim licenses synth budget pnr pins kicad erc stack grade pdf netlist clean help dfu-fabric \
	layout reroute layout-drc jlc refnet panel jlc-panel \
	fw-image fw-fixtures fw-test fw-build fw-lint fw-table fw-batch fw-flash fw-flash-table \
	fw-clean web web-bundle web-test

all: test synth budget pins kicad erc stack grade

help:
	@echo "make test     simulation suite + firmware host tests"
	@echo "make fw-image   container image for the Rust firmware (tools/rust/Containerfile)"
	@echo "make fw-test    firmware host tests (cargo test; fixtures from zlib and mkimagetable)"
	@echo "make fw-build   AG32 firmware -> build/fw/fastprom-{dfu,ag32,recovery}.{elf,bin} and a layout report"
	@echo "make fw-lint    cargo fmt --check, clippy -D warnings (host and riscv32), cargo-cycles"
	@echo "make fw-table   image table build/fw/images.bin (FW_IMAGES=\"a.bin b.bin\")"
	@echo "make fw-batch   combined flash image: DFU stage, application, fabric, manifest, table for PART"
	@echo "make web        update page -> build/web (WebAssembly, parts.json, update bundle)"
	@echo "make web-bundle firmware build, then the update bundle -> build/web/bundle"
	@echo "make web-test   the page in headless Chromium against the simulated board (Playwright)"
	@echo "make fw-flash   program the combined image over UART0 (PORT=/dev/ttyUSB0, BOOT0 high)"
	@echo "make synth    fabric cost per organisation -> docs/synth-results.md"
	@echo "make budget   timing budget -> docs/03-timing-budget.md"
	@echo "make pnr      Supra place-and-route timing (AGRV_PIO=<AgRV_pio SDK>) -> tools/ag32/pnr-results.json"
	@echo "make dfu-fabric  the DFU stage's USB-only fabric (AGRV_PIO=...), checked against fw/fastprom-dfu/fabric/"
	@echo "make pins     pin constraints and HDL params for PART=$(PART)"
	@echo "make refnet   regenerate the reference netlists in docs/netlist/, then make kicad"
	@echo "make kicad    draw both stack schematics (generated, not tracked)"
	@echo "make netlist  export each sheet's netlist and BOM into hw/ (not tracked)"
	@echo "make erc      electrical rule check on both schematics"
	@echo "make stack    verify both halves agree on the connector pinout"
	@echo "make grade    grade the stack sheets against hand-drafting rules"
	@echo "make pdf      schematic and routed-board PDFs into build/pdf/ (not tracked)"
	@echo "make layout   build the two stack boards from placement + committed routing"
	@echo "make reroute  route them again with Freerouting (after placement changes)"
	@echo "make layout-drc  DRC both routed boards, zero violations required"
	@echo "make jlc      gerbers, drill, BOM and CPL for JLCPCB in build/jlc/"
	@echo "make panel    assembly panel of matched pairs -> hw/fastprom-panel.kicad_pcb (DRC)"
	@echo "make jlc-panel  the panel's gerbers, drill, BOM and CPL in build/jlc/fastprom-panel/"
	@echo "make licenses  check every file's licence (REUSE.toml, reuse lint)"
	@echo "make all      test synth budget pins kicad erc stack grade"

# The simulation suite needs iverilog; without it on PATH it runs in the
# hdlc/sim image, under FW_ENGINE (podman here, docker on CI runners). Its
# firmware-trace benches replay what `make fw-test` recorded in build/fw/
# (scripts/fw_tests.sh), so run that first where there is no cargo.
SIM_IMAGE ?= docker.io/hdlc/sim:osvb
SIM_RUN    = $(if $(shell command -v iverilog),,$(FW_ENGINE) run --rm \
	$(if $(filter docker,$(FW_ENGINE)),--user $(shell id -u):$(shell id -g),--security-opt label=disable --userns=keep-id) \
	-v $(CURDIR):/w -w /w $(SIM_IMAGE))

test sim:
	$(SIM_RUN) bash scripts/run_sim.sh

# Every tracked file's licence and copyright, per REUSE.toml (LICENSE has
# the same in prose), checked by FSFE's reuse tool.
REUSE_IMAGE ?= docker.io/fsfe/reuse:5.1.1
licenses:
	$(FW_ENGINE) run --rm $(if $(filter docker,$(FW_ENGINE)),,--security-opt label=disable) \
		-v $(CURDIR):/data:ro $(REUSE_IMAGE) lint

# ---- firmware (Rust, fw/) ---------------------------------------------------
# Built in a container (tools/rust/Containerfile: cosmic-toad's toolchain
# plus llvm-tools). The repo is mounted so the tests reach build/fw/.
# docs/06-build-and-flash.md has the memory map and the flash procedure.
FW_IMAGE  ?= fastprom-rust
FW_TARGET := riscv32imafc-unknown-none-elf
# FW_ENGINE=docker for docker (as on the CI runner), run as the caller so
# the build's files are theirs; podman's rootless mapping already does that.
FW_ENGINE ?= podman
FW_USER    = $(if $(filter docker,$(FW_ENGINE)),--user $(shell id -u):$(shell id -g),--security-opt label=disable)
FW_RUN     = $(FW_ENGINE) run --rm $(FW_USER) -v $(CURDIR):/w -w /w/fw \
	-e CARGO_HOME=/w/fw/.cargo-home $(FW_IMAGE)
FW_ELF    := fw/target/$(FW_TARGET)/release/fastprom-ag32
DFU_ELF   := fw/target/$(FW_TARGET)/release/fastprom-dfu
REC_ELF   := fw/target/$(FW_TARGET)/release/fastprom-recovery
FW_OUT    := build/fw
# The flash map (docs/10-updates.md). fw/fastprom-core/src/memmap.rs is the
# source; tools/fw_layout.py --batch fails if the combined image disagrees
# with the ELFs, which record it.
DFU_ADDR      := 0x80000000
FW_ADDR       := 0x80008000
LOGIC_ADDR    := 0x80014000
MANIFEST_ADDR := 0x8002D000
TABLE_ADDR    := 0x8002E000
# The BRAM-mode fabric bitstream for PART, from tools/ag32/run_flow.sh.
FW_LOGIC  ?= build/ag32/$(PART)-bram/logic/top.bin
FW_BATCH  := $(FW_OUT)/fastprom-$(PART)-batch.bin
# Images for the table, in bank order; empty builds a test pattern.
FW_IMAGES ?=
GEN_BATCH  = "$(AGRV_PIO)/python_dist/python3" "$(AGRV_PIO)/platforms/AgRV/etc/gen_batch"
FLASHER    = "$(AGRV_PIO)/packages/tool-agrv_flashloader/bin/agrv32flash_cmd"
PORT      ?= /dev/ttyUSB0

fw-image:
	$(FW_ENGINE) build -q -t $(FW_IMAGE) -f tools/rust/Containerfile tools/rust

fw-fixtures:
	python3 fw/tools/gen_fixtures.py $(FW_OUT)/fixtures

fw-test:
	FW_IMAGE=$(FW_IMAGE) FW_ENGINE=$(FW_ENGINE) scripts/fw_tests.sh

# The DFU stage, the application and the recovery loader. The
# application's header, and the loader's, get their lengths and CRCs after
# linking (tools/fw_stamp.py, in the ELF), then all three become raw images:
# the first two for AGM's loader (fw-batch) and the page's recovery, the
# loader for the page's recovery, which uploads it into SRAM.
FW_LAYOUT = python3 tools/fw_layout.py --elf $(FW_OUT)/fastprom-ag32.elf --bin $(FW_OUT)/fastprom-ag32.bin \
	--dfu-elf $(FW_OUT)/fastprom-dfu.elf --dfu-bin $(FW_OUT)/fastprom-dfu.bin \
	--recovery-elf $(FW_OUT)/fastprom-recovery.elf --recovery-bin $(FW_OUT)/fastprom-recovery.bin

fw-build:
	$(FW_RUN) cargo build -q --release --target $(FW_TARGET) -p fastprom-ag32 -p fastprom-dfu \
		-p fastprom-recovery
	@mkdir -p $(FW_OUT)
	$(FW_RUN) sh -c 'cp /w/$(FW_ELF) /w/$(FW_OUT)/fastprom-ag32.elf && \
		cp /w/$(DFU_ELF) /w/$(FW_OUT)/fastprom-dfu.elf && \
		cp /w/$(REC_ELF) /w/$(FW_OUT)/fastprom-recovery.elf'
	python3 tools/fw_stamp.py app --elf $(FW_OUT)/fastprom-ag32.elf
	python3 tools/fw_stamp.py loader --elf $(FW_OUT)/fastprom-recovery.elf
	$(FW_RUN) sh -c 'llvm-objcopy -O binary /w/$(FW_OUT)/fastprom-ag32.elf /w/$(FW_OUT)/fastprom-ag32.bin && \
		llvm-objcopy -O binary /w/$(FW_OUT)/fastprom-dfu.elf /w/$(FW_OUT)/fastprom-dfu.bin && \
		llvm-objcopy -O binary /w/$(FW_OUT)/fastprom-recovery.elf /w/$(FW_OUT)/fastprom-recovery.bin'
	@$(FW_LAYOUT) \
		$(if $(wildcard $(FW_LOGIC)),--fabric $(FW_LOGIC)) \
		$(if $(wildcard $(FW_OUT)/images.bin),--table $(FW_OUT)/images.bin) \
		> $(FW_OUT)/layout.txt; s=$$?; cat $(FW_OUT)/layout.txt; exit $$s

# The web crates are std (fastprom-host) and WebAssembly (fastprom-web): the
# riscv32 pass leaves them out, and a wasm32 pass lints them as the page
# builds them.
fw-lint:
	$(FW_RUN) sh -c 'cargo fmt --check && \
		cargo clippy -q --all-targets -- -D warnings && \
		cargo clippy -q --target $(FW_TARGET) --workspace \
			--exclude fastprom-host --exclude fastprom-web -- -D warnings && \
		cargo clippy -q --target $(WEB_TARGET) -p fastprom-host -p fastprom-web -- -D warnings && \
		cargo-cycles'

# ---- the update page (web/, docs/12-web.md) ------------------------------------
# Static files into build/web: web/'s HTML, CSS and JS, parts.json from
# tools/partdb.yaml, the WebAssembly (fastprom-web, wasm-bindgen --target
# web) in pkg/, and the update bundle in bundle/ (the application from
# make fw-build, if built, and each part's fabric found in build/ag32).
WEB_TARGET := wasm32-unknown-unknown
WEB_OUT    := build/web
WEB_WASM   := fw/target/$(WEB_TARGET)/web/fastprom_web.wasm
# Where each part's BRAM-mode bitstream is looked for (tools/ag32/run_flow.sh).
WEB_FABRICS ?= build/ag32
# The browser tests: Playwright's own image, pinned; the npm package in
# web/tests/package-lock.json is the same version.
PLAYWRIGHT_IMAGE ?= mcr.microsoft.com/playwright:v1.64.0-noble
WEB_TEST_OUT := build/web-test

web:
	@mkdir -p $(WEB_OUT)/pkg
	$(FW_RUN) sh -c 'cargo build -q --profile web --target $(WEB_TARGET) -p fastprom-web && \
		wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section --out-dir /w/$(WEB_OUT)/pkg /w/$(WEB_WASM)'
	python3 tools/mkweb.py --out $(WEB_OUT)
	@if [ -f $(FW_OUT)/fastprom-ag32.bin ]; then \
		python3 tools/mkbundle.py --app $(FW_OUT)/fastprom-ag32.bin --dfu $(FW_OUT)/fastprom-dfu.bin \
			--loader $(FW_OUT)/fastprom-recovery.bin --fabric-dir $(WEB_FABRICS) --out $(WEB_OUT)/bundle; \
	else \
		echo "no $(FW_OUT)/fastprom-ag32.bin (make fw-build): the page carries no firmware"; \
		python3 tools/mkbundle.py --empty --out $(WEB_OUT)/bundle; \
	fi
	@ls -l $(WEB_OUT)/pkg | awk 'NR > 1 { print "  " $$5 "\t" $$9 }'

# The bundle from a fresh firmware build.
web-bundle: fw-build
	python3 tools/mkbundle.py --app $(FW_OUT)/fastprom-ag32.bin --dfu $(FW_OUT)/fastprom-dfu.bin \
		--loader $(FW_OUT)/fastprom-recovery.bin --fabric-dir $(WEB_FABRICS) --out $(WEB_OUT)/bundle

# The page in headless Chromium against the simulated board, with synthetic
# bundles for board revisions A and B (web/tests/).
web-test: web
	@rm -rf $(WEB_TEST_OUT) && mkdir -p $(WEB_TEST_OUT)
	cp -r $(WEB_OUT)/. $(WEB_TEST_OUT)/site
	python3 tools/mkbundle.py --synthetic --version 0.2.0 --board-rev A --out $(WEB_TEST_OUT)/site/test-bundle-A > /dev/null
	python3 tools/mkbundle.py --synthetic --version 0.2.0 --board-rev B --out $(WEB_TEST_OUT)/site/test-bundle-B > /dev/null
	$(FW_ENGINE) run --rm $(FW_USER) --ipc=host -v $(CURDIR):/w -w /w/web/tests \
		-e HOME=/tmp -e npm_config_cache=/w/$(WEB_TEST_OUT)/npm-cache \
		$(PLAYWRIGHT_IMAGE) sh -c 'npm ci --silent --no-audit --no-fund && npx playwright test'

fw-clean:
	$(FW_RUN) cargo clean

# The image table: FW_IMAGES in bank order (1, 2 or 4 of them), at PART's
# organisation from tools/partdb.yaml.
fw-table:
	@mkdir -p $(FW_OUT)
	@set -e; org=$$(python3 -c "import yaml; p = yaml.safe_load(open('tools/partdb.yaml'))['parts']['$(PART)']; print(p['depth'], p['width'])"); \
	depth=$${org% *}; width=$${org#* }; images="$(FW_IMAGES)"; \
	if [ -z "$$images" ]; then \
		python3 tools/mkimage.py --kind palette --depth $$depth --width $$width \
			--out $(FW_OUT)/pattern.hex > /dev/null; \
		images=$(FW_OUT)/pattern.hex; \
		echo "FW_IMAGES not set: a test-pattern table for $(PART)"; \
	fi; \
	python3 tools/mkimagetable.py $$(for i in $$images; do printf -- '--image %s ' $$i; done) \
		--part $(PART) --out $(FW_OUT)/images.bin

# One file for AGM's serial loader, as AgRV_pio's "Create Batch" builds it:
# option bytes (the fabric configuration address), the fabric bitstream,
# the DFU stage, the application, the fabric's manifest and the image
# table, each behind gen_batch's download header.
fw-batch: fw-table fw-build
	@test -f $(FW_LOGIC) || { echo "no fabric bitstream $(FW_LOGIC): run"; \
		echo "  tools/ag32/run_flow.sh $(AGRV_PIO) $(PART) bram build/ag32/$(PART)-bram"; exit 1; }
	python3 tools/fw_stamp.py manifest --fabric $(FW_LOGIC) --part $(PART) \
		--out $(FW_OUT)/manifest.bin
	$(GEN_BATCH) --flash-size 262144 -d 0x40200001 \
		--logic-config $(FW_LOGIC) --logic-address $(LOGIC_ADDR) \
		-i $(FW_OUT)/fastprom-dfu.bin $(FW_OUT)/fastprom-ag32.bin $(FW_OUT)/manifest.bin \
		   $(FW_OUT)/images.bin \
		-a $(DFU_ADDR) $(FW_ADDR) $(MANIFEST_ADDR) $(TABLE_ADDR) \
		-o $(FW_BATCH) > $(FW_OUT)/batch.log
	@$(FW_LAYOUT) --fabric $(FW_LOGIC) --manifest $(FW_OUT)/manifest.bin \
		--table $(FW_OUT)/images.bin --batch $(FW_BATCH) \
		> $(FW_OUT)/layout.txt; s=$$?; cat $(FW_OUT)/layout.txt; exit $$s

# Over UART0 with BOOT0 held high (J3), with the flags AgRV_pio's
# batch_upload target passes (platforms/AgRV/builder/main.py). Not run here:
# there is no hardware on this bench.
fw-flash: fw-batch
	$(FLASHER) -m 8e1 -G 1 -T 0x20010000 -L -B 460800 -X 0x10 -b 460800 -R -Z \
		-N $(FW_BATCH) $(PORT)

# Images only: rewrite the table region, leaving firmware and fabric alone
# (AgRV_pio's serial upload flags, at the table's address).
fw-flash-table: fw-table
	$(FLASHER) -m 8e1 -G 1 -T 0x20010000 -B 460800 -X 0x10 -b 460800 -R \
		-S $(TABLE_ADDR) -v -w $(FW_OUT)/images.bin $(PORT)

synth:
	python3 tools/synth_matrix.py

budget:
	python3 tools/timing_budget.py

# Place and route the fabric on the stack's QFN-32 with AGM's native flow and
# collect the timing the budget uses. Needs the AgRV_pio SDK and the podman
# image from tools/ag32/Containerfile; see docs/06-build-and-flash.md.
AGRV_PIO ?= /opt/agm/AgRV_pio
pnr:
	sh tools/ag32/run_matrix.sh $(AGRV_PIO)
	python3 tools/timing_budget.py

# The DFU stage's own fabric (clocks and USB0, no pins), built and checked
# against the committed fw/fastprom-dfu/fabric/; run the script with
# --install to replace it. docs/06-build-and-flash.md, "The DFU stage's fabric".
dfu-fabric:
	sh tools/ag32/build_dfu_fabric.sh $(AGRV_PIO)

pins:
	python3 tools/gen_pins.py --part $(PART) --board $(BOARD) --docs

# ---- the stack's KiCad files -----------------------------------------------
# Generated, never tracked: each stack board's schematic and project
# (gen_wired.py), its netlist and BOM (KiCad), the routed boards
# (gen_layout.py, from the netlists and the committed routing) and the panel
# (gen_panel.py). Each is rebuilt when a generator or table under tools/,
# the footprint library, the library tables, the revision or a board's
# routing changes, so a fresh checkout builds them on first use.
HW_SRC := $(wildcard tools/*.py tools/*.yaml tools/layout/*.py) \
	hw/REVISION hw/sym-lib-table hw/fp-lib-table \
	$(wildcard hw/lib/fastprom.pretty/*.kicad_mod)
SCH    := $(STACK:%=hw/%.kicad_sch)
NETS   := $(STACK:%=hw/%.net)
PCBS   := $(STACK:%=hw/%.kicad_pcb)
ROUTES := $(STACK:%=hw/%.route.json)
# a generator that fails leaves no half-written file behind
.DELETE_ON_ERROR:

kicad: $(SCH)

hw/%.kicad_sch: $(HW_SRC)
	python3 tools/gen_wired.py --board $*

erc: kicad
	@for b in $(STACK); do \
		printf '%-28s ' "$$b"; \
		$(KICAD) sch erc --severity-all --output hw/erc-$$b.rpt \
			hw/$$b.kicad_sch > /dev/null; \
		grep -E "ERC messages" hw/erc-$$b.rpt; \
	done

# Schematics, and the routed boards' copper and silkscreen.
pdf: kicad layout
	@mkdir -p build/pdf
	@for b in $(STACK); do \
		printf '%-28s ' "$$b-sch"; \
		$(KICAD) sch export pdf --output build/pdf/$$b-sch.pdf \
			hw/$$b.kicad_sch | tail -1; \
		printf '%-28s ' "$$b-pcb"; \
		$(KICAD) pcb export pdf --output build/pdf/$$b-pcb.pdf \
			--layers Edge.Cuts,F.Cu,In1.Cu,In2.Cu,B.Cu,F.SilkS,B.SilkS \
			--mode-multipage --include-border-title \
			hw/$$b.kicad_pcb | tail -1; \
	done
	@ls -1 build/pdf/

# The two stack boards are drawn with wires only; grade them against the
# hand-drafting rules and confirm the drawing did not change connectivity.
# The reference netlists live in docs/netlist/.
CONV := $(STACK)

# Regenerate those reference netlists from tools/gen_kicad.py after a circuit
# change, then redraw the stack sheets (gen_kicad writes the same paths, so
# its sheet is removed once exported).
refnet:
	@for b in $(CONV); do \
		python3 tools/gen_kicad.py --board $$b > /dev/null && \
		$(KICAD) sch export netlist --output docs/netlist/$$b.net \
			hw/$$b.kicad_sch > /dev/null && rm -f hw/$$b.kicad_sch && \
		echo "docs/netlist/$$b.net"; \
	done
	$(MAKE) kicad

grade: netlist
	@for b in $(CONV); do \
		printf '%-28s ' "$$b"; \
		python3 tools/grade_schematic.py hw/$$b.kicad_sch --quiet; \
	done
	@for b in $(CONV); do \
		printf '%-28s ' "$$b"; \
		python3 tools/diff_netlist.py docs/netlist/$$b.net hw/$$b.net \
			| tail -1; \
	done

stack: netlist
	python3 tools/check_stack.py

netlist: $(NETS)

hw/%.net: hw/%.kicad_sch
	$(KICAD) sch export netlist --output $@ $<
	$(KICAD) sch export bom --output hw/$*-bom.csv \
		--fields 'Reference,Value,Footprint,$${QUANTITY}' \
		--group-by Value $<

# ---- the routed stack ------------------------------------------------------
# Layout runs in KiCad's own Python, from the official container; the
# netlists it reads are the exported ones that `make grade` signs off.
KPY := scripts/kicad.sh py
KCLI := scripts/kicad.sh cli

# Place the boards and lay their committed routing (hw/<board>.route.json)
# onto them. No autorouter: the routing is data, redone only by `reroute`.
layout: $(PCBS)

$(PCBS) &: $(NETS) $(ROUTES) $(HW_SRC)
	$(KPY) tools/gen_layout.py

# Route again from scratch: place and pour (--dsn), Freerouting, then import,
# tidy and widen its result into hw/<board>.route.json (--ses), and rebuild.
# Needed after any placement or netlist change; `layout` refuses until then.
# (not GND: gen_layout.py --dsn hands Freerouting the ground copper fixed,
# with nothing left for it to route)
PLANES_fastprom-lower        =
PLANES_fastprom-upper  =
reroute: netlist
	$(KPY) tools/gen_layout.py --dsn
	@for b in $(STACK); do \
		case $$b in \
		fastprom-lower) p='$(PLANES_fastprom-lower)' ;; \
		fastprom-upper) p='$(PLANES_fastprom-upper)' ;; \
		esac; \
		scripts/freerouting.sh build/fr/$$b.dsn build/fr/$$b.ses "$$p" || exit 1; \
	done
	$(KPY) tools/gen_layout.py --ses
	$(MAKE) layout

# DRC each board, then its pin 1 marks: every one clear of every part's
# body on its side (tools/check_pin1.py), so none is hidden once the parts
# are on.
layout-drc: layout
	@for b in $(STACK); do \
		$(KCLI) pcb drc --severity-error --severity-warning \
			--schematic-parity --exit-code-violations \
			--output hw/drc-$$b.rpt hw/$$b.kicad_pcb > /dev/null \
			&& echo "$$b: DRC clean" \
			|| { echo "$$b: DRC FAILED, see hw/drc-$$b.rpt"; exit 1; }; \
	done
	@$(KPY) tools/check_pin1.py $(STACK:%=hw/%.kicad_pcb)

jlc: layout-drc
	@for b in $(STACK); do \
		d=build/jlc/$$b; rm -rf $$d; mkdir -p $$d/gerbers; \
		cu=F.Cu,In1.Cu,In2.Cu,B.Cu; \
		$(KCLI) pcb export gerbers --output $$d/gerbers/ \
			--layers $$cu,F.Paste,B.Paste,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts \
			--subtract-soldermask hw/$$b.kicad_pcb > /dev/null; \
		$(KCLI) pcb export drill --output $$d/gerbers/ --format excellon \
			--excellon-units mm --excellon-separate-th hw/$$b.kicad_pcb > /dev/null; \
		$(KCLI) pcb export pos --output $$d/pos.csv --format csv --units mm \
			--side both hw/$$b.kicad_pcb > /dev/null; \
		python3 tools/jlc_fab.py $$b; \
	done

# The assembly panel: matched lower+upper pairs, tab-routed, built from the
# routed boards (read only) by tools/gen_panel.py; see DESIGN_NOTES.md
# "Panel" and docs/07-bom.md for ordering. PAIRS=N (jlc-panel) lists only
# the first N pairs of each panel in the BOM and CPL, for a prototype order. A panel has no schematic, so its
# DRC runs without the parity check the boards get.
PANEL := fastprom-panel
hw/$(PANEL).kicad_pcb: $(PCBS) $(HW_SRC)
	$(KPY) tools/gen_panel.py

panel: layout-drc hw/$(PANEL).kicad_pcb
	@$(KCLI) pcb drc --severity-error --severity-warning --exit-code-violations \
		--output hw/drc-$(PANEL).rpt hw/$(PANEL).kicad_pcb > /dev/null \
		&& echo "$(PANEL): DRC clean" \
		|| { echo "$(PANEL): DRC FAILED, see hw/drc-$(PANEL).rpt"; exit 1; }

jlc-panel: panel
	@d=build/jlc/$(PANEL); rm -rf $$d; mkdir -p $$d/gerbers; \
	cu=F.Cu,In1.Cu,In2.Cu,B.Cu; \
	$(KCLI) pcb export gerbers --output $$d/gerbers/ \
		--layers $$cu,F.Paste,B.Paste,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts \
		--subtract-soldermask hw/$(PANEL).kicad_pcb > /dev/null; \
	$(KCLI) pcb export drill --output $$d/gerbers/ --format excellon \
		--excellon-units mm --excellon-separate-th hw/$(PANEL).kicad_pcb > /dev/null; \
	$(KCLI) pcb export pos --output $$d/pos.csv --format csv --units mm \
		--side both hw/$(PANEL).kicad_pcb > /dev/null; \
	python3 tools/jlc_fab.py $(PANEL) --from $(STACK) $(if $(PAIRS),--pairs $(PAIRS))

clean:
	rm -rf build gen/*.v gen/*.pcf gen/*.ve gen/*.vh hw/erc-*.rpt hw/drc*.rpt \
		hw/*.kicad_sch hw/*.kicad_pro hw/*.kicad_prl hw/*.kicad_pcb \
		hw/*.kicad_dru hw/*.net hw/*-bom.csv
