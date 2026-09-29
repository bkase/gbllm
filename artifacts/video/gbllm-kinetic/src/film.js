// gbllm kinetic typography film. Pure function of time: film.draw(t).
// Canvas 1920x1080. Cue times live in timeline.js; real evidence in data.js.

function createFilm(canvas) {
  const W = 1920, H = 1080;
  const ctx = canvas.getContext("2d");
  const TL = globalThis.TIMELINE;
  const D = globalThis.FILM_DATA;

  // ---------------------------------------------------------------- palette
  const C = {
    bg: "#0a0a09",
    cream: "#ede7d9",
    dim: "rgba(237,231,217,0.42)",
    faint: "rgba(237,231,217,0.14)",
    lime: "#bde33f",
    amber: "#f3b75c",
    ember: "#e0763c",
  };
  const F = {
    disp: (px) => `${px}px GBDisplay`,
    bold: (px) => `700 ${px}px "Helvetica Neue"`,
    med: (px) => `500 ${px}px "Helvetica Neue"`,
    ital: (px) => `italic 300 ${px}px "Helvetica Neue"`,
    mono: (px) => `${px}px GBMono`,
    monob: (px) => `700 ${px}px GBMono`,
  };

  // ---------------------------------------------------------------- math
  const clamp = (x, a = 0, b = 1) => Math.max(a, Math.min(b, x));
  const seg = (t, a, b) => clamp((t - a) / (b - a));
  const lerp = (a, b, p) => a + (b - a) * p;
  const E = {
    outExpo: (x) => (x >= 1 ? 1 : 1 - Math.pow(2, -10 * x)),
    inExpo: (x) => (x <= 0 ? 0 : Math.pow(2, 10 * x - 10)),
    outCubic: (x) => 1 - Math.pow(1 - x, 3),
    inCubic: (x) => x * x * x,
    inOut: (x) => (x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2),
    outBack: (x) => { const c1 = 2.2, c3 = c1 + 1; return 1 + c3 * Math.pow(x - 1, 3) + c1 * Math.pow(x - 1, 2); },
  };
  function mulberry(seed) {
    return () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  const hash = (i) => { let x = Math.sin(i * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };

  // ---------------------------------------------------------------- text
  function text(str, x, y, o = {}) {
    ctx.save();
    ctx.font = o.font ?? F.bold(40);
    ctx.fillStyle = o.color ?? C.cream;
    ctx.textAlign = o.align ?? "left";
    ctx.textBaseline = o.base ?? "alphabetic";
    ctx.letterSpacing = (o.ls ?? 0) + "px";
    ctx.globalAlpha *= o.alpha ?? 1;
    if (o.blur) ctx.filter = `blur(${o.blur}px)`;
    const s = o.scale ?? 1;
    ctx.translate(x, y);
    if (o.rot) ctx.rotate(o.rot);
    ctx.scale(s * (o.sx ?? 1), s * (o.sy ?? 1));
    if (o.glow) { ctx.shadowColor = o.glow; ctx.shadowBlur = o.glowBlur ?? 30; }
    ctx.fillText(str, 0, 0);
    ctx.restore();
  }
  function measure(str, font, ls = 0) {
    ctx.save();
    ctx.font = font;
    ctx.letterSpacing = ls + "px";
    const w = ctx.measureText(str).width;
    ctx.restore();
    return w;
  }
  // Largest px size (<= max) at which `str` fits `width`.
  function fit(str, fontFn, width, max) {
    const w = measure(str, fontFn(100));
    return Math.min(max, Math.floor((100 * width) / w));
  }
  // Slam-in: overshoot scale collapsing to 1, quick alpha.
  function slam(t, t0, dur = 0.24, amt = 0.55) {
    const p = seg(t, t0, t0 + dur);
    return { a: seg(t, t0, t0 + 0.05), s: 1 + amt * (1 - E.outExpo(p)), p };
  }
  // A run of words laid out on one line; each word slams in at its own time.
  function wordLine(words, cx, y, font, t, o = {}) {
    const gap = o.gap ?? measure(" ", font);
    const widths = words.map((w) => measure(w.s, font, o.ls ?? 0));
    const total = widths.reduce((a, b) => a + b, 0) + gap * (words.length - 1);
    let x = o.align === "left" ? cx : cx - total / 2;
    words.forEach((w, i) => {
      const k = slam(t, w.t, 0.22, o.amt ?? 0.6);
      if (t >= w.t) {
        text(w.s, x + widths[i] / 2, y, { font, color: w.c ?? o.color ?? C.cream, align: "center", scale: k.s, alpha: k.a * (o.alpha ?? 1), ls: o.ls, glow: w.glow });
      }
      x += widths[i] + gap;
    });
    return total;
  }
  function typed(str, t, t0, dt) { return str.slice(0, clamp(Math.floor((t - t0) / dt) + 1, 0, str.length)); }
  const blink = (t, rate = 2) => Math.floor(t * rate * 2) % 2 === 0;

  function rect(x, y, w, h, color, alpha = 1) {
    ctx.save(); ctx.globalAlpha *= alpha; ctx.fillStyle = color; ctx.fillRect(x, y, w, h); ctx.restore();
  }
  function strokeRect(x, y, w, h, color, lw = 2, alpha = 1) {
    ctx.save(); ctx.globalAlpha *= alpha; ctx.strokeStyle = color; ctx.lineWidth = lw; ctx.strokeRect(x, y, w, h); ctx.restore();
  }
  function line(x0, y0, x1, y1, color, lw = 2, alpha = 1) {
    ctx.save(); ctx.globalAlpha *= alpha; ctx.strokeStyle = color; ctx.lineWidth = lw;
    ctx.beginPath(); ctx.moveTo(x0, y0); ctx.lineTo(x1, y1); ctx.stroke(); ctx.restore();
  }
  function label(str, x, y, o = {}) {
    text(str.toUpperCase(), x, y, { font: F.mono(o.size ?? 20), color: o.color ?? C.dim, ls: o.ls ?? 4, align: o.align, alpha: o.alpha });
  }

  // ---------------------------------------------------------------- textures
  const grain = [0, 1, 2, 3].map((k) => {
    const g = document.createElement("canvas");
    g.width = g.height = 384;
    const gx = g.getContext("2d");
    const img = gx.createImageData(384, 384);
    const r = mulberry(99 + k);
    for (let i = 0; i < img.data.length; i += 4) {
      const v = r() * 255;
      img.data[i] = img.data[i + 1] = img.data[i + 2] = v;
      img.data[i + 3] = 16;
    }
    gx.putImageData(img, 0, 0);
    return gx.createPattern(g, "repeat");
  });
  const vignette = (() => {
    const v = document.createElement("canvas");
    v.width = W; v.height = H;
    const vx = v.getContext("2d");
    const g = vx.createRadialGradient(W / 2, H / 2, H * 0.25, W / 2, H / 2, H * 1.05);
    g.addColorStop(0, "rgba(0,0,0,0)");
    g.addColorStop(1, "rgba(0,0,0,0.75)");
    vx.fillStyle = g; vx.fillRect(0, 0, W, H);
    return v;
  })();
  // The authentic framebuffer, pixel for pixel (values are 0 or 255).
  const fb = (() => {
    const c = document.createElement("canvas");
    c.width = D.framebuffer.width; c.height = D.framebuffer.height;
    const g = c.getContext("2d");
    const img = g.createImageData(c.width, c.height);
    const raw = atob(D.framebuffer.gray_b64);
    for (let i = 0; i < raw.length; i++) {
      const v = raw.charCodeAt(i);
      img.data[i * 4] = img.data[i * 4 + 1] = img.data[i * 4 + 2] = v;
      img.data[i * 4 + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    return c;
  })();

  // ---------------------------------------------------------------- global fx
  function shakeAt(t) {
    let x = 0, y = 0;
    for (const c of TL.slams) {
      const d = t - c;
      if (d < 0 || d > 0.4) continue;
      const a = 9 * Math.exp(-d * 16);
      x += a * Math.sin(d * 90 + c);
      y += a * Math.cos(d * 77 + c * 3);
    }
    return [x, y];
  }
  function flashAt(t) {
    let a = 0;
    for (const c of TL.flashes) { const d = t - c; if (d >= 0 && d < 0.4) a += 0.55 * Math.exp(-d * 13); }
    return a;
  }

  // Background letter constellation (conceptual; warm, drifting).
  const GLYPHS = "abcdefghijklmnopqrstuvwxyzOnceuponatime.,";
  const motes = Array.from({ length: 170 }, (_, i) => {
    const r = mulberry(1000 + i);
    return { x: r() * W, y: r() * H, z: 0.25 + r() * 0.9, ch: GLYPHS[Math.floor(r() * GLYPHS.length)], ph: r() * 6.28, sp: 8 + r() * 26 };
  });
  function constellation(t, alpha, drift = 1) {
    if (alpha <= 0) return;
    for (const m of motes) {
      const x = ((m.x - t * m.sp * m.z * drift) % W + W) % W;
      const y = m.y + Math.sin(t * 0.6 + m.ph) * 14 * m.z;
      const tw = 0.55 + 0.45 * Math.sin(t * 2.1 + m.ph * 3);
      text(m.ch, x, y, { font: F.mono(10 + 26 * m.z), color: C.amber, alpha: alpha * tw * (0.15 + 0.5 * m.z), align: "center" });
    }
  }
  function crosses(alpha) {
    for (let gx = 0; gx < 7; gx++) for (let gy = 0; gy < 4; gy++) {
      const x = 120 + gx * 280, y = 150 + gy * 260;
      line(x - 7, y, x + 7, y, C.cream, 1, alpha * 0.18);
      line(x, y - 7, x, y + 7, C.cream, 1, alpha * 0.18);
    }
  }

  // ================================================================ SCENES
  // S1 cold open: a cursor, a prompt.
  function sOpen(t) {
    const o = TL.typeOpen;
    constellation(t, seg(t, 0.3, 2.5) * 0.6, 0.6);
    const font = F.mono(64);
    const full = o.text;
    const w = measure(full, font);
    const x0 = W / 2 - w / 2;
    const zoom = 1 + 3 * E.inExpo(seg(t, 3.3, 4.0));
    const fade = 1 - seg(t, 3.75, 4.0);
    ctx.save();
    ctx.translate(W / 2, 560); ctx.scale(zoom, zoom); ctx.translate(-W / 2, -560);
    ctx.globalAlpha = fade;
    const s = typed(full, t, o.t0, o.dt);
    if (t >= o.t0) text(s, x0, 580, { font, color: C.cream });
    const cx = x0 + measure(t >= o.t0 ? s : "", font);
    const typing = t >= o.t0 && t < o.t0 + o.dt * full.length;
    if (typing || blink(t, 1.6)) rect(cx + 6, 528, 34, 62, C.lime);
    label("prompt", x0, 470, { alpha: seg(t, 0.4, 0.9) });
    ctx.restore();
  }

  // S2 hook.
  function sHook(t) {
    constellation(t, 0.35);
    const out = E.inExpo(seg(t, 8.75, 9.0));
    ctx.save();
    ctx.translate(-out * 2200, 0);
    // "A LANGUAGE MODEL" slams, then lifts and shrinks.
    const lift = E.outExpo(seg(t, 6.0, 6.35));
    ctx.save();
    ctx.translate(W / 2, lerp(560, 330, lift));
    ctx.scale(lerp(1, 0.62, lift), lerp(1, 0.62, lift));
    wordLine([{ s: "A", t: 4.0 }, { s: "LANGUAGE", t: 4.5 }, { s: "MODEL", t: 5.0 }], 0, 90, F.disp(fit("A LANGUAGE MODEL", F.disp, 1720, 260)), t, { ls: 2 });
    ctx.restore();
    wordLine([{ s: "RUNS", t: 6.0 }, { s: "ON", t: 6.0 }, { s: "AN", t: 6.5 }, { s: "ORIGINAL", t: 6.5 }], W / 2, 520, F.disp(120), t, { color: C.dim, amt: 0.4 });
    const g = slam(t, 7.0, 0.3, 0.9);
    if (t >= 7.0) text("GAME BOY.", W / 2, 850, { font: F.disp(330), color: C.lime, align: "center", scale: g.s, alpha: g.a, ls: 4, glow: "rgba(189,227,63,0.35)", glowBlur: 50 });
    if (t >= 7.9) text(typed("(yes, the one from 1989.)", t, 7.9, 0.025), W / 2, 950, { font: F.ital(40), color: C.dim, align: "center" });
    ctx.restore();
    // Negations.
    if (t >= 9.0) {
      const inn = 1 - E.outExpo(seg(t, 9.0, 9.25));
      ctx.save();
      ctx.translate(inn * 1400, 0);
      const rows = [["NO NETWORK.", 9.0], ["NO HOST PC.", 9.5], ["NO COPROCESSOR.", 10.0]];
      rows.forEach(([s, t0], i) => {
        const k = slam(t, t0, 0.2, 0.35);
        if (t >= t0) text(s, 260, 330 + i * 150, { font: F.disp(140), color: C.cream, alpha: k.a * (t > 10.5 ? 0.35 : 1), scale: k.s, ls: 2 });
        if (t >= t0) label(`0${i + 1}`, 200, 280 + i * 150, { alpha: k.a });
      });
      const k = slam(t, 10.5, 0.3, 0.7);
      if (t >= 10.5) text("JUST THE CARTRIDGE.", 260, 880, { font: F.disp(fit("JUST THE CARTRIDGE.", F.disp, 1500, 190)), color: C.lime, alpha: k.a, scale: k.s, ls: 2, glow: "rgba(189,227,63,0.3)" });
      ctx.restore();
    }
    // exit: collapse to a line
    const ex = E.inExpo(seg(t, 11.65, 12.0));
    if (ex > 0) rect(0, 0, W, H, C.bg, ex);
  }

  // S3 the machine: four specs, each full-frame then docked in a strip.
  const SPECS = [
    { t: 12.0, big: "8-BIT", sub: "LR35902 CPU · 4.19 MHz", dock: "8-bit CPU" },
    { t: 13.5, big: "8 KiB", sub: "of work RAM. For everything.", dock: "8 KiB work RAM" },
    { t: 15.0, big: "NO FPU", sub: "no floating point. integers only.", dock: "no floating point" },
    { t: 16.5, big: "8 MiB", sub: "one MBC5 cartridge. that's the budget.", dock: "8 MiB cartridge" },
  ];
  function sMachine(t) {
    crosses(1);
    label("the machine", 120, 110, { color: C.lime });
    label("Nintendo Game Boy · DMG", 1800, 110, { align: "right" });
    SPECS.forEach((s, i) => {
      const t1 = s.t + 1.5;
      const dockX = 120 + i * 440, dockY = 1000;
      if (t < s.t) return;
      const k = slam(t, s.t, 0.26, 0.7);
      const d = E.inOut(seg(t, t1 - 0.25, t1 + 0.1));
      // background ghost for NO FPU: 3.14159 loses its decimals
      if (i === 2 && t < t1) {
        const pi = "3.14159";
        const keep = 7 - Math.floor(seg(t, 15.2, 16.1) * 6);
        text(pi.slice(0, Math.max(1, keep)).replace(/\.$/, ""), W / 2, 700, { font: F.disp(560), color: C.faint, align: "center", alpha: 1 - d });
      }
      if (d < 1) {
        const x = lerp(W / 2, dockX + 90, d), y = lerp(610, dockY - 20, d);
        text(s.big, x, y, { font: F.disp(380), color: i === 2 ? C.amber : C.cream, align: "center", scale: k.s * lerp(1, 0.16, d), alpha: k.a });
        text(s.sub, W / 2, 740, { font: F.mono(44), color: C.amber, align: "center", alpha: k.a * (1 - d * 3) });
      } else {
        const dk = t >= 18.0 ? 0.5 : 1;
        text(s.big, dockX, dockY - 8, { font: F.disp(62), color: i === 2 ? C.amber : C.cream, alpha: dk });
        label(s.dock, dockX, dockY + 28, { size: 18, alpha: dk });
        line(dockX, dockY - 70, dockX + 360, dockY - 70, C.faint, 1, dk);
      }
    });
    if (t >= 18.0) {
      const k1 = slam(t, 18.0, 0.25, 0.3);
      text("We didn't shrink a model to fit.", W / 2, 470, { font: F.bold(84), align: "center", color: C.dim, alpha: k1.a, scale: k1.s, ls: -1 });
      if (t >= 18.9) {
        const k2 = slam(t, 18.9, 0.3, 0.5);
        text("We built one for the machine.", W / 2, 610, { font: F.bold(100), align: "center", color: C.lime, alpha: k2.a, scale: k2.s, ls: -1 });
      }
    }
    const ex = E.inExpo(seg(t, 19.7, 20.0));
    if (ex > 0) rect(0, 0, W, H, C.bg, ex);
  }

  // S4 the build pipeline: seven stations under a progress rail.
  const STATIONS = ["DATA", "TOKENS", "DISTILL", "QUANTIZE", "COMPILE", "CARTRIDGE", "RUNTIME"];
  function rail(t) {
    const st = TL.stations;
    const x0 = 120, x1 = 1800, y = 92;
    const idx = st.findIndex((s, i) => t >= s && t < st[i + 1]);
    line(x0, y + 22, x1, y + 22, C.faint, 2);
    const segW = (x1 - x0) / 7;
    const prog = idx + seg(t, st[idx], st[idx + 1]);
    line(x0, y + 22, x0 + segW * prog, y + 22, C.lime, 3);
    STATIONS.forEach((n, i) => {
      const on = i === idx;
      label(`${String(i + 1).padStart(2, "0")} ${n}`, x0 + segW * i, y, { size: 19, color: on ? C.lime : i < idx ? C.dim : C.faint, ls: 3 });
    });
  }
  function headline(str, t, t0, o = {}) {
    const k = slam(t, t0 + 0.15, 0.3, 0.25);
    if (t < t0 + 0.15) return;
    const size = o.size ?? fit(str, F.disp, 1680, 132);
    text(str, o.x ?? 120, o.y ?? 270, { font: F.disp(size), color: o.color ?? C.cream, alpha: k.a, scale: 1, ls: 1 });
  }
  function caption(str, t, t0, y = 1000, color = C.dim) {
    if (t < t0) return;
    text(typed(str, t, t0, 0.012), 120, y, { font: F.mono(26), color, ls: 1 });
  }

  const PROMPT = "Once upon a time";
  function stData(t, u) {
    headline("IT STARTS WITH STORIES.", t, TL.stations[0]);
    const font = F.mono(84);
    const cell = 96, x0 = W / 2 - (cell * 16) / 2;
    [...PROMPT].forEach((ch, i) => {
      const flip = seg(u, 1.0 + i * 0.07, 1.18 + i * 0.07);
      const hex = ch.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0");
      const cx = x0 + cell * i + cell / 2;
      const inA = seg(u, 0.35 + i * 0.03, 0.5 + i * 0.03);
      const sy = Math.abs(Math.cos(flip * Math.PI));
      const face = flip < 0.5 ? (ch === " " ? "·" : ch) : hex;
      text(face, cx, 590, { font: flip < 0.5 ? font : F.mono(48), color: flip < 0.5 ? (ch === " " ? C.dim : C.cream) : C.amber, align: "center", base: "middle", sy: Math.max(0.02, sy), alpha: inA });
      if (flip >= 0.5) rect(cx - cell / 2 + 6, 640, cell - 12, 3, C.amber, 0.5);
    });
    // falling byte rain
    for (let i = 0; i < 70; i++) {
      const r = hash(i);
      const x = 60 + r * 1800, sp = 120 + hash(i + 9) * 220;
      const y = ((u * sp + hash(i + 3) * 1200) % 1300) - 100;
      if (y > 180 && y < 1000) text(Math.floor(hash(i + 7) * 256).toString(16).toUpperCase().padStart(2, "0"), x, y, { font: F.mono(22), color: C.amber, alpha: 0.12 * seg(u, 0.2, 0.8) });
    }
    caption("TinyStories text, read as raw bytes. Nothing pretrained: every weight starts from scratch.", t, TL.stations[0] + 1.4);
  }

  const TOKENS = D.promptTokens; // [[435,"Once"],[443," upon"],[258," a"],[402," time"]]
  function stTokens(t, u) {
    headline("BYTES BECOME TOKENS.", t, TL.stations[1]);
    const cell = 96;
    const spread = E.outBack(seg(u, 0.9, 1.5));
    const gapPx = 70 * spread;
    const totalW = cell * 16 + gapPx * 3;
    let x = W / 2 - totalW / 2;
    let ci = 0;
    TOKENS.forEach(([id, s], ti) => {
      const w = cell * s.length;
      // bracket box
      const bx = seg(u, 1.2 + ti * 0.1, 1.6 + ti * 0.1);
      if (bx > 0) {
        strokeRect(x + 4, 520, (w - 8) * E.outCubic(bx), 140, C.lime, 3, 0.9);
      }
      [...s].forEach((ch) => {
        const back = seg(u, 0.1 + ci * 0.03, 0.3 + ci * 0.03);
        const cx = x + cell * (ci - TOKENS.slice(0, ti).reduce((a, [, q]) => a + q.length, 0)) + cell / 2;
        const hex = ch.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0");
        const face = back < 0.5 ? hex : ch === " " ? "·" : ch;
        text(face, cx, 590, { font: back < 0.5 ? F.mono(48) : F.mono(84), color: back < 0.5 ? C.amber : ch === " " ? C.dim : C.cream, align: "center", base: "middle", sy: Math.max(0.02, Math.abs(Math.cos(back * Math.PI))) });
        ci++;
      });
      const idk = slam(t, TL.stations[1] + 1.8 + ti * 0.18, 0.25, 0.8);
      if (idk.a > 0) {
        text(String(id), x + w / 2, 760, { font: F.disp(96), color: C.lime, align: "center", scale: idk.s, alpha: idk.a });
      }
      x += w + gapPx;
    });
    caption("byte-level BPE · 1,024-token vocabulary · these are the real production token IDs", t, TL.stations[1] + 2.0);
  }

  function dotGrid(cx, cy, cols, rows, pitch, color, t, alpha, seed) {
    const x0 = cx - ((cols - 1) * pitch) / 2, y0 = cy - ((rows - 1) * pitch) / 2;
    for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
      const h = hash(seed + r * 97 + c);
      const pulse = 0.35 + 0.65 * (0.5 + 0.5 * Math.sin(t * 3 + h * 12));
      rect(x0 + c * pitch - 2.5, y0 + r * pitch - 2.5, 5, 5, color, alpha * pulse);
    }
  }
  function stDistill(t, u) {
    headline("A BIG TEACHER. A SMALL STUDENT.", t, TL.stations[2]);
    const ta = seg(u, 0.25, 0.6), sa = seg(u, 0.9, 1.3);
    // teacher: 512 dots (32 x 16); student: 192 dots (16 x 12)
    ctx.save(); ctx.globalAlpha = ta;
    ctx.shadowColor = "rgba(243,183,92,0.6)"; ctx.shadowBlur = 16;
    dotGrid(520, 580, 32, 16, 17, C.amber, t, 1, 3);
    ctx.restore();
    text("TEACHER", 520, 470 - 20, { font: F.disp(84), color: C.amber, align: "center", alpha: ta, ls: 4 });
    text("d512 · fp32 · MLX · trained from scratch", 520, 760, { font: F.mono(26), color: C.dim, align: "center", alpha: ta });
    ctx.save(); ctx.globalAlpha = sa;
    dotGrid(1450, 580, 16, 12, 17, C.lime, t, 1, 7);
    ctx.restore();
    text("STUDENT", 1450, 450, { font: F.disp(84), color: C.lime, align: "center", alpha: sa, ls: 4 });
    text("d192 · 6 blocks · ~1.16M params", 1450, 760, { font: F.mono(26), color: C.dim, align: "center", alpha: sa });
    // distillation stream: particles from teacher to student
    const flow = seg(u, 1.0, 1.4) * (1 - seg(u, 3.6, 3.9));
    for (let i = 0; i < 90; i++) {
      const ph = (u * 0.55 + hash(i)) % 1;
      const x = lerp(800, 1300, ph);
      const y = 580 + Math.sin(ph * Math.PI * 2 + i) * 90 * Math.sin(ph * Math.PI) + (hash(i + 5) - 0.5) * 60 * (1 - ph);
      const col = ph < 0.5 ? C.amber : C.lime;
      text(GLYPHS[i % GLYPHS.length], x, y, { font: F.mono(18 + 10 * hash(i + 2)), color: col, alpha: flow * Math.sin(ph * Math.PI) * 0.9, align: "center" });
    }
    label("distill →", 1050, 700, { color: C.cream, align: "center", alpha: flow });
    caption("The student learns the teacher's whole next-token distribution, not just the right answer.", t, TL.stations[2] + 1.6);
  }

  function stQuantize(t, u) {
    headline("PROJECTION WEIGHTS: −1, 0 OR +1.", t, TL.stations[3]);
    const cols = 14, rows = 5, cw = 112, ch = 66;
    const x0 = W / 2 - (cols * cw) / 2, y0 = 380;
    const q = TL.qatBar;
    const snapT = lerp(q.t0, q.t1, q.snapFrac);
    for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
      const i = r * cols + c;
      const base = (hash(i + 40) * 2 - 1) * 0.9;
      const v = base + Math.sin(t * 2.3 + i) * 0.08 * (1 - seg(t, snapT, snapT + 0.1));
      const tern = Math.abs(base) < 0.33 ? 0 : Math.sign(base);
      const sp = seg(t, snapT + c * 0.02, snapT + 0.12 + c * 0.02);
      const x = x0 + c * cw + cw / 2, y = y0 + r * ch + ch / 2;
      const inA = seg(u, 0.2 + i * 0.004, 0.5 + i * 0.004);
      if (sp < 1) {
        text((v >= 0 ? " " : "−") + Math.abs(v).toFixed(3), x, y, { font: F.mono(28), color: C.dim, align: "center", base: "middle", alpha: inA * (1 - sp) });
      }
      if (sp > 0) {
        const col = tern > 0 ? C.lime : tern < 0 ? C.amber : C.faint;
        if (tern !== 0) rect(x - cw / 2 + 6, y - ch / 2 + 6, cw - 12, ch - 12, col, 0.14 * sp);
        text(tern > 0 ? "+1" : tern < 0 ? "−1" : "0", x, y, { font: F.monob(40), color: col, align: "center", base: "middle", alpha: sp, scale: 1 + 0.5 * (1 - E.outExpo(sp)) });
      }
    }
    // training progress bar
    const bx = 120, bw = 1680, by = 820;
    const p = seg(t, q.t0, q.t1);
    line(bx, by, bx + bw, by, C.faint, 6);
    line(bx, by, bx + bw * p, by, t >= snapT ? C.lime : C.cream, 6);
    const mx = bx + bw * q.snapFrac;
    line(mx, by - 26, mx, by + 26, C.lime, 2, 0.8);
    label("step 9,600 · hard QAT on", mx + 14, by - 18, { color: C.lime, size: 18 });
    label(`step ${Math.round(24000 * p).toLocaleString("en-US")} / 24,000`, bx, by + 50, { size: 18, color: C.cream });
    const fl = Math.exp(-Math.max(0, t - snapT) * 8) * (t >= snapT ? 1 : 0);
    if (fl > 0.01) rect(0, 0, W, H, C.lime, 0.12 * fl);
    caption("quantization-aware training · one learned scale per row · activations trained for an Int8 grid", t, TL.stations[3] + 1.0);
  }

  // Simplified shape of the generated accumulate code (see gbf-kernel asm_impl_state.rs).
  const ADD = ["ld   a, l", "add  a, e", "ld   e, a", "adc  a, d", "sub  e", "ld   d, a"];
  const SUB = ["ld   a, e", "sub  l", "ld   e, a", "ld   a, d", "sbc  a, 0", "ld   d, a"];
  const LISTING = (() => {
    const out = [];
    for (let i = 0; i < 60; i++) {
      const w = [1, 0, -1, 1, -1, 0, 0, 1, -1, 1][i % 10] * (hash(i) > 0.15 ? 1 : -1);
      out.push(i % 2 === 0 ? ["pop  hl", 0] : null);
      if (w === 1) ADD.forEach((l, j) => out.push([l, j === 0 ? 1 : 0]));
      else if (w === -1) SUB.forEach((l, j) => out.push([l, j === 0 ? -1 : 0]));
      else out.push(["; 0 → nothing emitted", 2]);
    }
    return out.filter(Boolean);
  })();
  function stCompile(t, u) {
    headline("WEIGHTS BECOME INSTRUCTIONS.", t, TL.stations[4]);
    const rows = [["+1", "add", C.lime], ["−1", "sub", C.amber], ["0", "nothing at all", C.dim]];
    rows.forEach(([w, op, col], i) => {
      const k = slam(t, TL.stations[4] + 0.5 + i * 0.4, 0.25, 0.5);
      if (k.a <= 0) return;
      const y = 470 + i * 150;
      text(w, 300, y, { font: F.disp(130), color: col, align: "right", alpha: k.a, scale: k.s });
      text("→", 360, y - 8, { font: F.bold(70), color: C.dim, alpha: k.a });
      text(op, 460, y, { font: i === 2 ? F.ital(72) : F.monob(96), color: i === 2 ? C.dim : C.cream, alpha: k.a, scale: k.s });
    });
    // scrolling listing
    const px = 1180, py = 360, pw = 620, ph = 540;
    const la = seg(u, 0.3, 0.6);
    strokeRect(px, py, pw, ph, C.faint, 1.5, la);
    label("LR35902 · illustrative excerpt", px, py - 18, { size: 16, alpha: la });
    ctx.save();
    ctx.beginPath(); ctx.rect(px, py, pw, ph); ctx.clip();
    const lh = 36, scroll = u * 170;
    const first = Math.floor(scroll / lh);
    for (let k = 0; k < 17; k++) {
      const idx = (first + k) % LISTING.length;
      const [l, tag] = LISTING[idx];
      const y = py + 30 + k * lh - (scroll % lh);
      const col = tag === 1 ? C.lime : tag === -1 ? C.amber : tag === 2 ? C.faint : C.cream;
      text(l, px + 30, y + 10, { font: F.mono(28), color: col, alpha: la * (tag === 0 ? 0.75 : 1) });
    }
    ctx.restore();
    caption("the compiler specializes every ternary row: no general multiplies in the hot path", t, TL.stations[4] + 1.6);
  }

  function stCartridge(t, u) {
    headline("8 MiB. 408 BANKS.", t, TL.stations[5]);
    const cols = 32, rows = 16, pitch = 40, sz = 32;
    const x0 = 120, y0 = 360;
    const lit = Math.floor(408 * E.outCubic(seg(u, 0.4, 2.6)));
    for (let i = 0; i < cols * rows; i++) {
      const c = i % cols, r = Math.floor(i / cols);
      const x = x0 + c * pitch, y = y0 + r * pitch;
      if (i < lit) {
        const col = i === 0 ? C.lime : hash(i) < 0.72 ? C.amber : C.cream;
        rect(x, y, sz, sz, col, i === 0 ? 1 : 0.35 + 0.5 * hash(i + 3));
      } else strokeRect(x + 0.5, y + 0.5, sz - 1, sz - 1, C.faint, 1, seg(u, 0.1, 0.4));
    }
    const lx = x0 + cols * pitch + 60;
    const k = seg(u, 0.4, 0.7);
    text(String(lit), lx, 520, { font: F.disp(180), color: C.cream, alpha: k });
    label("of 512 × 16 KiB banks", lx, 570, { alpha: k });
    [["bank 0 · driver + shell", C.lime], ["generated weight code", C.amber], ["embeddings · head · tables", C.cream], ["tokenizer · font · UI", C.cream]].forEach(([s, col], i) => {
      const a = seg(u, 1.0 + i * 0.15, 1.3 + i * 0.15);
      rect(lx, 640 + i * 52 - 18, 20, 20, col, a * (i === 0 ? 1 : 0.7));
      label(s, lx + 36, 640 + i * 52, { size: 18, alpha: a, color: C.cream });
    });
    caption("assembled by gbf-asm into one MBC5 ROM · the whole cartridge is the network", t, TL.stations[5] + 1.4);
  }

  function stRuntime(t, u) {
    headline("A TINY RUNTIME.", t, TL.stations[6]);
    const k = seg(u, 0.35, 0.7);
    text("It alternates inference with drawing, at token boundaries.", 120, 370, { font: F.med(46), color: C.cream, alpha: k });
    // two lanes scrolling past a playhead
    const lx = 330, lw = 1470, y1 = 520, y2 = 660, period = 360, think = 330;
    label("think", 120, y1 + 40, { color: C.amber, size: 22 });
    label("draw", 120, y2 + 26, { color: C.lime, size: 22 });
    ctx.save(); ctx.beginPath(); ctx.rect(lx, 460, lw, 300); ctx.clip();
    const off = u * 260;
    for (let n = -1; n < 8; n++) {
      const x = lx + n * period - (off % period);
      const tok = n + Math.floor(off / period) + 1;
      rect(x, y1, think, 64, C.amber, 0.28 * k);
      strokeRect(x, y1, think, 64, C.amber, 1.5, 0.7 * k);
      text(`token ${tok} · ≈22 s`, x + 16, y1 + 41, { font: F.mono(22), color: C.amber, alpha: k });
      rect(x + think + 4, y2, 22, 44, C.lime, k);
    }
    ctx.restore();
    const ph = lx + 900;
    line(ph, 470, ph, 750, C.cream, 1, 0.5 * k);
    label("vblank → vram", lx + 1470, y2 + 80, { size: 16, align: "right", alpha: k });
    caption("The screen holds still while it thinks. Not a full OS: a small shell for compute, bank switching and the LCD.", t, TL.stations[6] + 1.3);
  }

  const STATION_FNS = [stData, stTokens, stDistill, stQuantize, stCompile, stCartridge, stRuntime];
  function sBuild(t) {
    crosses(0.6);
    rail(t);
    const st = TL.stations;
    for (let i = 0; i < 7; i++) {
      const a = st[i], b = st[i + 1];
      if (t < a - 0.01 || t >= b) continue;
      const inn = 1 - E.outExpo(seg(t, a, a + 0.4));
      const out = E.inExpo(seg(t, b - 0.3, b));
      ctx.save();
      ctx.translate(inn * 700 - out * 1100, 0);
      ctx.globalAlpha = (1 - inn * 0.9) * (1 - out);
      STATION_FNS[i](t, t - a);
      ctx.restore();
    }
  }

  // S5 the loop on the cartridge, then the 22-second gag.
  const LOOP = [
    ["TOKENIZE", "your prompt, byte-BPE, on the cartridge"],
    ["REMEMBER", "192 recurrent state slots · no KV cache"],
    ["THINK", "six residual blocks"],
    ["CHOOSE", "1,024-way head · top-k 4 · T = 0.6"],
    ["WRITE", "paint the token's bytes to the LCD"],
    ["FEED BACK", "the new token becomes the next input"],
    ["REPEAT", "24 times"],
  ];
  function sLoop(t) {
    crosses(0.5);
    const t0 = 46.0;
    if (t < 49.5) {
      const i = clamp(Math.floor((t - t0) / 0.5), 0, 6);
      const k = slam(t, t0 + i * 0.5, 0.2, 0.7);
      const [w, sub] = LOOP[i];
      text(w, W / 2, 600, { font: F.disp(fit(w, F.disp, 1600, 330)), color: i % 2 ? C.cream : C.lime, align: "center", scale: k.s, alpha: k.a });
      text(sub, W / 2, 720, { font: F.mono(38), color: C.amber, align: "center", alpha: k.a });
      // step dots
      LOOP.forEach((_, j) => rect(W / 2 - 7 * 22 + j * 44 + 8, 860, 28, 6, j <= i ? C.lime : C.faint));
      label(`step ${i + 1} / 7 · every token`, W / 2, 910, { align: "center", size: 18 });
    } else {
      // tape stop: the headline droops as time slows
      const sag = E.inCubic(seg(t, TL.tapeStop, TL.tapeStop + 0.7));
      const k = slam(t, 49.5, 0.25, 0.6);
      text("ALL ON THE CARTRIDGE.", W / 2, 600 + sag * 520, { font: F.disp(170), color: C.lime, align: "center", scale: k.s * (1 - sag * 0.3), alpha: k.a * (1 - sag), rot: sag * 0.12 });
      if (t >= 50.6) {
        const u = t - 50.6;
        const a = seg(u, 0, 0.3) * (1 - seg(t, 53.75, 54.0));
        label("one token takes about", W / 2, 280, { align: "center", alpha: a, color: C.cream, size: 26 });
        text("22 SECONDS", W / 2, 470, { font: F.disp(220), color: C.cream, align: "center", alpha: a });
        // real-time progress toward 22 s
        const bx = 460, bw = 1000, by = 560;
        rect(bx, by, bw, 10, C.faint, a);
        rect(bx, by, bw * Math.min(1, u / 22), 10, C.amber, a);
        label(`${u.toFixed(1)} s of 22 · real time`, bx, by + 50, { alpha: a, size: 20 });
        if (t >= 51.4) text("Give it a second.", W / 2, 760, { font: F.bold(76), color: C.cream, align: "center", alpha: seg(t, 51.4, 51.55) * a, ls: -1 });
        if (t >= 52.5) text("…or twenty-two.", W / 2, 870, { font: F.bold(76), color: C.lime, align: "center", alpha: seg(t, 52.5, 52.65) * a, ls: -1 });
        if (t >= 53.5) label(blink(t, 4) ? "▶▶ time-lapse" : "", 1800, 1010, { align: "right", color: C.lime, size: 24 });
      }
    }
  }

  // S6 proof: the authentic framebuffer + the real token stream.
  const GEN = D.generatedTokens;
  const MCYC_HZ = 4194304 / 4;
  const cum = (() => { let s = 0; return D.generationBoundaryMCycles.map((m) => (s += m) / MCYC_HZ); })();
  const totalSec = cum[cum.length - 1];
  const lapseDur = TL.tokens.dt * GEN.length;
  const speedup = Math.round(totalSec / lapseDur);
  function fmtClock(s) { const m = Math.floor(s / 60); return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`; }

  function sProof(t) {
    // exact pixels: integer scale, no smoothing, no alpha, no overlays
    const S = 6, fx = 110, fy = 96, fw = 160 * S, fh = 144 * S;
    const rv = E.outCubic(seg(t, TL.frameReveal, TL.frameReveal + 0.5));
    label("▣ real Game Boy output", fx, fy - 26, { color: C.lime, size: 20, alpha: seg(t, 54.0, 54.2) });
    strokeRect(fx - 7, fy - 7, fw + 14, fh + 14, C.dim, 2, seg(t, 54.0, 54.3));
    if (rv > 0) {
      ctx.save();
      ctx.imageSmoothingEnabled = false;
      ctx.globalAlpha = 1; ctx.filter = "none"; ctx.shadowBlur = 0;
      const h = Math.round((rv * fh) / S) * S;
      ctx.drawImage(fb, 0, 0, 160, h / S, fx, fy, fw, h);
      ctx.restore();
      if (rv < 1) rect(fx, fy + h - 2, fw, 4, C.lime, 0.9);
    }
    text("framebuffer from the accepted ROM run · 160×144 · shown 6× nearest-neighbor · unretouched", fx, fy + fh + 44, { font: F.mono(19), color: C.dim, alpha: seg(t, 54.6, 55.0) });

    // right panel
    const px = 1180, pw = 640;
    const pa = seg(t, 54.5, 54.9);
    label("prompt · typed on the joypad", px, 150, { alpha: pa, size: 18 });
    text(PROMPT, px, 212, { font: F.bold(54), color: C.cream, alpha: pa, ls: -1 });
    label(TOKENS.map(([id]) => id).join("  "), px, 252, { alpha: pa, size: 18, color: C.lime, ls: 2 });
    label("output · generated on the cartridge", px, 330, { alpha: pa, size: 18 });
    const shown = clamp(Math.floor((t - TL.tokens.t0) / TL.tokens.dt) + 1, 0, GEN.length);
    const font = F.med(42);
    let x = px, y = 395;
    const lh = 58;
    for (let i = 0; i < shown; i++) {
      const [id, s] = GEN[i];
      if (s === "\n") { x = px; y += lh; continue; }
      const w = measure(s, font);
      if (x + w > px + pw && s.startsWith(" ")) { x = px; y += lh; }
      const age = t - (TL.tokens.t0 + i * TL.tokens.dt);
      const hot = 1 - seg(age, 0, 0.5);
      const draw = x === px && s.startsWith(" ") ? s.slice(1) : s;
      const dw = measure(draw, font);
      if (hot > 0) rect(x - 2, y - 40, dw + 6, 52, C.lime, 0.85 * hot);
      text(draw, x, y, { font, color: hot > 0.5 ? C.bg : C.cream });
      if (i === shown - 1 && age < 0.25) label(String(id), x, y - 50, { color: C.lime, size: 16, ls: 1 });
      x += dw;
    }
    if (shown > 0 && shown < GEN.length && blink(t, 3)) rect(x + 4, y - 38, 20, 46, C.lime);
    // clock
    const ca = seg(t, 55.2, 55.5);
    const gbTime = shown > 0 ? cum[Math.min(shown, cum.length) - 1] : 0;
    label("game boy time (generation)", px, 810, { alpha: ca, size: 18 });
    text(fmtClock(gbTime), px, 900, { font: F.monob(84), color: C.cream, alpha: ca });
    label(`${GEN.length} tokens · ≈22 s each · time-lapse ≈${speedup}×`, px, 945, { alpha: ca, size: 18, color: C.amber });
    // honesty beat
    [["SMALL.", 62.2, C.cream], ["IMPERFECT.", 62.7, C.cream], ["REAL.", 63.2, C.lime]].forEach(([w, t0, col], i) => {
      if (t < t0) return;
      const k = slam(t, t0, 0.22, 0.5);
      const f = F.disp(66);
      const x = px + ["SMALL. ", "IMPERFECT. "].slice(0, i).reduce((acc, q) => acc + measure(q, f), 0);
      text(w, x, 1030, { font: f, color: col, alpha: k.a, scale: k.s });
    });
  }

  // S7 recap + coda + wordmark.
  const RECAP = ["TRAINED.", "DISTILLED.", "QUANTIZED.", "COMPILED.", "CARTRIDGED."];
  function sCoda(t) {
    if (t < 68.5) {
      crosses(0.5);
      const i = clamp(Math.floor((t - 66.0) / 0.5), 0, 4);
      const k = slam(t, 66.0 + i * 0.5, 0.22, 0.7);
      const w = RECAP[i];
      text(w, W / 2, 640, { font: F.disp(fit(w, F.disp, 1700, 320)), color: i === 4 ? C.lime : C.cream, align: "center", scale: k.s, alpha: k.a * (1 - seg(t, 68.3, 68.5)) });
      RECAP.forEach((r, j) => label(r, 120 + j * 360, 1000, { color: j <= i ? C.lime : C.faint, size: 18 }));
      return;
    }
    constellation(t, 0.45 * (1 - seg(t, 73.0, 74.0)));
    if (t < 71.0) {
      const o = TL.typeCoda;
      const s = typed(o.text, t, o.t0, o.dt);
      const out = E.inExpo(seg(t, 70.75, 71.0));
      ctx.save(); ctx.globalAlpha = 1 - out;
      text(s, W / 2, 470, { font: F.disp(150), color: C.cream, align: "center", ls: 1 });
      if (t >= 69.5) {
        const a = "there was a little ", b = "Game Boy.";
        const font = F.disp(150);
        const wa = measure(a, font, 1), wb = measure(b, font, 1);
        const x0 = W / 2 - (wa + wb) / 2;
        const k1 = slam(t, 69.5, 0.25, 0.3), k2 = slam(t, 70.0, 0.3, 0.8);
        text(a, x0, 660, { font, color: C.cream, alpha: k1.a, ls: 1 });
        if (t >= 70.0) text(b, x0 + wa + wb / 2, 660, { font, color: C.lime, align: "center", scale: k2.s, alpha: k2.a, ls: 1, glow: "rgba(189,227,63,0.4)", glowBlur: 40 });
      }
      ctx.restore();
      return;
    }
    const fade = 1 - seg(t, 73.1, 73.9);
    ctx.save(); ctx.globalAlpha = fade;
    const k = slam(t, 71.0, 0.35, 0.4);
    const font = F.disp(300);
    const w = measure("gbllm", font, 4);
    text("gbllm", W / 2 - 30, 600, { font, color: C.cream, align: "center", scale: k.s, alpha: k.a, ls: 4 });
    if (blink(t, 1.6) && t > 71.3) rect(W / 2 - 30 + w / 2 + 20, 400, 70, 200, C.lime);
    text("a language model that runs entirely on an original Game Boy cartridge", W / 2, 720, { font: F.mono(34), color: C.cream, align: "center", alpha: seg(t, 71.4, 71.8) });
    label("d192 · six blocks · 1,024 tokens · ternary weights · 8 MiB MBC5 · LR35902", W / 2, 790, { align: "center", alpha: seg(t, 71.8, 72.2), size: 18 });
    ctx.restore();
  }

  const SCENES = [
    [0, 4, sOpen], [4, 12, sHook], [12, 20, sMachine], [20, 46, sBuild],
    [46, 54, sLoop], [54, 66, sProof], [66, 74.1, sCoda],
  ];

  function draw(t) {
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalAlpha = 1;
    ctx.fillStyle = C.bg;
    ctx.fillRect(0, 0, W, H);
    // grain + vignette sit *under* the content so the proof frame stays exact
    ctx.fillStyle = grain[Math.floor(t * 24) % 4];
    ctx.fillRect(0, 0, W, H);
    ctx.drawImage(vignette, 0, 0);
    const proof = t >= 54 && t < 66;
    const [sx, sy] = proof ? [0, 0] : shakeAt(t);
    ctx.translate(sx, sy);
    for (const [a, b, fn] of SCENES) if (t >= a && t < b) fn(t);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    const fl = proof ? 0 : flashAt(t);
    if (fl > 0) rect(0, 0, W, H, C.cream, fl);
    ctx.restore();
  }

  const ready = Promise.all([
    document.fonts.load("100px GBDisplay"),
    document.fonts.load("20px GBMono"),
    document.fonts.load("700 20px GBMono"),
    document.fonts.load('700 20px "Helvetica Neue"'),
    document.fonts.load('500 20px "Helvetica Neue"'),
    document.fonts.load('italic 300 20px "Helvetica Neue"'),
  ]).then(() => {
    const ok = ["100px GBDisplay", "20px GBMono"].map((f) => document.fonts.check(f));
    console.log("fonts", JSON.stringify(ok), "gbDisplay width", measure("GAME BOY.", F.disp(100)).toFixed(1), "helv width", measure("GAME BOY.", F.bold(100)).toFixed(1));
  });

  return { draw, ready };
}
