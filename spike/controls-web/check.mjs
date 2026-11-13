// Drives the page in Chrome and checks what a person would otherwise have to: the accessibility tree the
// browser makes of the mirror, a real Tab key, typing, and an input method's composition.
//
//   node spike/controls-web/check.mjs own  [picture.png]
//   node spike/controls-web/check.mjs egui [picture.png]
//
// It serves `www/` itself, opens Chrome in a window with an empty profile it deletes afterwards, and talks to
// it over Chrome's debugging protocol. It needs Chrome, a display and a GPU, so it is not run on CI. It exits
// with 1 if a check fails. Lines that start with "result:" are measurements, not checks: the two candidates
// differ there.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const candidate = process.argv[2] ?? 'own';
const picture = process.argv[3];
const www = join(dirname(fileURLToPath(import.meta.url)), 'www');
const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };

const server = createServer((req, res) => {
  const path = normalize(new URL(req.url, 'http://x').pathname).replace(/^(\.\.[/\\])+/, '');
  try {
    const body = readFileSync(join(www, path === '/' ? 'index.html' : path));
    res.writeHead(200, { 'Content-Type': TYPES[extname(path)] ?? 'text/html', 'Cache-Control': 'no-store' });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

const profile = mkdtempSync(join(tmpdir(), 'controls-web-'));
const port = 9400 + Math.floor(Math.random() * 400);
const chrome = spawn(CHROME, [`--user-data-dir=${profile}`, `--remote-debugging-port=${port}`, '--no-first-run',
  '--no-default-browser-check', '--window-size=900,400', 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const check = (what, ok, got) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}${ok ? '' : `: got ${JSON.stringify(got)}`}`);
  if (!ok) failed += 1;
};

try {
  let target;
  for (let i = 0; i < 60 && !target; i++) {
    await sleep(250);
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch {}
  }
  if (!target) throw new Error('Chrome did not start');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let next = 0;
  const waiting = new Map();
  ws.onmessage = (m) => { const d = JSON.parse(m.data); if (waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); } };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++next;
    waiting.set(id, (d) => (d.error ? reject(new Error(`${method}: ${d.error.message}`)) : resolve(d.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
  const js = async (expression) => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
  const state = () => js('window.trial.state');
  const commands = async () => js('window.trial.commands.splice(0)');
  const focusName = async () => js(`(() => { const s = window.trial.state; const n = s.nodes.find((n) => n.id === s.focus); return n ? (n.label ?? n.value ?? n.role) : null; })()`);
  const key = async (name, code, vk, text) => {
    await send('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key: name, code, windowsVirtualKeyCode: vk, text });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code, windowsVirtualKeyCode: vk });
    await sleep(60);
  };
  const tab = () => key('Tab', 'Tab', 9);
  const type = async (text) => { for (const c of text) await key(c, `Key${c.toUpperCase()}`, c.toUpperCase().charCodeAt(0), c); };

  await send('Page.enable');
  await send('Accessibility.enable');
  await send('Page.navigate', { url: `${origin}/index.html?candidate=${candidate}` });
  let title = '';
  for (let i = 0; i < 80 && !/ready|failed/.test(title); i++) { await sleep(250); title = await js('document.title'); }
  check(`the page is ready (${title})`, title.endsWith('ready'), await js("document.getElementById('log').textContent"));
  if (!title.endsWith('ready')) throw new Error('the page did not come up');

  // 1. The accessibility tree the browser makes of the mirror: what a screen reader is given.
  const ax = (await send('Accessibility.getFullAXTree')).nodes.filter((n) => !n.ignored);
  const named = (role, name) => ax.find((n) => n.role?.value === role && n.name?.value === name);
  const prop = (n, p) => n?.properties?.find((x) => x.name === p)?.value?.value;
  check('a toolbar named Tools', !!named('toolbar', 'Tools'));
  check('radio groups named Tool and Colour', !!named('radiogroup', 'Tool') && !!named('radiogroup', 'Colour'));
  for (const t of ['Select', 'Pen', 'Eraser', 'Shape', 'Line']) {
    check(`a radio button ${t}, ${t === 'Pen' ? 'checked' : 'not checked'}`, prop(named('radio', t), 'checked') === (t === 'Pen' ? 'true' : 'false'), named('radio', t));
  }
  for (const c of ['Black', 'Red', 'Orange', 'Green', 'Blue', 'Purple']) {
    check(`a radio button ${c}, ${c === 'Black' ? 'checked' : 'not checked'}`, prop(named('radio', c), 'checked') === (c === 'Black' ? 'true' : 'false'), named('radio', c));
  }
  check('a button Undo, disabled', prop(named('button', 'Undo'), 'disabled') === true, named('button', 'Undo'));
  check('a button Redo', !!named('button', 'Redo'));
  check('the zoom figure as text', ax.some((n) => n.name?.value === 'Zoom 100%' || n.value?.value === 'Zoom 100%'));
  const field = ax.find((n) => n.role?.value === 'textbox' && n.name?.value === 'Hex colour');
  check('a text field named Hex colour holding 202020', field?.value?.value === '202020', field);

  // 2. The pointer goes to the canvas, through the mirror that lies over it.
  const centre = async (label) => js(`(() => { const n = window.trial.state.nodes.find((n) => n.label === ${JSON.stringify(label)}); const r = document.getElementById('canvas').getBoundingClientRect(); return [r.left + (n.bounds[0] + n.bounds[2]) / 2, r.top + (n.bounds[1] + n.bounds[3]) / 2]; })()`);
  const click = async (label) => {
    const [x, y] = await centre(label);
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
    await sleep(120);
  };
  await click('Eraser');
  check('a click on Eraser gives SetTool(Eraser)', JSON.stringify(await commands()) === '["SetTool(Eraser)"]');
  await click('Green');
  check('a click on Green gives SetColour', (await commands()).join() === 'SetColour([48, 160, 80])');

  // 3. What a screen reader does to activate: a click on the mirror's element.
  await js(`document.querySelector('#mirror [aria-label="Line"]').click()`);
  await sleep(120);
  check('a click on the mirror element Line gives SetTool(Line)', (await commands()).join() === 'SetTool(Line)');
  const ax2 = (await send('Accessibility.getFullAXTree')).nodes.filter((n) => !n.ignored);
  check('the browser now says Line is checked and Pen is not',
    prop(ax2.find((n) => n.role?.value === 'radio' && n.name?.value === 'Line'), 'checked') === 'true'
    && prop(ax2.find((n) => n.role?.value === 'radio' && n.name?.value === 'Pen'), 'checked') === 'false');
  check('and that Undo is no longer disabled', prop(ax2.find((n) => n.role?.value === 'button' && n.name?.value === 'Undo'), 'disabled') !== true);

  // 4. A real Tab key, from outside the controls.
  await js('document.activeElement.blur()');
  await send('Page.reload');
  title = '';
  for (let i = 0; i < 80 && !/ready|failed/.test(title); i++) { await sleep(250); title = await js('document.title'); }
  const stops = [];
  for (let i = 0; i < 30; i++) {
    await tab();
    const name = await focusName();
    const dom = await js(`document.activeElement.getAttribute('aria-label') ?? document.activeElement.tagName`);
    stops.push(name);
    if (i === 0) check(`the first Tab puts the DOM's focus and the controls' focus on the same control (${name})`, dom === name, dom);
    if (name === 'Hex colour') break;
  }
  check('Tab reaches the hex field', stops.at(-1) === 'Hex colour', stops);
  console.log(`result: ${stops.length} presses of Tab to the field: ${stops.join(', ')}`);
  check('the DOM focus is on the textarea', await js('document.activeElement instanceof HTMLTextAreaElement'));

  // 5. Typing in the field, with real key events.
  for (let i = 0; i < 6; i++) await key('Backspace', 'Backspace', 8);
  check('Backspace empties the field', (await state()).textInput?.value === '', (await state()).textInput);
  await type('1a2b');
  check('typed characters go in once each', (await state()).textInput?.value === '1a2b', (await state()).textInput);
  check('the textarea holds the same text, for a screen reader', await js('document.activeElement.value') === '1a2b');

  // 6. A dead key, as an input method's composition: the accent, then the letter.
  await send('Input.imeSetComposition', { text: '´', selectionStart: 1, selectionEnd: 1 });
  await sleep(120);
  let t = (await state()).textInput;
  check('the accent is being composed and is not in the value', t?.preedit === '´' && t?.value === '1a2b' && t?.shown === '1a2b´', t);
  if (picture) {
    const box = await js(`(() => { const r = document.getElementById('stage').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()`);
    const shot = await send('Page.captureScreenshot', { format: 'png', clip: { x: box[0] - 4, y: box[1] - 4, width: box[2] + 8, height: box[3] + 8, scale: 2 } });
    writeFileSync(picture, Buffer.from(shot.data, 'base64'));
    console.log(`wrote ${picture}`);
  }
  await send('Input.insertText', { text: 'é' });
  await sleep(120);
  t = (await state()).textInput;
  check('the letter completes it, once', t?.preedit == null && t?.value === '1a2bé', t);

  // 7. A composition given up.
  await send('Input.imeSetComposition', { text: '`', selectionStart: 1, selectionEnd: 1 });
  await sleep(100);
  await send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 });
  await sleep(120);
  t = (await state()).textInput;
  check('a composition given up leaves the field as it was', t?.preedit == null && t?.value === '1a2bé', t);

  // 8. An input method that composes in steps (a smoke test).
  await key('Backspace', 'Backspace', 8);
  for (const text of ['に', 'にほ', '日本']) {
    await send('Input.imeSetComposition', { text, selectionStart: text.length, selectionEnd: text.length });
    await sleep(100);
  }
  t = (await state()).textInput;
  check('steps of a composition replace one another', t?.preedit === '日本' && t?.value === '1a2b', t);
  await send('Input.insertText', { text: '日本' });
  await sleep(120);
  t = (await state()).textInput;
  check('and its text goes in once', t?.preedit == null && t?.value === '1a2b日本', t);
  // With the field full (six characters), is a seventh still shown while it is being composed?
  await send('Input.imeSetComposition', { text: '´', selectionStart: 1, selectionEnd: 1 });
  await sleep(120);
  t = (await state()).textInput;
  console.log(`result: composing into a full field shows ${JSON.stringify(t?.shown)} (composing ${JSON.stringify(t?.preedit)})`);
  await send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 });
  await sleep(100);

  // 9. Enter gives a colour.
  for (let i = 0; i < 8; i++) await key('Backspace', 'Backspace', 8);
  await type('00ff00');
  await commands();
  await key('Enter', 'Enter', 13);
  check('Enter in the field gives the colour', (await commands()).join() === 'SetColour([0, 255, 0])');

  // 10. The arrows in the tool group: the candidates differ.
  await send('Page.reload');
  title = '';
  for (let i = 0; i < 80 && !/ready|failed/.test(title); i++) { await sleep(250); title = await js('document.title'); }
  await tab();
  const before = await focusName();
  await key('ArrowRight', 'ArrowRight', 39);
  console.log(`result: Tab focuses ${before}; the right arrow then focuses ${await focusName()} and gives ${JSON.stringify(await commands())}`);
  ws.close();
} catch (e) {
  console.log(`FAIL ${e.message}`);
  failed += 1;
} finally {
  chrome.kill();
  server.close();
  await sleep(500);
  rmSync(profile, { recursive: true, force: true });
}
console.log(failed ? `${candidate}: ${failed} checks failed` : `${candidate}: every check passed`);
process.exit(failed ? 1 : 0);
