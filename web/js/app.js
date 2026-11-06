// The FastPROM update page. The protocol, the file checks, the image table
// and the update and recovery procedures are Rust (fastprom-host, through
// fastprom-web's WebAssembly); this is the page around them: WebUSB devices,
// Web Serial ports, files, and what the user sees. docs/12-web.md has the
// design.

import init, * as fp from '../pkg/fastprom_web.js';
import { UpdateLink, waitForDevice } from './usb.js';
import { FakeUsb } from './sim-usb.js';
import { SerialLink } from './serial.js';
import { FakeSerial } from './sim-serial.js';

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);
const SIM = params.has('sim');
const BUNDLE = params.get('bundle') ?? 'bundle/';
const POSITIONS = 4;

const st = {
  usb: null,
  serial: null,
  port: null,
  ids: null,
  sim: null,
  parts: {},
  device: null,
  link: null,
  info: null,
  table: null,
  tableSummary: null,
  bundleText: null,
  bundle: null,
  busy: false,
  wrote: false,
};

const hex = (v) => `0x${(v >>> 0).toString(16).padStart(8, '0')}`;

function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'text') e.textContent = v;
    else if (v !== false && v !== null && v !== undefined) e.setAttribute(k, v === true ? '' : v);
  }
  e.append(...kids);
  return e;
}

function log(msg) {
  const li = el('li', { text: msg });
  $('log').append(li);
  li.scrollIntoView({ block: 'nearest' });
}

function showError(msg) {
  const box = $('error');
  box.replaceChildren(el('p', { text: msg }));
  box.hidden = false;
}

function clearError() {
  $('error').hidden = true;
}

function progress(step, done, total) {
  $('progress-box').hidden = false;
  const pct = total ? Math.round((100 * done) / total) : 0;
  $('progress-label').textContent = `${step[0].toUpperCase()}${step.slice(1)}: ${pct}%`;
  $('progress').max = total || 1;
  $('progress').value = done;
}

/** The switch's two bits, numbered as printed on it: 1 is bit 0, 2 is bit 1; on is 1. */
function switchText(n) {
  return `switch 1 ${n & 1 ? 'on' : 'off'}, 2 ${n & 2 ? 'on' : 'off'}`;
}

// ---- running things ---------------------------------------------------------

async function run(fn) {
  if (st.busy) return;
  st.busy = true;
  clearError();
  render();
  try {
    await fn();
  } catch (e) {
    failed(e);
  } finally {
    st.busy = false;
    render();
  }
}

function failed(e) {
  const name = e?.name ?? '';
  let msg = e?.message ?? String(e);
  if (name === 'Gone' || name === 'NetworkError') {
    dropDevice();
    msg = 'The board went away: unplugged, or its power was lost. Power it, connect again, and run '
      + 'what you were doing again. An interrupted update leaves the board in its update mode or on its '
      + 'old firmware, never on half of the new; an interrupted image write leaves the socket off.';
  } else if (name === 'NotFoundError' && /No device selected/.test(msg)) {
    msg = 'No board was chosen.';
  }
  showError(msg);
  log(`Error: ${msg}`);
}

function dropDevice() {
  const link = st.link;
  st.link = null;
  st.info = null;
  st.table = null;
  st.tableSummary = null;
  st.wrote = false;
  link?.close();
}

// ---- the board -------------------------------------------------------------------

async function connect() {
  const { vid, pid } = st.ids;
  const known = (await st.usb.getDevices()).find((d) => d.vendorId === vid && d.productId === pid);
  const dev = known ?? (await st.usb.requestDevice({ filters: [{ vendorId: vid, productId: pid }] }));
  await attach(dev);
}

async function attach(dev) {
  if (st.link) await st.link.close();
  st.device = dev;
  st.link = await UpdateLink.open(dev);
  await refresh();
}

