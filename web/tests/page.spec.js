// The update page against the simulated board: fastprom-core's update core
// compiled to WebAssembly behind the page's fake WebUSB (js/sim-usb.js).
// Every test fails on any console error or warning, or uncaught exception.
import { test as base, expect } from '@playwright/test';

const test = base.extend({
  page: async ({ page }, use) => {
    const problems = [];
    page.on('console', (m) => {
      if (m.type() === 'error' || m.type() === 'warning') problems.push(`${m.type()}: ${m.text()}`);
    });
    page.on('pageerror', (e) => problems.push(`exception: ${e.message}`));
    await use(page);
    expect(problems, 'console errors and warnings').toEqual([]);
  },
});

async function open(page, query = '?sim&bundle=test-bundle-A/') {
  await page.goto(`index.html${query}`);
  await expect(page.locator('body')).toHaveAttribute('data-ready', 'true');
}

async function connect(page) {
  await page.getByRole('button', { name: /^(Connect|Reconnect)$/ }).click();
  await expect(page.locator('#connect-status')).toHaveText(/^Connected/);
}

const row = (page, n) => page.locator('#d-table tr').nth(n);

test('connect shows the board', async ({ page }) => {
  await open(page);
  await expect(page.locator('#sim-banner')).toBeVisible();
  await connect(page);
  await expect(page.locator('#d-mode')).toContainText('serving the socket');
  await expect(page.locator('#d-fw')).toHaveText('0.1.0');
  await expect(page.locator('#d-rev')).toHaveText('A');
  await expect(page.locator('#d-part')).toHaveText('512x8');
  await expect(page.locator('#d-fabric')).toContainText('512x8, interface 1');
  await expect(page.locator('#d-bank')).toHaveText('position 0 (switch 1 off, 2 off)');
  for (let n = 0; n < 4; n++) await expect(row(page, n)).toContainText(`factory-${n}.bin`);
  await expect(row(page, 0)).toContainText('selected by the switch');
  await expect(page.locator('#u-fw-what')).toHaveText('(0.1.0 → 0.2.0)');
  // Recovery is offered, with the J3 wiring.
  await expect(page.locator('#j3')).toContainText('the adapter\'s RX');
  await expect(page.locator('#r-plan')).toContainText('the fabric for part 512x8; the firmware 0.2.0');
  await expect(page.locator('#serial-unsupported')).toBeHidden();
});

test('finds the update interface in both firmwares\' descriptors', async ({ page }) => {
  await open(page);
  // The page's own rule (js/usb.js), on the configuration descriptors the
  // two firmwares send (fastprom-usb's, through the WebAssembly).
  const found = await page.evaluate(async () => {
    const fp = await import('./pkg/fastprom_web.js');
    const { findUpdateInterface } = await import('./js/usb.js');
    const { parseConfiguration } = await import('./js/sim-usb.js');
    const config = (dfu) => parseConfiguration(fp.firmwareConfiguration(dfu));
    return {
      app: findUpdateInterface(config(false)),
      dfu: findUpdateInterface(config(true)),
      interfaces: [config(false).interfaces.length, config(true).interfaces.length],
      ids: JSON.parse(fp.usbIds()),
    };
  });
  const want = { interfaceNumber: 0, epOut: 1, epIn: 1 };
  expect(found.app).toEqual(want);
  expect(found.dfu).toEqual(want);
  expect(found.interfaces).toEqual([3, 1]);
  expect(found.ids.updateInterface).toBe(0);
});

// ---- UART recovery -------------------------------------------------------------------

/** Hold BOOT0 high and power-cycle: the mask ROM's loader answers on J3. */
async function intoBootLoader(page) {
  await page.getByLabel('BOOT0 held at 3.3 V by the fixture').check();
  await page.getByRole('button', { name: 'Power off' }).click();
  await page.getByRole('button', { name: 'Power on' }).click();
  await expect(page.locator('#sim-status')).toContainText('the mask ROM on its UART');
}

async function startRecovery(page) {
  await page.locator('#r-part').selectOption('512x8');
  await page.getByLabel(/The board is revision A/).check();
  await page.getByRole('button', { name: 'Choose the serial port' }).click();
  await expect(page.locator('#r-port-status')).toContainText('Port chosen');
  await page.getByRole('button', { name: 'Recover the board' }).click();
}

