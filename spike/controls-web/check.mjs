// Drives the page in Chrome and checks what a person would otherwise have to: the accessibility tree the
// browser makes of the mirror, Tab keys, typing, and an input method's composition.
//
//   node spike/controls-web/check.mjs own  [picture.png]
//   node spike/controls-web/check.mjs egui [picture.png]
//
// It serves `www/` itself, opens Chrome in a window with an empty profile it deletes afterwards, and talks to
// it over Chrome's debugging protocol: the keys and the text go through the browser's own input path, but no
// person presses them and no screen reader listens. It needs Chrome, a display and a GPU, so it is not run on
// CI. It exits with 1 if a check fails. Lines that start with "result:" are measurements, not checks: the two
// candidates differ there.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const candidate = process.argv[2] ?? 'own';
const picture = process.argv[3];
const www = join(dirname(fileURLToPath(import.meta.url)), 'www');
const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };

// Files under www/ and nothing else, to this machine only.
const server = createServer((req, res) => {
  const name = new URL(req.url, 'http://x').pathname;
  const file = resolve(www, `.${name === '/' ? '/index.html' : name}`);
  if (!file.startsWith(www + sep) || !existsSync(file)) return res.writeHead(404).end();
  res.writeHead(200, { 'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream', 'Cache-Control': 'no-store' });
  res.end(readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const check = (what, ok, got) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}${ok ? '' : `: got ${JSON.stringify(got)}`}`);
  if (!ok) failed += 1;
};

const profile = mkdtempSync(join(tmpdir(), 'controls-web-'));
let chrome;
try {
  // Port 0: Chrome picks a free one and writes it into its profile, so this never talks to another browser.
  chrome = spawn(CHROME, [`--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run',
    '--no-default-browser-check', '--window-size=900,400', 'about:blank'], { stdio: 'ignore' });
  const started = new Promise((_, reject) => chrome.on('error', (e) => reject(new Error(`Chrome did not start: ${e.message}`))));
  const port = await Promise.race([started, (async () => {
    const file = join(profile, 'DevToolsActivePort');
    for (let i = 0; i < 80; i++) {
      await sleep(250);
      if (existsSync(file)) return readFileSync(file, 'utf8').split('\n')[0];
    }
    throw new Error('Chrome did not say which port it listens on');
  })()]);
  let target;
  for (let i = 0; i < 40 && !target; i++) {
    await sleep(250);
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page'); } catch {}
  }
  if (!target) throw new Error('Chrome has no page');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let next = 0;
  const waiting = new Map();
  ws.onmessage = (m) => { const d = JSON.parse(m.data); if (waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); } };
  const send = (method, params = {}) => new Promise((done, reject) => {
    const id = ++next;
    waiting.set(id, (d) => (d.error ? reject(new Error(`${method}: ${d.error.message}`)) : done(d.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
  const js = async (expression) => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
  const state = () => js('window.trial.state');
  const commands = async () => js('window.trial.commands.splice(0)');
  // The control that has the controls' focus, by name; null when none has.
  const focusName = async () => js(`(() => { const s = window.trial.state; if (s.focus === s.root) return null; const n = s.nodes.find((n) => n.id === s.focus); return n ? (n.label ?? n.value ?? n.role) : null; })()`);
  // The mirror element that has the DOM's focus, by name; null when the focus is outside the mirror.
  const domFocus = async () => js(`(() => { const a = document.activeElement; return document.getElementById('mirror').contains(a) ? a.getAttribute('aria-label') : null; })()`);
  const key = async (name, code, vk, opts = {}) => {
    const base = { key: name, code, windowsVirtualKeyCode: vk, modifiers: opts.shift ? 8 : 0 };
    await send('Input.dispatchKeyEvent', { type: opts.text ? 'keyDown' : 'rawKeyDown', ...base, text: opts.text, commands: opts.commands });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
    await sleep(70);
  };
  const tab = (shift = false) => key('Tab', 'Tab', 9, { shift });
  const backspace = () => key('Backspace', 'Backspace', 8);
  const type = async (text) => { for (const c of text) await key(c, `Key${c.toUpperCase()}`, c.toUpperCase().charCodeAt(0), { text: c }); };
  // Loads the page afresh and waits for it, not for the title the last load left behind.
  const load = async () => {
    await js("document.title = 'controls-web: leaving'").catch(() => {});
    await send('Page.navigate', { url: `${origin}/index.html?candidate=${candidate}` });
    let title = '';
    for (let i = 0; i < 100 && !/ready|failed/.test(title); i++) { await sleep(200); title = await js('document.title').catch(() => ''); }
    if (!title.endsWith('ready')) throw new Error(`the page did not come up: ${title} ${await js("document.getElementById('log').textContent")}`);
    return title;
  };
  const tree = async () => (await send('Accessibility.getFullAXTree')).nodes.filter((n) => !n.ignored);
  const prop = (n, p) => n?.properties?.find((x) => x.name === p)?.value?.value;
  const watchMirror = () => js(`(() => { window.trial.touched = 0; new MutationObserver((m) => { window.trial.touched += m.length; }).observe(document.getElementById('mirror'), { subtree: true, childList: true, characterData: true }); })()`);

  await send('Page.enable');
  await send('Accessibility.enable');
  // Without this a window that is not in front gets no key events at all, and every key check fails or,
  // worse, measures nothing.
  await send('Emulation.setFocusEmulationEnabled', { enabled: true });
  await send('Page.bringToFront');
  await send('Browser.grantPermissions', { origin, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'] });
  check(`the page is ready (${await load()})`, true);

  // 1. The accessibility tree the browser makes of the mirror: what a screen reader is given.
  let ax = await tree();
  const named = (role, name) => ax.find((n) => n.role?.value === role && n.name?.value === name);
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
  check('the zoom figure as text', ax.some((n) => n.name?.value === 'Zoom 100%'));
  check('which is not a live region', !ax.some((n) => n.role?.value === 'status'));
  const field = ax.find((n) => n.role?.value === 'textbox' && n.name?.value === 'Hex colour');
  check('a text field named Hex colour holding 202020', field?.value?.value === '202020', field);

  // 2. Every mirror element is really where its control is drawn: not clipped away, and the one on top there.
  const hidden = await js(`(() => {
    const out = [];
    const stage = document.getElementById('stage').getBoundingClientRect();
    const style = document.createElement('style');
    style.textContent = '#mirror, #mirror * { pointer-events: auto !important; }';
    document.head.appendChild(style);
    for (const n of window.trial.state.nodes) {
      if (!n.bounds || n.children.length) continue;
      const el = document.querySelector('#mirror [data-node="' + n.id + '"]');
      if (!el) continue;
      const x = stage.left + 1 + (n.bounds[0] + n.bounds[2]) / 2, y = stage.top + 1 + (n.bounds[1] + n.bounds[3]) / 2;
      if (document.elementFromPoint(x, y) !== el) out.push(n.label ?? n.value);
    }
    style.remove();
    return out;
  })()`);
  check('every mirror element is found at the middle of its control', hidden.length === 0, hidden);

  // 3. The pointer goes to the canvas, through the mirror that lies over it.
  const centre = async (label) => js(`(() => { const n = window.trial.state.nodes.find((n) => n.label === ${JSON.stringify(label)}); const r = document.getElementById('canvas').getBoundingClientRect(); return [r.left + (n.bounds[0] + n.bounds[2]) / 2, r.top + (n.bounds[1] + n.bounds[3]) / 2]; })()`);
  const click = async (label) => {
    const [x, y] = await centre(label);
    await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
    await sleep(120);
  };
  await click('Eraser');
  check('a pointer click on Eraser gives SetTool(Eraser)', (await commands()).join() === 'SetTool(Eraser)');
  await click('Green');
  check('a pointer click on Green gives SetColour', (await commands()).join() === 'SetColour([48, 160, 80])');

  // 4. A click on the mirror's own element (`element.click()`), standing in for a screen reader's activation.
  await js(`document.querySelector('#mirror [aria-label="Line"]').click()`);
  await sleep(120);
  check('a click on the mirror element Line gives SetTool(Line)', (await commands()).join() === 'SetTool(Line)');
  ax = await tree();
  check('the browser now says Line is checked and Pen is not', prop(named('radio', 'Line'), 'checked') === 'true' && prop(named('radio', 'Pen'), 'checked') === 'false');
  check('and that Undo is there and no longer disabled', !!named('button', 'Undo') && prop(named('button', 'Undo'), 'disabled') !== true, named('button', 'Undo'));
  await js(`document.querySelector('#mirror [aria-label="Redo"]').click()`);
  await sleep(120);
  check('a click on the mirror element of a disabled button gives nothing', (await commands()).length === 0);

  // 5. Tab from outside the controls, forwards to the field. The DOM's focus and the controls' agree at each stop.
  await load();
  const stops = [];
  let agree = true;
  for (let i = 0; i < 30; i++) {
    await tab();
    const [name, dom] = [await focusName(), await domFocus()];
    stops.push(name);
    if (name !== dom) agree = false;
    if (name === 'Hex colour' || name === null) break;
  }
  check('the first Tab enters the controls', stops[0] !== null, stops);
  check('Tab reaches the hex field', stops.at(-1) === 'Hex colour', stops);
  check("at every stop the DOM's focus and the controls' focus are on the same control", agree, stops);
  console.log(`result: ${stops.length} presses of Tab to the field: ${stops.join(', ')}`);
  check('the DOM focus is on the textarea', await js('document.activeElement instanceof HTMLTextAreaElement'));

  // 6. Tab past the field leaves the controls; and back in again with Shift and Tab.
  await tab();
  check('Tab past the field leaves the controls', (await focusName()) === null && (await domFocus()) === null, [await focusName(), await domFocus()]);
  await js('document.activeElement.blur()');
  const back = [];
  agree = true;
  for (let i = 0; i < 30; i++) {
    await tab(true);
    const [name, dom] = [await focusName(), await domFocus()];
    if (name !== dom) agree = false;
    if (name === null || (back.length > 1 && name === back[0])) { back.push(name); break; }
    back.push(name);
  }
  check('Shift and Tab comes in at the field', back[0] === 'Hex colour', back);
  check("going back, the DOM's focus and the controls' focus agree at every stop", agree, back);
  console.log(`result: Shift and Tab from outside: ${back.map((b) => b ?? '(leaves the controls)').join(', ')}`);

  // 7. Typing in the field, with key events.
  await load();
  for (let i = 0; i < 30 && (await focusName()) !== 'Hex colour'; i++) await tab();
  for (let i = 0; i < 6; i++) await backspace();
  check('Backspace empties the field', (await state()).textInput?.value === '', (await state()).textInput);
  await type('1a');
  check('typed characters go in once each', (await state()).textInput?.value === '1a', (await state()).textInput);
  await js("navigator.clipboard.writeText('2b')");
  await key('v', 'KeyV', 86, { commands: ['paste'] });
  await sleep(120);
  check('pasted text goes in', (await state()).textInput?.value === '1a2b', [(await state()).textInput, await js('window.trial.events.slice(-3)')]);
  check('the textarea holds the same text, for a screen reader', await js('document.activeElement.value') === '1a2b');

  // 8. A dead key, as an input method's composition: the accent, then the letter.
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

  // 9. A composition given up.
  await send('Input.imeSetComposition', { text: '`', selectionStart: 1, selectionEnd: 1 });
  await sleep(100);
  await send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 });
  await sleep(120);
  t = (await state()).textInput;
  check('a composition given up leaves the field as it was', t?.preedit == null && t?.value === '1a2bé', t);

  // 10. An input method that composes in steps (a smoke test).
  await backspace();
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
  console.log(`result: composing into a full field shows ${JSON.stringify(t?.shown)}`);
  await send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 });
  await sleep(100);

  // 11. Enter gives a colour.
  for (let i = 0; i < 8; i++) await backspace();
  await type('00ff00');
  await commands();
  await key('Enter', 'Enter', 13);
  check('Enter in the field gives the colour', (await commands()).join() === 'SetColour([0, 255, 0])');

  // 12. The arrows in the tool group, and Escape: the candidates differ.
  await load();
  await tab();
  const before = await focusName();
  check('Tab enters the controls again', before !== null, before);
  await commands();
  await key('ArrowRight', 'ArrowRight', 39);
  console.log(`result: Tab focuses ${before}; the right arrow then focuses ${await focusName()} and gives ${JSON.stringify(await commands())}`);

  // 13. Keys that change no text rewrite no text in the mirror (the zoom figure is not announced over and over).
  await watchMirror();
  await key('ArrowRight', 'ArrowRight', 39);
  await key('ArrowLeft', 'ArrowLeft', 37);
  check('keys that change no text rewrite no text in the mirror', (await js('window.trial.touched')) === 0, await js('window.trial.touched'));
  await key('Escape', 'Escape', 27);
  console.log(`result: after Escape the controls' focus is on ${(await focusName()) ?? 'nothing'} and the DOM's on ${(await domFocus()) ?? 'nothing in the mirror'}`);
  ws.close();
} catch (e) {
  console.log(`FAIL ${e.message}`);
  failed += 1;
} finally {
  chrome?.kill();
  server.close();
  await sleep(500);
  rmSync(profile, { recursive: true, force: true });
}
console.log(failed ? `${candidate}: ${failed} checks failed` : `${candidate}: every check passed`);
process.exit(failed ? 1 : 0);