async function refresh() {
  const c = await fp.connect(st.link);
  st.info = JSON.parse(c.info);
  st.table = c.table ?? null;
  st.tableSummary = st.table ? JSON.parse(fp.tableImages(st.table)) : null;
  const i = st.info;
  log(`Connected: ${i.mode === 'dfu' ? 'update mode (DFU stage)' : 'firmware'} ${i.fw_version}, board revision ${i.board_rev}`);
  buildSlots();
}

/** After a reboot: the same board, enumerating as the other firmware. */
async function reattach(bcd, what) {
  const serial = st.device.serialNumber;
  await st.link?.close();
  st.link = null;
  log(`Waiting for the board to come back ${what}`);
  const dev = await waitForDevice(st.usb, serial, bcd);
  if (!dev) throw new Error(`The board did not come back ${what}. Unplug it, plug it in again and connect.`);
  await attach(dev);
}

/** parts.json's keys, for fastprom-web: what an image table's part may be. */
function knownParts() {
  return JSON.stringify(Object.keys(st.parts));
}

/**
 * The part the board reports: its fabric manifest's, or, with no manifest
 * (an interrupted fabric update erases it first), its image table's when
 * that is a known part. { part, source } or null.
 */
function boardPart() {
  return st.info ? JSON.parse(fp.boardPart(JSON.stringify(st.info), knownParts())) : null;
}

function part() {
  const p = boardPart()?.part ?? $('img-part')?.value ?? null;
  return p && st.parts[p] ? p : null;
}

/** The organisation ROM files are checked against. */
function geometry() {
  const p = part();
  if (p) return { part: p, depth: st.parts[p].depth, width: st.parts[p].width };
  return null;
}

// ---- rendering ----------------------------------------------------------------------

function render() {
  const i = st.info;
  const busy = st.busy;
  $('connect-btn').disabled = busy || !st.usb;
  $('connect-btn').textContent = i ? 'Reconnect' : 'Connect';
  $('connect-status').textContent = i
    ? `Connected: ${i.mode === 'dfu' ? 'update mode' : 'running its firmware'}.`
    : 'Not connected.';
  $('device').hidden = !i;
  if (i) renderDevice(i);
  renderImages(busy);
  renderUpdate(busy);
  renderRecovery(busy);
  if (!busy) $('progress-box').hidden = true;
}

function renderDevice(i) {
  $('d-mode').textContent = i.mode === 'dfu'
    ? 'Update mode (the DFU stage): the socket is off'
    : 'Its firmware, serving the socket';
  const app = i.app;
  $('d-fw').textContent = app
    ? `${app.version}${app.committed ? '' : ' (written, not committed: the board stays in update mode)'}`
      + (i.mode === 'dfu' ? `; update mode ${i.fw_version}` : '')
    : `none (update mode ${i.fw_version})`;
  $('d-rev').textContent = i.board_rev;
  const bp = boardPart();
  $('d-part').textContent = !bp
    ? (i.table ? `unknown (its image table says "${i.table.part}", which is not a part)` : 'unknown')
    : bp.source === 'table'
      ? `${bp.part} (from its image table: it has no fabric manifest)`
      : bp.part;
  $('d-fabric').textContent = i.fabric
    ? `${i.fabric.part}, interface ${i.fabric.fabric_id}, CRC ${hex(i.fabric.crc)}${i.fabric.committed ? '' : ' (not committed)'}`
    : 'no manifest';
  $('d-bank').textContent = i.bank === null ? 'not read' : `position ${i.bank} (${switchText(i.bank)})`;
  const rows = [];
  const images = st.tableSummary?.images ?? [];
  for (let n = 0; n < POSITIONS; n++) {
    const img = images[n];
    const what = img
      ? `${img.name}${img.intact ? '' : ' (damaged)'}${i.bank === n ? ' - selected by the switch' : ''}`
      : 'nothing: the socket stays off';
    rows.push(el('tr', {},
      el('th', { scope: 'row', text: `${n} (${switchText(n)})` }),
      el('td', { text: what }),
      el('td', { class: 'mono', text: img ? hex(img.crc) : '' })));
  }
  if (st.table === null && i.table === null) {
    rows.push(el('tr', {}, el('td', { colspan: 3, text: 'The board has no valid image table.' })));
  } else if (i.table?.intact && i.table.intact.includes(false)) {
    // The header is whole, an image under it is not: a write cut short.
    const bad = i.table.intact.map((ok, n) => (ok ? null : n)).filter((n) => n !== null);
    rows.push(el('tr', {}, el('td', {
      colspan: 3,
      class: 'bad',
      text: `The table's header is whole, but position${bad.length > 1 ? 's' : ''} ${bad.join(', ')} `
        + 'do not match their CRCs (a write cut short?): the board will not serve them. Write the images again.',
    })));
  }
  $('d-table').replaceChildren(...rows);
}

