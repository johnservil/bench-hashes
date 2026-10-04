// Exercise the generated guide in a real browser: every decision-table
// route, every uncertain default, Back and restart, and each ending's
// example, map link, and table. npm install playwright; node guide.js GUIDE.html [CHROMIUM]
const { chromium } = require('playwright');
const { pathToFileURL } = require('url');
const assert = require('assert');
const path = require('path');
(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.argv[3] || undefined, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 900, height: 1000 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.goto(pathToFileURL(path.resolve(process.argv[2])).href);
  // The table, written independently of the page's function.
  const after = { message: ['hash', 'hash_multithreaded', 'OneMessage'], pieces: ['update', 'update', 'OneMessage'], batch: ['hash_many', 'hash_many_multithreaded', 'ManyMessages'] };  // after other work, the default for a program that keeps up
  const continuous = {
    message: ['hash', 'hash_multithreaded', 'Queue::messages', 'LentMessages', 'ContinuousMessages'],
    pieces: ['update', 'update_multithreaded', 'Queue::pieces', 'LentMessages', 'ContinuousMessages'],
    batch: ['hash_many', 'hash_many_multithreaded', 'Queue::fixed', 'LentBatches', 'ContinuousBatches'],
  };
  let routes = 0;
  for (const threads of ['one', 'many']) for (const shape of ['message', 'pieces', 'batch']) for (const keepsUp of ['yes', 'no']) for (const buffer of ['owned', 'lent']) {
    const a = { threads, shape, keepsUp, buffer };
    const r = await page.evaluate(a => recommendation(a), a);
    const call = keepsUp === 'yes' ? after[shape][threads === 'many' ? 1 : 0] : continuous[shape][threads === 'one' ? 0 : buffer === 'owned' ? 2 : 1];
    const use = keepsUp === 'yes' ? after[shape][2] : continuous[shape][threads === 'many' && buffer === 'owned' ? 4 : 3];
    assert.equal(r.call, call, JSON.stringify(a));
    assert.equal(r.uses[r.useIndex], use, JSON.stringify(a));
    routes++;
  }
  // Click every reachable ending; check what the reader sees.
  let endings = 0, measured = 0;
  for (const threads of [0, 1]) for (const shape of [0, 1, 2]) for (const keepsUp of [0, 1]) for (const buffer of threads === 0 && keepsUp === 1 ? [0, 1, 2, 3] : [null]) {
    if (await page.locator('#restart').isVisible()) await page.locator('#restart').click();
    for (const i of [threads, shape, keepsUp]) await page.locator('#choices button').nth(i).click();
    if (buffer !== null) await page.locator('#choices button').nth(buffer).click();
    const state = await page.evaluate(() => ({
      call: current.call, example: document.getElementById('example').textContent, unmeasured: !document.getElementById('unmeasured').hidden,
      map: (document.querySelector('#chart a') || {}).getAttribute?.('href') || '',
      chips: [...document.querySelectorAll('#how button')].map(b => [b.textContent, b.getAttribute('aria-pressed')]),
      summary: document.getElementById('speed-summary').textContent, resultVisible: !document.getElementById('result').hidden,
      standIn: !document.getElementById('stand-in').hidden,
      uses: current.uses,
    }));
    assert(state.resultVisible);
    assert(state.example.includes('fn main'), `${state.call}: a complete program`);
    assert(state.example.includes(state.call.replace('Queue::', '')), `${state.call}: the example calls it`);
    if (state.unmeasured) { assert(['Queue::pieces', 'update', 'update_multithreaded'].includes(state.call), `${state.call}: only cells past 1 MiB may be absent from a quick run`); }
    else {
      measured++;
      /* The door to the measurement: its chart on the map. */
      assert(/^bench-hashes\.map\.html#(solo|shared)%7C[a-z]+%7C[a-z]+$/.test(state.map), `${state.call}: a link to its chart on the map: ${state.map}`);
      assert(/Faster|Slower|As fast as|trade places|measured alone/.test(state.summary), state.summary);
      if (state.call.startsWith('Queue::')) assert(!state.standIn, 'queue measurements are actual queue calls');
      if (shape === 1 && threads === 0 && !state.call.startsWith('Queue::')) {
        assert.deepEqual(state.uses, state.call === 'update_multithreaded' ? [null, null, 'LentMessages'] : ['IdleOneMessage', 'OneMessage', null],
          'piece patterns keep the measured API');
      }
      /* The chips: the pattern (three for a plain function), alone or beside another program. */
      const pressed = state.chips.filter(([, p]) => p === 'true').map(([l]) => l);
      const patterns = ['after idling', 'after other work', 'nonstop'];
      const alone = pressed.includes('alone') || !state.chips.some(([l]) => l === 'beside another program');
      assert(alone, JSON.stringify(state.chips));
      if (!state.call.startsWith('Queue::')) assert.equal(pressed.filter(l => patterns.includes(l)).length, 1, JSON.stringify(state.chips));
      if (state.chips.some(([l]) => l === 'beside another program')) {
        await page.locator('#how button', { hasText: 'beside another program' }).click();
        assert.equal(await page.evaluate(() => [...document.querySelectorAll('#how button')].find(b => b.textContent === 'beside another program').getAttribute('aria-pressed')), 'true');
        await page.locator('#how button', { hasText: 'alone' }).click();
      }
      if (!state.call.startsWith('Queue::')) {
        // Some calls have only one measured
        // pattern. Drive only the buttons actually offered to the reader.
        for (const other of patterns.filter(l => state.chips.some(([label]) => label === l) && !pressed.includes(l))) {
          await page.locator('#how button', { hasText: other }).click();
          assert.equal(await page.evaluate(() => current.call), state.call, 'the function stays; the pattern changes');
          assert.equal(await page.evaluate(o => [...document.querySelectorAll('#how button')].find(b => b.textContent === o).getAttribute('aria-pressed'), other), 'true');
          /* Two programs at once are offered nonstop alone. */
          const sharedChip = await page.evaluate(() => [...document.querySelectorAll('#how button')].some(b => b.textContent === 'beside another program'));
          assert.equal(sharedChip, other === 'nonstop', `${state.call}, ${other}: the shared chip`);
          assert(await page.evaluate(() => !!document.querySelector('#chart a[href^="bench-hashes.map.html#"]')), 'the map link follows the pattern');
        }
      }
    }
    endings++;
  }
  await page.locator('#restart').click();
  for (let i = 0; i < 3; i++) await page.locator('#choices button').last().click();
  assert.equal(await page.evaluate(() => current.call), 'hash');
  await page.locator('#back').click();
  assert((await page.locator('#q-title').innerText()).includes('done with the last'));
  await page.locator('#restart').click();
  assert((await page.locator('#q-title').innerText()).includes('several threads'));
  const defaults = await page.evaluate(() => Object.fromEntries(Object.entries(QUESTIONS).map(([k, q]) => [k, q.choices.at(-1)[0]])));
  assert.deepEqual(defaults, { threads: 'one', shape: 'message', keepsUp: 'yes', buffer: 'lent' });
  assert.deepEqual(errors, []);
  console.log(`${routes} table routes, ${endings} clicked endings (${measured} measured), defaults, Back, restart, examples, map links, chips: pass`);
  await browser.close();
})().catch(e => { console.error(e); process.exit(1); });