test('recovery provisions a blank board, which then boots on USB', async ({ page }) => {
  await open(page, '?sim&blank&bundle=test-bundle-A/');
  await expect(page.locator('#sim-status')).toContainText('not on USB');
  await intoBootLoader(page);
  await startRecovery(page);
  await expect(page.locator('#r-done')).toContainText('Recovered: option bytes, the DFU stage, the fabric, the fabric manifest, the firmware written and read back.', { timeout: 45000 });
  await expect(page.locator('#log')).toContainText('Found the mask ROM; read protection removed; the recovery loader uploaded.');
  await expect(page.locator('#r-done')).toContainText('take the fixture off');
  // As the page says: fixture off, power-cycle, connect.
  await page.getByLabel('BOOT0 held at 3.3 V by the fixture').uncheck();
  await page.getByRole('button', { name: 'Power off' }).click();
  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  await expect(page.locator('#d-mode')).toContainText('serving the socket');
  await expect(page.locator('#d-fw')).toHaveText('0.2.0');
  await expect(page.locator('#d-part')).toHaveText('512x8');
  await expect(page.locator('#d-table')).toContainText('The board has no valid image table.');
  await expect(page.locator('#u-fw-what')).toHaveText('(up to date: 0.2.0)');
});

test('recovery retries through faults on the wire', async ({ page }) => {
  await open(page, '?sim&blank&bundle=test-bundle-A/');
  await intoBootLoader(page);
  // A byte lost from the board during the upload through the mask ROM...
  await page.locator('#sim-fault-kind').selectOption('drop');
  await page.locator('#sim-fault-after').fill('400');
  await page.getByRole('button', { name: 'Inject' }).click();
  await startRecovery(page);
  await expect(page.locator('#r-done')).toContainText('Recovered', { timeout: 45000 });
  await expect(page.locator('#log')).toContainText(/1 command retried|commands retried/);
  // ...and, run again on the recovered board, one corrupted on its way in
  // during the recovery loader's writes.
  await page.getByRole('button', { name: 'Power off' }).click();
  await page.getByRole('button', { name: 'Power on' }).click();
  await page.locator('#sim-fault-kind').selectOption('corrupt');
  await page.locator('#sim-fault-after').fill('20000');
  await page.getByRole('button', { name: 'Inject' }).click();
  await page.getByRole('button', { name: 'Recover the board' }).click();
  await expect(page.locator('#log')).toContainText('The flash held firmware 0.2.0 for board A', { timeout: 45000 });
  await expect(page.locator('#log').locator('li').last()).toContainText('Recovered');
  await page.getByLabel('BOOT0 held at 3.3 V by the fixture').uncheck();
  await page.getByRole('button', { name: 'Power off' }).click();
  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  await expect(page.locator('#d-fw')).toHaveText('0.2.0');
});

test('recovery refuses a chip that is not the AG32VF303', async ({ page }) => {
  await open(page, '?sim&blank&device=0x12340001&bundle=test-bundle-A/');
  await intoBootLoader(page);
  await startRecovery(page);
  await expect(page.locator('#error')).toContainText('device 0x12340001, not the AG32VF303 (0x40200001)', { timeout: 20000 });
  await expect(page.locator('#error')).toContainText('Nothing was written.');
  await expect(page.locator('#r-done')).toBeHidden();
  await expect(page.locator('#sim-status')).toContainText('0 flash operations');
});

test('recovery needs the board revision confirmed', async ({ page }) => {
  await open(page, '?sim&blank&bundle=test-bundle-B/');
  await expect(page.locator('#r-rev-text')).toContainText('revision B');
  await page.getByRole('button', { name: 'Choose the serial port' }).click();
  await expect(page.getByRole('button', { name: 'Recover the board' })).toBeDisabled();
  await page.getByLabel(/The board is revision B/).check();
  await expect(page.getByRole('button', { name: 'Recover the board' })).toBeEnabled();
});