function buildSlots() {
  const g = geometry();
  const intro = $('images-intro');
  intro.replaceChildren();
  if (!st.info) {
    intro.textContent = 'Connect a board first.';
    $('slots').replaceChildren();
    return;
  }
  const bp = boardPart();
  if (bp?.source === 'table') {
    intro.append(`The board has no fabric manifest (an interrupted fabric update?): its image table says part ${bp.part}. `);
  }
  if (!bp) {
    const sel = el('select', { id: 'img-part' }, el('option', { value: '', text: 'choose...' }),
      ...Object.keys(st.parts).map((p) => el('option', { value: p, text: `${p} (${st.parts[p].chips.slice(0, 3).join(', ')}...)` })));
    sel.addEventListener('change', () => buildSlots());
    intro.append('The board does not report its part. ', el('label', {}, 'Part: ', sel));
    if (!g) {
      $('slots').replaceChildren();
      return;
    }
    sel.value = g.part;
  }
  if (g) {
    intro.append(`Part ${g.part}: ${g.depth} words of ${g.width} bits. A binary file (.bin, one byte per word) `
      + 'or a text file of hex words (.hex). A shorter file is padded with blank words (all ones, as an '
      + 'unprogrammed PROM reads).');
  }
  const current = st.tableSummary;
  const sameShape = current && g && current.depth === g.depth && current.width === g.width;
  const slots = [];
  for (let n = 0; n < POSITIONS; n++) {
    const kept = sameShape ? current.images[n] : null;
    const kind = el('select', { id: `slot-${n}-kind` },
      ...(kept && kept.intact ? [el('option', { value: 'keep', text: `Keep ${kept.name}` })] : []),
      el('option', { value: 'file', text: 'A ROM file' }),
      el('option', { value: 'empty', text: 'Nothing' }));
    if (!kept?.intact) kind.value = 'empty';
    const file = el('input', { type: 'file', id: `slot-${n}-file`, 'aria-label': `ROM file for position ${n}`, hidden: kind.value !== 'file' });
    const note = el('p', { class: 'note', id: `slot-${n}-note`, 'aria-live': 'polite' });
    kind.addEventListener('change', () => {
      file.hidden = kind.value !== 'file';
      note.textContent = '';
      note.classList.remove('bad');
      if (kind.value === 'file') file.focus();
    });
    file.addEventListener('change', () => checkFile(n));
    slots.push(el('div', { class: 'slot' },
      el('h3', { text: `Position ${n} (${switchText(n)})` }),
      el('div', { class: 'row' }, el('label', { for: `slot-${n}-kind`, text: 'Holds' }), kind, file),
      note));
  }
  $('slots').replaceChildren(...slots);
  st.wrote = false;
}

function renderImages(busy) {
  const ok = !!st.info && !!geometry();
  $('images-form').disabled = busy || !ok;
  const app = st.info?.mode === 'application';
  $('load-step').hidden = !(st.wrote && app);
  $('load-btn').disabled = busy || !$('host-off').checked;
}

