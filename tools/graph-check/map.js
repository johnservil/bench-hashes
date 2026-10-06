// Drives a generated bench-hashes.map.html in Chromium: no script error;
// every chart opens in place and closes; its row and column headers light
// while it is open; a header greys its charts; every call name links to its
// documentation; a link to one chart (#solo|one|lent) opens it; on a touch
// screen a tap shows the values at its point, the chart open, and the
// button puts it back.
// npm install playwright; node map.js MAP.html [CHROMIUM]
const path = require('path');
const assert = require('assert');
const { pathToFileURL } = require('url');
const { chromium } = require('playwright');
(async () => {
  const [file, exe] = process.argv.slice(2);
  const url = pathToFileURL(path.resolve(file)).href;
  const browser = await chromium.launch(exe ? { executablePath: exe, args: ['--no-sandbox'] } : {});
  const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(url);
  const charts = await page.locator('.cell:not(.empty)').count();
  assert(charts >= 1, 'some chart');
  const docs = await page.$$eval('a.doc', as => as.map(a => a.href));
  assert.equal(docs.length, charts, 'every chart names its call, a link');
  for (const d of docs) assert(/^https:\/\//.test(d), d);
  for (let i = 0; i < charts; i++) {
    const cell = page.locator('.cell:not(.empty)').nth(i);
    await cell.scrollIntoViewIfNeeded();
    await cell.click({ position: { x: 100, y: 30 } });
    await page.waitForTimeout(260);
    assert.equal(await page.locator('.zoom').count(), 1, `chart ${i} opens`);
    assert.equal(await page.locator('.h.lit').count(), 2, `chart ${i}: its row and column light`);
    const box = await page.locator('.zoom').boundingBox();
    // Inside the opened chart and the window both, clear of its title.
    await page.mouse.move(box.x + box.width / 2, Math.min(box.y + box.height / 2, Math.max(box.y + 60, 850)));
    assert(await page.locator('#tip').isVisible(), `chart ${i}: the values under the pointer`);
    assert(/\d/.test(await page.locator('#tip').textContent()), `chart ${i}: numbers in the tip`);
    await page.locator('.zoom').click({ position: { x: box.width / 2, y: 60 } });
    await page.waitForTimeout(300);
    assert.equal(await page.locator('.zoom').count(), 0, `chart ${i} closes`);
    assert.equal(await page.locator('.h.lit').count(), 0, `chart ${i}: headers unlit`);
  }
  await page.locator('.h').first().click();
  assert(await page.locator('.cell.dim').count() >= 1, 'a header greys its charts');
  const first = await page.locator('.cell:not(.empty)').first().getAttribute('data-key');
  await page.goto(url + '#' + encodeURIComponent(first.split(':').slice(1).join(':')));
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.zoom').count(), 1, 'a link to one chart opens it');
  // A touch screen: tap a chart open, tap it for its values, tap the button.
  const phone = await browser.newPage({ viewport: { width: 420, height: 860 }, hasTouch: true, isMobile: true });
  phone.on('pageerror', e => errors.push(e.message));
  await phone.goto(url);
  await phone.locator('.cell:not(.empty)').first().tap();
  await phone.waitForTimeout(300);
  const zoom = await phone.locator('.zoom').boundingBox();
  await phone.touchscreen.tap(zoom.x + zoom.width * 0.6, zoom.y + zoom.height * 0.5);
  await phone.waitForTimeout(150);
  assert.equal(await phone.locator('.zoom').count(), 1, 'a tap on an open chart keeps it open');
  assert(await phone.locator('#tip').isVisible(), 'a tap shows the values at its point');
  await phone.locator('.zoom .close').tap();
  await phone.waitForTimeout(300);
  assert.equal(await phone.locator('.zoom').count(), 0, 'the button puts the chart back');
  assert.deepEqual(errors, []);
  console.log(`${charts} charts opened and closed, ${docs.length} documentation links, headers, a chart's link, touch: pass`);
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });
