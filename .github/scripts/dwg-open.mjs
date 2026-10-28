// Opens real DWG files in the built browser version, in headless Chromium, and writes a screenshot
// and the browser console for each to the output folder.
// Usage: DWG_FILES="a.dwg\nb.dwg" node dwg-open.mjs <site> <out>
import { chromium, devices } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';

const [, , root, out] = process.argv;
const files = (process.env.DWG_FILES || '').split('\n').map((f) => f.trim()).filter(Boolean);
fs.mkdirSync(out, { recursive: true });
const logs = [];
const log = (line) => {
  logs.push(line);
  console.log(line);
};

const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json' };
const server = http
  .createServer((req, res) => {
    let p = decodeURIComponent(req.url.split('?')[0]);
    if (p === '/') p = '/index.html';
    fs.readFile(path.join(root, p), (error, data) => {
      if (error) {
        res.statusCode = 404;
        res.end();
        return;
      }
      res.setHeader('content-type', types[path.extname(p)] || 'application/octet-stream');
      res.end(data);
    });
  })
  .listen(8098);

const browser = await chromium.launch({
  args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--enable-webgl'],
});

log(`DWG files to open: ${files.length}`);
let n = 0;
for (const file of files) {
  n += 1;
  const name = `dwg-${n}`;
  const size = fs.statSync(file).size;
  log(`[${name}] ${path.basename(file)} (${size} bytes)`);
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  page.on('console', (m) => log(`[${name}] console.${m.type()}: ${m.text().slice(0, 400)}`));
  page.on('pageerror', (e) => log(`[${name}] PAGEERROR: ${e.message.slice(0, 400)}`));
  page.on('crash', () => log(`[${name}] PAGE CRASHED`));
  try {
    await page.goto('http://localhost:8098/?webgl', { waitUntil: 'load', timeout: 60000 });
    await page.waitForTimeout(15000);
    await page.screenshot({ path: `${out}/${name}-0-before.png` });
    // Open... (Ctrl+O) starts the browser's file picker; hand it the DWG.
    await page.mouse.click(640, 400);
    const chooser = page.waitForEvent('filechooser', { timeout: 15000 });
    await page.keyboard.press('Control+o');
    try {
      const fc = await chooser;
      await fc.setFiles(file);
      log(`[${name}] file chooser answered`);
    } catch (error) {
      log(`[${name}] no file chooser: ${error.message.split('\n')[0]}`);
    }
    await page.waitForTimeout(30000);
    await page.screenshot({ path: `${out}/${name}-1-opened.png` });
  } catch (error) {
    log(`[${name}] SCRIPT ERROR: ${error.message.split('\n')[0]}`);
    try {
      await page.screenshot({ path: `${out}/${name}-error.png` });
    } catch {}
  }
  await context.close();
}

// Phone pass: the same page in an iPhone-size window, with the last DWG opened.
if (files.length > 0) {
  const name = 'phone';
  const file = files[files.length - 1];
  const context = await browser.newContext({ ...devices['iPhone 14 Pro'] });
  // Headless Chromium reports devicePixelContentBoxSize in CSS pixels at an emulated pixel ratio; Safari does not.
  await context.addInitScript(() => {
    try {
      delete ResizeObserverEntry.prototype.devicePixelContentBoxSize;
    } catch {}
  });
  const page = await context.newPage();
  page.on('console', (m) => log(`[${name}] console.${m.type()}: ${m.text().slice(0, 400)}`));
  page.on('pageerror', (e) => log(`[${name}] PAGEERROR: ${e.message.slice(0, 400)}`));
  page.on('crash', () => log(`[${name}] PAGE CRASHED`));
  try {
    await page.goto('http://localhost:8098/?webgl', { waitUntil: 'load', timeout: 60000 });
    await page.waitForTimeout(15000);
    await page.screenshot({ path: `${out}/phone-0-start.png` });
    await page.mouse.click(195, 420);
    const chooser = page.waitForEvent('filechooser', { timeout: 15000 });
    await page.keyboard.press('Control+o');
    try {
      const fc = await chooser;
      await fc.setFiles(file);
      log(`[${name}] file chooser answered for ${path.basename(file)}`);
    } catch (error) {
      log(`[${name}] no file chooser: ${error.message.split('\n')[0]}`);
    }
    await page.waitForTimeout(30000);
    await page.screenshot({ path: `${out}/phone-1-opened.png` });
  } catch (error) {
    log(`[${name}] SCRIPT ERROR: ${error.message.split('\n')[0]}`);
  }
  await context.close();
}

await browser.close();
server.close();
fs.writeFileSync(`${out}/logs.txt`, logs.join('\n') + '\n');