function renderUpdate(busy) {
  const box = $('bundle-info');
  const form = $('update-form');
  form.disabled = true;
  if (!st.bundle) return;
  const b = st.bundle;
  const facts = [];
  if (b.firmware) facts.push(`firmware ${b.firmware.version}`);
  if (b.fabrics.length) facts.push(`fabric for ${b.fabrics.map((f) => f.part).join(', ')}`);
  const kids = [el('p', { text: `Published with this page, for board revision ${b.board_rev}: ${facts.join('; ') || 'nothing'}.` })];
  for (const n of b.notes) kids.push(el('p', { class: 'note', text: n }));
  if (!st.info) {
    kids.push(el('p', { text: 'Connect a board to see what it needs.' }));
    box.replaceChildren(...kids);
    return;
  }
  const bp = boardPart();
  const chosen = bp ? null : ($('u-part').value || null);
  const plan = JSON.parse(fp.planUpdate(st.bundleText, JSON.stringify(st.info), chosen, knownParts()));
  if (plan.refuse) {
    kids.push(el('div', { class: 'notice error', role: 'alert', id: 'refuse' }, el('p', { text: plan.refuse })));
    box.replaceChildren(...kids);
    return;
  }
  box.replaceChildren(...kids);
  const offer = (o, box, what) => {
    $(box).disabled = !o.available;
    if (!o.available) $(box).checked = false;
    else if (!st.planned) $(box).checked = !o.up_to_date;
    $(what).textContent = o.available
      ? (o.up_to_date ? `(up to date: ${o.new})` : `(${o.current ?? 'none'} → ${o.new})`)
      : `(${o.reason})`;
  };
  offer(plan.firmware, 'u-fw', 'u-fw-what');
  offer(plan.fabric, 'u-fabric', 'u-fabric-what');
  st.planned = true;
  const needPart = !bp && b.fabrics.length > 0;
  $('u-part-row').hidden = !needPart;
  if (needPart && $('u-part').options.length === 0) {
    $('u-part').replaceChildren(el('option', { value: '', text: 'choose...' }),
      ...b.fabrics.map((f) => el('option', { value: f.part, text: f.part })));
  }
  form.disabled = busy;
  $('update-btn').disabled = busy || !($('u-fw').checked || $('u-fabric').checked);
}

// ---- images ---------------------------------------------------------------------------

async function readFile(n) {
  const f = $(`slot-${n}-file`).files[0];
  if (!f) throw new Error(`Position ${n}: choose a ROM file, or Nothing.`);
  return { name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) };
}

function builder() {
  const g = geometry();
  return new fp.TableBuilder(g.part, g.depth, g.width);
}

async function checkFile(n) {
  const note = $(`slot-${n}-note`);
  note.classList.remove('bad');
  try {
    const { name, bytes } = await readFile(n);
    const s = JSON.parse(builder().setFile(n, name, bytes));
    note.textContent = `${s.given} words, CRC ${hex(s.crc)}`
      + (s.padded ? `; padded with ${s.padded} blank words` : '') + `; stored as "${s.name}".`;
  } catch (e) {
    note.textContent = e.message;
    note.classList.add('bad');
  }
}

async function writeImages() {
  const b = builder();
  for (let n = 0; n < POSITIONS; n++) {
    const kind = $(`slot-${n}-kind`).value;
    if (kind === 'keep') b.setKeep(n, st.table, n);
    else if (kind === 'file') {
      const { name, bytes } = await readFile(n);
      b.setFile(n, name, bytes);
    } else b.setEmpty(n);
  }
  const summary = JSON.parse(b.summary());
  const table = b.build();
  log(`Writing a table of ${summary.images.length}: ${summary.images.map((i) => i.name).join(', ')}`);
  await fp.writeTable(st.link, table, progress);
  log('Written and read back: the table in flash is the one built.');
  await refresh();
  st.wrote = true;
  $('host-off').checked = false;
  if (st.info.mode !== 'application') log('The board loads the new table when it next starts its firmware.');
}

