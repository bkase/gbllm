// Deterministic offline renderer for film.html.
//
// Launches headless Google Chrome, drives it over the DevTools protocol with
// Node's built-in WebSocket (no npm dependencies), asks the page to draw each
// frame at an exact time, and pipes PNG frames into ffmpeg.
//
//   node render.mjs                      # full film -> ../out/frames.mp4 (video only)
//   node render.mjs --from 20 --to 26    # a slice, for iteration
//   node render.mjs --stills 3,12.5,40   # PNG stills -> ../out/stills/
//
// Muxing with the score is done by build.sh.

import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../out");
mkdirSync(outDir, { recursive: true });

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith("--")) acc.push([a.slice(2), all[i + 1]?.startsWith("--") ? true : all[i + 1] ?? true]);
    return acc;
  }, []),
);
const FPS = Number(args.fps ?? 60);
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function launch() {
  const profile = path.join(outDir, ".chrome-profile");
  rmSync(profile, { recursive: true, force: true });
  const port = 9300 + Math.floor(Math.random() * 500);
  const proc = spawn(
    CHROME,
    [
      "--headless=new",
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${profile}`,
      "--allow-file-access-from-files",
      "--hide-scrollbars",
      "--force-device-scale-factor=1",
      "--window-size=1920,1080",
      "--disable-gpu-vsync",
      "--no-first-run",
      "--no-default-browser-check",
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  for (let i = 0; i < 100; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/list`);
      const targets = await res.json();
      const page = targets.find((t) => t.type === "page");
      if (page) return { proc, wsUrl: page.webSocketDebuggerUrl };
    } catch {}
    await sleep(100);
  }
  proc.kill();
  throw new Error("Chrome did not expose a DevTools page target");
}

function cdp(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
    } else if (msg.method) listeners.forEach((l) => l(msg));
  };
  const ready = new Promise((r) => (ws.onopen = r));
  return {
    ready,
    on: (fn) => listeners.push(fn),
    send(method, params = {}) {
      const n = ++id;
      ws.send(JSON.stringify({ id: n, method, params }));
      return new Promise((resolve, reject) => pending.set(n, { resolve, reject }));
    },
    close: () => ws.close(),
  };
}

// Large CDP messages (> ~4 MiB) stall Node's WebSocket, so frames come back in slices.
async function grabFrame(c, t) {
  const len = await evaluate(c, `(window.__f = window.renderFrame(${t})).length`);
  const parts = [];
  for (let i = 0; i < len; i += 2_000_000) parts.push(evaluate(c, `window.__f.slice(${i}, ${i + 2_000_000})`));
  const url = (await Promise.all(parts)).join("");
  return Buffer.from(url.slice(url.indexOf(",") + 1), "base64");
}

async function evaluate(c, expression) {
  const r = await c.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
}

const { proc, wsUrl } = await launch();
const c = cdp(wsUrl);
await c.ready;
c.on((m) => {
  if (m.method === "Runtime.consoleAPICalled") console.log("[page]", ...m.params.args.map((a) => a.value));
});
await c.send("Runtime.enable");
await c.send("Page.enable");
await c.send("Page.navigate", { url: "file://" + path.join(here, "film.html") + "?render=1" });
for (let i = 0; i < 200; i++) {
  if (await evaluate(c, "window.FILM_READY === true").catch(() => false)) break;
  await sleep(100);
}
const duration = await evaluate(c, "window.FILM_DURATION");

try {
  if (args.stills) {
    const dir = path.join(outDir, "stills");
    mkdirSync(dir, { recursive: true });
    for (const t of String(args.stills).split(",").map(Number)) {
      const file = path.join(dir, `t${t.toFixed(2).padStart(6, "0")}.png`);
      writeFileSync(file, await grabFrame(c, t));
      console.log(file);
    }
  } else {
    const from = Number(args.from ?? 0);
    const to = Math.min(Number(args.to ?? duration), duration);
    const out = path.resolve(args.out ?? path.join(outDir, args.from || args.to ? `slice_${from}_${to}.mp4` : "frames.mp4"));
    const ff = spawn(
      "ffmpeg",
      ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(FPS), "-c:v", "png", "-i", "-",
        "-c:v", "libx264", "-preset", "slow", "-crf", "14", "-pix_fmt", "yuv420p",
        "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", out],
      { stdio: ["pipe", "inherit", "inherit"] },
    );
    const n0 = Math.round(from * FPS);
    const n1 = Math.round(to * FPS);
    const t0 = Date.now();
    for (let n = n0; n < n1; n++) {
      const buf = await grabFrame(c, n / FPS);
      if (!ff.stdin.write(buf)) await new Promise((r) => ff.stdin.once("drain", r));
      if ((n - n0) % FPS === 0) {
        const done = n - n0 + 1;
        const rate = done / ((Date.now() - t0) / 1000);
        process.stdout.write(`\rframe ${n}/${n1}  ${rate.toFixed(1)} fps  eta ${((n1 - n) / rate).toFixed(0)} s   `);
      }
    }
    ff.stdin.end();
    await new Promise((r) => ff.on("close", r));
    console.log("\n" + out);
  }
} finally {
  c.close();
  proc.kill();
}