test('image update: write, verify, then load with the host off', async ({ page }) => {
  await open(page);
  await connect(page);
  // A file that does not fit the part is refused on the spot.
  await page.locator('#slot-3-kind').selectOption('file');
  await page.locator('#slot-3-file').setInputFiles({ name: 'big.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(600) });
  await expect(page.locator('#slot-3-note')).toContainText('600 words: more than the 512');

  const rom = Buffer.alloc(300, 0x5a);
  await page.locator('#slot-0-kind').selectOption('file');
  await page.locator('#slot-0-file').setInputFiles({ name: 'game.bin', mimeType: 'application/octet-stream', buffer: rom });
  await expect(page.locator('#slot-0-note')).toContainText('300 words');
  await expect(page.locator('#slot-0-note')).toContainText('padded with 212 blank words');
  // Position 1 keeps its image; 2 and 3 hold nothing: a table of two.
  await page.locator('#slot-2-kind').selectOption('empty');
  await page.locator('#slot-3-kind').selectOption('empty');
  await page.getByRole('button', { name: 'Write and verify' }).click();
  await expect(page.locator('#log')).toContainText('Written and read back');

  const load = page.getByRole('button', { name: 'Load the new image now' });
  await expect(page.locator('#load-step')).toContainText('Power off the host machine before loading');
  await expect(load).toBeDisabled();
  await page.getByLabel('The host is powered off').check();
  await load.click();
  await expect(page.locator('#log')).toContainText('Loaded: the socket serves position 0, game.bin.');
  await expect(row(page, 0)).toContainText('game.bin');
  await expect(row(page, 1)).toContainText('factory-1.bin');
  await expect(row(page, 2)).toContainText('nothing');
  const crc = (await row(page, 0).locator('td.mono').textContent()).trim();
  await expect(page.locator('#sim-status')).toContainText(`the socket serves ${crc}`);
});

test('firmware and fabric update through DFU mode', async ({ page }) => {
  await open(page);
  await connect(page);
  await expect(page.locator('#u-fw')).toBeChecked();
  await expect(page.locator('#u-fabric')).toBeChecked();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#log')).toContainText('Waiting for the board to come back in its update mode');
  await expect(page.locator('#log')).toContainText('Done: the board runs firmware 0.2.0.', { timeout: 45000 });
  await expect(page.locator('#log')).toContainText('Fabric for 512x8 written, checked and committed.');
  await expect(page.locator('#d-fw')).toHaveText('0.2.0');
  await expect(page.locator('#d-mode')).toContainText('serving the socket');
  await expect(page.locator('#u-fw-what')).toHaveText('(up to date: 0.2.0)');
  await expect(page.locator('#sim-status')).toContainText('the socket serves 0x');
});

test('a power cut mid-update, then recovery', async ({ page }) => {
  await open(page);
  await connect(page);
  await page.locator('#u-fabric').uncheck();
  // In the firmware's erase and write: well after the reboot into DFU.
  await page.locator('#sim-ops').fill('400');
  await page.getByRole('button', { name: 'Arm' }).click();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#error')).toContainText('The board went away', { timeout: 45000 });
  await expect(page.locator('#connect-status')).toHaveText('Not connected.');
  await expect(page.locator('#sim-status')).toContainText('Unpowered');

  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  // The old firmware's commit word went first: the board is in DFU mode.
  await expect(page.locator('#d-mode')).toContainText('Update mode');
  await expect(row(page, 0)).toContainText('factory-0.bin');
  await page.locator('#u-fabric').uncheck();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#log')).toContainText('Done: the board runs firmware 0.2.0.', { timeout: 45000 });
  await expect(page.locator('#d-fw')).toHaveText('0.2.0');
});

test('an update for another board revision is refused', async ({ page }) => {
  await open(page, '?sim&bundle=test-bundle-B/');
  await connect(page);
  await expect(page.locator('#refuse')).toContainText('this update is for board revision B, and the board is revision A');
  await expect(page.locator('#update-form')).toHaveAttribute('disabled', '');
  await expect(page.getByRole('button', { name: 'Update' })).toBeDisabled();
});

test('a browser without WebUSB or Web Serial is told so', async ({ page }) => {
  await page.addInitScript(() => {
    delete Navigator.prototype.usb;
    delete Navigator.prototype.serial;
  });
  await open(page, '');
  await expect(page.locator('#unsupported')).toBeVisible();
  await expect(page.locator('#unsupported')).toContainText('needs WebUSB');
  await expect(page.getByRole('button', { name: 'Connect' })).toBeDisabled();
  await expect(page.locator('#serial-unsupported')).toContainText('needs Web Serial');
  await expect(page.locator('#recovery-form')).toHaveAttribute('disabled', '');
});

test('the image table part stands in for a missing fabric manifest', async ({ page }) => {
  await open(page);
  await connect(page);
  // A fabric update cut just after it erased the manifest.
  await page.locator('#u-fw').uncheck();
  await page.locator('#sim-ops').fill('2');
  await page.getByRole('button', { name: 'Arm' }).click();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#error')).toContainText('The board went away', { timeout: 45000 });
  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  // No committed manifest: the DFU stage stays in update mode.
  await expect(page.locator('#d-mode')).toContainText('Update mode');
  await expect(page.locator('#d-fabric')).toHaveText('no manifest');
  await expect(page.locator('#d-part')).toHaveText('512x8 (from its image table: it has no fabric manifest)');
  // Not asked: the images and the fabric go by the table's part.
  await expect(page.locator('#img-part')).toHaveCount(0);
  await expect(page.locator('#u-part-row')).toBeHidden();
  await expect(page.locator('#u-fabric')).toBeChecked();
  await page.locator('#u-fw').uncheck();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#log')).toContainText('Fabric for 512x8 written, checked and committed.', { timeout: 45000 });
  await expect(page.locator('#d-part')).toHaveText('512x8');
});

test('a power cut in the fabric update leaves the board in update mode, on USB', async ({ page }) => {
  await open(page);
  await connect(page);
  await page.locator('#u-fw').uncheck();
  // Into the bitstream's writes, the manifest long erased.
  await page.locator('#sim-ops').fill('300');
  await page.getByRole('button', { name: 'Arm' }).click();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#error')).toContainText('The board went away', { timeout: 45000 });
  await page.getByRole('button', { name: 'Power on' }).click();
  await expect(page.locator('#sim-status')).toContainText('in update mode');
  await connect(page);
  await expect(page.locator('#d-mode')).toContainText('Update mode');
  await expect(page.locator('#d-fw')).toContainText('0.1.0');
  await page.locator('#u-fw').uncheck();
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#log')).toContainText('Done: the board runs firmware 0.1.0.', { timeout: 45000 });
  await expect(page.locator('#d-mode')).toContainText('serving the socket');
  await expect(page.locator('#d-fabric')).toContainText('512x8, interface 1');
});

test('a table write cut short is reported from INFO', async ({ page }) => {
  await open(page);
  await connect(page);
  await page.locator('#slot-0-kind').selectOption('file');
  await page.locator('#slot-0-file').setInputFiles({ name: 'game.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(512, 0x11) });
  await expect(page.locator('#slot-0-note')).toContainText('512 words');
  // Past the erase and the header page, into the images.
  await page.locator('#sim-ops').fill('120');
  await page.getByRole('button', { name: 'Arm' }).click();
  await page.getByRole('button', { name: 'Write and verify' }).click();
  await expect(page.locator('#error')).toContainText('The board went away', { timeout: 45000 });
  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  await expect(page.locator('#d-table')).toContainText('do not match their CRCs');
  await expect(page.locator('#sim-status')).toContainText('the socket is off');
});

// The bundle `make web` published, from the real firmware build: its
// application (and fabrics, where build/ag32 had them) pass the page's
// checks and update a simulated board that has an older version.
test('the published bundle updates a board', async ({ page, request }) => {
  const manifest = await (await request.get('bundle/manifest.json')).json();
  test.skip(!manifest.firmware, 'make web was run without make fw-build: no firmware in the bundle');
  await open(page, `?sim&version=0.0.1&rev=${manifest.board_rev}&bundle=bundle/`);
  await connect(page);
  await expect(page.locator('#u-fw-what')).toHaveText(`(0.0.1 → ${manifest.firmware.version})`);
  await expect(page.locator('#u-fabric')).toBeChecked({ checked: manifest.fabrics.some((f) => f.part === '512x8') });
  await page.getByRole('button', { name: 'Update' }).click();
  await expect(page.locator('#log')).toContainText(`Done: the board runs firmware ${manifest.firmware.version}.`, { timeout: 45000 });
});

// The bundle's recovery set from the real firmware build: the stamped
// recovery loader and the DFU stage pass the page's checks, upload and
// write; with no fabric in the bundle, the board comes up in update mode.
test('the published bundle recovers a blank board', async ({ page, request }) => {
  const manifest = await (await request.get('bundle/manifest.json')).json();
  test.skip(!manifest.recovery, 'make web was run without make fw-build: no recovery set in the bundle');
  await open(page, `?sim&blank&rev=${manifest.board_rev}&bundle=bundle/`);
  await intoBootLoader(page);
  if (manifest.fabrics.some((f) => f.part === '512x8')) await page.locator('#r-part').selectOption('512x8');
  else await expect(page.locator('#r-part-row')).toBeHidden();
  await page.getByLabel(new RegExp(`The board is revision ${manifest.board_rev}`)).check();
  await page.getByRole('button', { name: 'Choose the serial port' }).click();
  await page.getByRole('button', { name: 'Recover the board' }).click();
  await expect(page.locator('#r-done')).toContainText('Recovered: option bytes, the DFU stage', { timeout: 45000 });
  await page.getByLabel('BOOT0 held at 3.3 V by the fixture').uncheck();
  await page.getByRole('button', { name: 'Power off' }).click();
  await page.getByRole('button', { name: 'Power on' }).click();
  await connect(page);
  if (manifest.fabrics.some((f) => f.part === '512x8')) {
    await expect(page.locator('#d-fw')).toHaveText(manifest.firmware.version);
  } else {
    await expect(page.locator('#d-mode')).toContainText('Update mode');
  }
});

test('fits a phone width', async ({ page }) => {
  await open(page);
  await connect(page);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  await expect(page.getByRole('button', { name: 'Write and verify' })).toBeVisible();
});