async function loadImage() {
  await fp.load(st.link);
  st.wrote = false;
  await refresh();
  const n = st.info.bank;
  const img = st.tableSummary?.images[n];
  log(`Loaded: the socket serves position ${n}, ${img?.name ?? 'nothing'}.`);
}

// ---- firmware and fabric ----------------------------------------------------------------

async function fetchBytes(name) {
  const r = await fetch(new URL(name, new URL(BUNDLE, location.href)));
  if (!r.ok) throw new Error(`${name}: ${r.status} ${r.statusText}`);
  return new Uint8Array(await r.arrayBuffer());
}

async function updateBoard() {
  const b = st.bundle;
  const doFw = $('u-fw').checked;
  const doFabric = $('u-fabric').checked;
  const p = boardPart()?.part ?? $('u-part').value;
  // Every file fetched and checked before the board is touched.
  let fw = null;
  let fabric = null;
  if (doFw) {
    fw = await fetchBytes(b.firmware.file);
    log(`Firmware ${fp.checkFirmware(st.bundleText, fw)} fetched and checked.`);
  }
  if (doFabric) {
    const f = b.fabrics.find((x) => x.part === p);
    if (!f) throw new Error('Choose the part whose fabric to write.');
    fabric = { part: p, bits: await fetchBytes(f.file), page: await fetchBytes(f.manifest) };
  }
  if (st.info.mode === 'application') {
    log('Restarting the board into its update mode');
    await fp.enterDfu(st.link);
    await reattach(st.ids.bcdDfu, 'in its update mode');
  }
  if (st.info.mode !== 'dfu') throw new Error('The board did not enter its update mode.');
  if (fabric) {
    await fp.updateFabric(st.link, JSON.stringify(st.info), st.bundleText, fabric.part, fabric.bits, fabric.page, progress);
    log(`Fabric for ${fabric.part} written, checked and committed.`);
  }
  if (fw) {
    await fp.updateFirmware(st.link, JSON.stringify(st.info), st.bundleText, fw, progress);
    log('Firmware written, checked and committed.');
  }
  log('Restarting the board');
  await fp.rebootApplication(st.link);
  await reattach(st.ids.bcdApplication, 'running its firmware');
  st.planned = false;
  log(`Done: the board runs firmware ${st.info.fw_version}.`);
}

// ---- recovery ---------------------------------------------------------------------------

function renderRecovery(busy) {
  const form = $('recovery-form');
  form.disabled = true;
  $('r-go').disabled = true;
  if (!st.serial || !st.bundle) {
    if (!st.bundle) $('r-plan').replaceChildren(el('p', { text: 'No update is published with this page: nothing to recover a board with.' }));
    return;
  }
  const b = st.bundle;
  const sel = $('r-part');
  if (sel.options.length === 0) {
    sel.replaceChildren(
      ...b.fabrics.map((f) => el('option', { value: f.part, text: `${f.part} (${st.parts[f.part]?.chips.slice(0, 3).join(', ') ?? ''}...)` })),
      el('option', { value: '', text: 'none: the update mode only, no firmware or fabric' }),
    );
  }
  $('r-part-row').hidden = b.fabrics.length === 0;
  const plan = JSON.parse(fp.recoveryPlan(st.bundleText, sel.value || null));
  if (plan.refuse) {
    $('r-plan').replaceChildren(el('div', { class: 'notice error', role: 'alert', id: 'r-refuse' }, el('p', { text: plan.refuse })));
    return;
  }
  $('r-plan').replaceChildren(
    el('p', { text: `Written to the board's flash, after erasing all of it: ${plan.writes.join('; ')}.` }),
    el('p', { text: plan.outcome }),
  );
  $('r-rev-text').textContent = `The board is revision ${plan.board_rev}, the revision this update is for `
    + '(printed on the upper board)';
  form.disabled = busy;
  $('r-go').disabled = busy || !$('r-rev').checked || !st.port;
}

