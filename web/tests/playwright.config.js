// make web-test: the page from build/web-test/site, in Chromium, in
// Playwright's own container image (Makefile PLAYWRIGHT_IMAGE).
import { defineConfig, devices } from '@playwright/test';

const PORT = 8765;

export default defineConfig({
  testDir: '.',
  outputDir: '../../build/web-test/results',
  timeout: 60000,
  workers: 2,
  reporter: [['list']],
  use: {
    baseURL: `http://127.0.0.1:${PORT}/`,
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'] }, grepInvert: /phone width/ },
    { name: 'phone', use: { ...devices['Pixel 7'] }, grep: /phone width/ },
  ],
  webServer: {
    command: `node serve.mjs ../../build/web-test/site ${PORT}`,
    url: `http://127.0.0.1:${PORT}/index.html`,
    reuseExistingServer: false,
  },
});
