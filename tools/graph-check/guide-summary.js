// Synthetic data exercises the guide's claims rather than its layout.
// NODE_PATH=... node tools/graph-check/guide-summary.js /path/to/chromium
const fs = require('fs');
const path = require('path');
const assert = require('assert');
const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ executablePath: process.argv[2], headless: true, args: ['--no-sandbox'] });
  try {
    const template = fs.readFileSync(path.join(__dirname, '../../src/guide.html'), 'utf8');
    const labels = ['64 B', '256 B', '1 KiB', '4 KiB'];
    const units = [64, 256, 1024, 4096];
    const series = mean => ({ mean,
      lat: mean.map((v, i) => v * units[i]), kernels: [] });
    const data = med => ({ machine: 'synthetic test', date: '2026-10-01', contenders: [
      { key: 'blake3-servil-st', name: 'BLAKE3 servil st', color: '#7c3aed' },
      { key: 'sha256-ring', name: 'SHA-256 ring', color: '#c2410c' }], plots: [{
        scenario: 'solo', use: 'LentMessages', batch: false, labels, units, bytes: units,
        series: { 'blake3-servil-st': series(med), 'sha256-ring': series([1, 1, 1, 1]) }
      }] });
    async function summary(med) {
      const page = await browser.newPage();
      try {
        await page.setContent(template.replace('@DATA@', JSON.stringify(data(med))).replace('@EXAMPLES@', '{"hash":"test example"}'));
        await page.evaluate(() => {
          answers = { threads: 'one', shape: 'message', keepsUp: 'no' };
          show();
        });
        return await page.locator('#speed-summary').textContent();
      } finally { await page.close(); }
    }
    const middleWin = await summary([2, 0.5, 0.5, 2]);
    assert(!middleWin.includes('Slower than SHA-256 ring at every size'), middleWin);
    assert(middleWin.toLowerCase().includes('trade places'), middleWin);
    const ties = await summary([1, 1, 1, 1]);
    assert(!ties.startsWith('Slower than'), ties);
    const lose = await summary([2, 2, 2, 2]);
    assert(lose.startsWith('Slower than'), lose);
    const win = await summary([0.5, 0.5, 0.5, 0.5]);
    assert(win.startsWith('Faster than'), win);
    console.log('guide-summary: middle-only wins, ties, all losses, all wins pass');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