async function choosePort() {
  try {
    st.port = await st.serial.requestPort();
  } catch (e) {
    if (e?.name === 'NotFoundError') throw new Error('No serial port was chosen.');
    throw e;
  }
  const info = st.port.getInfo?.() ?? {};
  const id = info.usbVendorId === undefined ? ''
    : ` (USB ${info.usbVendorId.toString(16).padStart(4, '0')}:${info.usbProductId.toString(16).padStart(4, '0')})`;
  $('r-port-status').textContent = `Port chosen${id}.`;
  log(`Serial port chosen${id}.`);
}

async function recoverBoard() {
  const b = st.bundle;
  const r = b.recovery;
  const p = $('r-part').value || null;
  $('r-done').hidden = true;
  // Every file fetched (and checked, by the WebAssembly) before the port is used.
  const loader = await fetchBytes(r.loader.file);
  const dfu = await fetchBytes(r.dfu.file);
  const options = await fetchBytes(r.option_bytes.file);
  let app = new Uint8Array(0);
  let bits = new Uint8Array(0);
  let page = new Uint8Array(0);
  if (p) {
    const f = b.fabrics.find((x) => x.part === p);
    app = await fetchBytes(b.firmware.file);
    bits = await fetchBytes(f.file);
    page = await fetchBytes(f.manifest);
  }
  log(`Recovering the board: ${p ? `firmware ${b.firmware.version} and the fabric for ${p}` : 'the update mode only'}.`);
  let link;
  try {
    link = await SerialLink.open(st.port);
  } catch (e) {
    throw new Error(`The serial port could not be opened (${e.message}): is another program using it?`);
  }
  let report;
  try {
    report = JSON.parse(await fp.recover(link, st.bundleText, p, loader, dfu, options, app, bits, page, progress));
  } catch (e) {
    // The board's answers, already in words; never the USB board's "went away".
    throw new Error(e.message);
  } finally {
    await link.close();
  }
  log(`Found ${report.found}${report.unprotected ? '; read protection removed' : ''}${report.uploaded ? '; the recovery loader uploaded' : ''}.`);
  if (report.before.length) log(`The flash held ${report.before.join(' and ')}.`);
  if (report.retries) log(`${report.retries} command${report.retries > 1 ? 's' : ''} retried after a fault on the wire.`);
  log(`Recovered: ${report.wrote.join(', ')} written and read back.`);
  const done = $('r-done');
  done.replaceChildren(
    el('p', { text: `Recovered: ${report.wrote.join(', ')} written and read back.` }),
    el('p', { text: 'Now take the fixture off, power the board off and on, and connect it over USB (step 1).' }),
  );
  done.hidden = false;
}

// ---- the bundle --------------------------------------------------------------------------

async function loadBundle() {
  try {
    const r = await fetch(new URL('manifest.json', new URL(BUNDLE, location.href)));
    if (!r.ok) throw new Error(`${r.status}`);
    const text = await r.text();
    st.bundle = JSON.parse(fp.parseBundle(text));
    st.bundleText = text;
  } catch (e) {
    $('bundle-info').replaceChildren(el('p', { text: `No update is published with this page (${e.message}).` }));
  }
}

// ---- the simulated board ------------------------------------------------------------------

function startSim() {
  const p = params.get('part') ?? '512x8';
  const g = st.parts[p];
  if (!g) throw new Error(`no part ${p} in parts.json`);
  const [major, minor, patch] = (params.get('version') ?? '0.1.0').split('.').map(Number);
  const rev = params.get('rev') ?? 'A';
  if (params.has('blank')) {
    // A board never provisioned: nothing runs until recovery.
    st.sim = fp.Sim.blank(rev, p, g.depth, g.width);
    st.sim.powerOn();
  } else {
    st.sim = new fp.Sim(rev, p, g.depth, g.width, major, minor, patch);
  }
  if (params.has('device')) st.sim.setDeviceId(Number(params.get('device')));
  const usb = new FakeUsb(fp, st.sim);
  st.serial = new FakeSerial(st.sim, () => usb.tick());
  usb.onchange = (s) => {
    const parts = [s.powered ? 'Powered' : 'Unpowered'];
    if (s.boot0) parts.push('BOOT0 held high');
    if (s.uart) parts.push(`the ${s.uart} on its UART`);
    else if (s.powered && !s.connected) parts.push('not on USB');
    if (s.mode && s.connected) parts.push(s.mode === 'dfu' ? 'in update mode' : 'running its firmware');
    if (s.mode === 'application') parts.push(s.served === null ? 'the socket is off' : `the socket serves ${hex(s.served)}`);
    parts.push(`${s.flash_ops} flash operations so far`);
    if (s.cut_armed !== null) parts.push(`power cut armed: ${s.cut_armed} to go`);
    $('sim-status').textContent = `${parts.join(', ')}.`;
  };
  usb.tick();
  $('sim-banner').hidden = false;
  $('sim-panel').hidden = false;
  $('sim-switch').addEventListener('change', (e) => {
    st.sim.setSwitch(Number(e.target.value));
    log(`Simulator: switch at position ${e.target.value}, read at the next power-up.`);
  });
  $('sim-arm').addEventListener('click', () => {
    const n = Math.max(1, Number($('sim-ops').value) || 1);
    st.sim.cutPowerAfter(n);
    usb.tick();
    log(`Simulator: the power goes at the ${n}th flash operation from now.`);
  });
  $('sim-off').addEventListener('click', () => {
    usb.powerOff();
    log('Simulator: power off.');
  });
  $('sim-on').addEventListener('click', () => {
    usb.powerOn();
    log('Simulator: power on.');
  });
  $('sim-boot0').addEventListener('change', (e) => {
    st.sim.setBoot0(e.target.checked);
    usb.tick();
    log(`Simulator: BOOT0 ${e.target.checked ? 'held at 3.3 V' : 'released'}, read at the next power-on.`);
  });
  $('sim-fault').addEventListener('click', () => {
    const kind = $('sim-fault-kind').value;
    const n = Math.max(1, Number($('sim-fault-after').value) || 1);
    st.sim.injectFault(kind, n);
    log(`Simulator: ${kind === 'drop' ? 'a byte from the board lost' : 'a byte to the board corrupted'} ${n} bytes from now.`);
  });
  return usb;
}

// ---- start ------------------------------------------------------------------------------

async function main() {
  await init();
  st.ids = JSON.parse(fp.usbIds());
  st.parts = await (await fetch('parts.json')).json();
  if (SIM) {
    st.usb = startSim();
  } else {
    if ('usb' in navigator) st.usb = navigator.usb;
    else $('unsupported').hidden = false;
    if ('serial' in navigator) st.serial = navigator.serial;
  }
  if (!st.serial) $('serial-unsupported').hidden = false;
  st.usb?.addEventListener('disconnect', (e) => {
    if (!st.busy && e.device === st.device && st.link) {
      dropDevice();
      log('The board was disconnected.');
      render();
    }
  });
  $('connect-btn').addEventListener('click', () => run(connect));
  $('write-btn').addEventListener('click', () => run(writeImages));
  $('host-off').addEventListener('change', () => render());
  $('load-btn').addEventListener('click', () => run(loadImage));
  $('update-btn').addEventListener('click', () => run(updateBoard));
  for (const id of ['u-fw', 'u-fabric', 'u-part', 'r-part', 'r-rev']) $(id).addEventListener('change', () => render());
  $('r-port').addEventListener('click', () => run(choosePort));
  $('r-go').addEventListener('click', () => run(recoverBoard));
  await loadBundle();
  render();
  document.body.dataset.ready = 'true';
}

main().catch((e) => showError(`The page could not start: ${e.message}`));
