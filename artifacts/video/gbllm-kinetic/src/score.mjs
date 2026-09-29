// Original score + sound design, synthesized from scratch (no samples).
// Voices echo the DMG APU: pulse waves, a triangle-ish wave channel, and
// 15-bit LFSR noise, plus a modern sub kick and a small reverb for size.
//
//   node score.mjs   ->  ../out/score.wav  (48 kHz, 16-bit stereo)

import { writeFileSync, mkdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import vm from "node:vm";

const here = path.dirname(fileURLToPath(import.meta.url));
const ctx = { globalThis: {} };
ctx.globalThis = ctx;
vm.runInNewContext(readFileSync(path.join(here, "timeline.js"), "utf8"), ctx);
vm.runInNewContext(readFileSync(path.join(here, "data.js"), "utf8").replace("window.", "globalThis."), ctx);
const TL = ctx.TIMELINE;
const DATA = ctx.FILM_DATA;

const SR = 48000;
const N = Math.ceil(TL.duration * SR);
const bus = () => [new Float32Array(N), new Float32Array(N)];
const music = bus(), sfx = bus(), verbSend = bus();

const mtof = (m) => 440 * Math.pow(2, (m - 69) / 12);
const beat = 60 / TL.bpm;
function rng(seed) { return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296); }

// ------------------------------------------------------------------ voices
function osc(kind, phase, duty = 0.5) {
  const p = phase - Math.floor(phase);
  switch (kind) {
    case "pulse": return p < duty ? 0.8 : -0.8;
    case "tri": return 1 - 4 * Math.abs(p - 0.5);
    case "wave": { const v = 1 - 4 * Math.abs(p - 0.5); return Math.round(v * 7.5) / 7.5; } // 4-bit wave channel
    case "sine": return Math.sin(2 * Math.PI * p);
    case "saw": return 2 * p - 1;
  }
}
// ADSR-ish envelope: attack a, decay d to sustain s, release r after dur.
function env(t, dur, a, d, s, r) {
  if (t < 0) return 0;
  if (t < a) return t / a;
  if (t < a + d) return 1 - (1 - s) * ((t - a) / d);
  if (t < dur) return s;
  if (t < dur + r) return s * (1 - (t - dur) / r);
  return 0;
}
function note(target, t0, dur, freq, o = {}) {
  const { kind = "pulse", duty = 0.5, amp = 0.2, a = 0.004, d = 0.08, s = 0.6, r = 0.08, pan = 0, send = 0, glide = null, vib = 0 } = o;
  const i0 = Math.max(0, Math.floor(t0 * SR)), i1 = Math.min(N, Math.ceil((t0 + dur + r) * SR));
  const gl = Math.cos(((pan + 1) * Math.PI) / 4), gr = Math.sin(((pan + 1) * Math.PI) / 4);
  let ph = 0;
  for (let i = i0; i < i1; i++) {
    const t = i / SR - t0;
    const f = (glide ? glide(t) : freq) * (1 + vib * Math.sin(2 * Math.PI * 5.5 * t));
    ph += f / SR;
    const v = osc(kind, ph, duty) * amp * env(t, dur, a, d, s, r);
    target[0][i] += v * gl; target[1][i] += v * gr;
    if (send) { verbSend[0][i] += v * gl * send; verbSend[1][i] += v * gr * send; }
  }
}
// 15-bit LFSR noise (short mode = metallic), like the DMG noise channel.
function lfsrNoise(target, t0, dur, o = {}) {
  const { amp = 0.2, rate = 22000, short = false, a = 0.001, d = 0.05, s = 0, r = 0.02, pan = 0, send = 0, hp = 0 } = o;
  const i0 = Math.max(0, Math.floor(t0 * SR)), i1 = Math.min(N, Math.ceil((t0 + dur + r) * SR));
  let lfsr = 0x7fff, acc = 0, out = 1, prev = 0, hpState = 0;
  const gl = Math.cos(((pan + 1) * Math.PI) / 4), gr = Math.sin(((pan + 1) * Math.PI) / 4);
  for (let i = i0; i < i1; i++) {
    acc += rate / SR;
    while (acc >= 1) {
      acc -= 1;
      const bit = (lfsr ^ (lfsr >> 1)) & 1;
      lfsr = (lfsr >> 1) | (bit << 14);
      if (short) lfsr = (lfsr & ~0x40) | (bit << 6);
      out = lfsr & 1 ? -1 : 1;
    }
    let v = out;
    if (hp) { hpState = hp * (hpState + v - prev); prev = v; v = hpState; }
    const t = i / SR - t0;
    v *= amp * env(t, dur, a, d, s, r);
    target[0][i] += v * gl; target[1][i] += v * gr;
    if (send) { verbSend[0][i] += v * gl * send; verbSend[1][i] += v * gr * send; }
  }
}
function kick(target, t0, amp = 0.9, len = 0.32) {
  note(target, t0, len, 0, { kind: "sine", amp, a: 0.001, d: len, s: 0, r: 0.02, glide: (t) => 42 + 130 * Math.exp(-t * 28) });
  lfsrNoise(target, t0, 0.01, { amp: amp * 0.25, rate: 30000, d: 0.012, hp: 0.6 });
}
function snare(target, t0, amp = 0.3) {
  note(target, t0, 0.08, 0, { kind: "tri", amp: amp * 0.8, d: 0.08, s: 0, glide: (t) => 190 + 90 * Math.exp(-t * 40) });
  lfsrNoise(target, t0, 0.16, { amp, rate: 18000, d: 0.16, s: 0, send: 0.25, hp: 0.9 });
}
function hat(target, t0, amp = 0.07, open = false, pan = 0.25) {
  lfsrNoise(target, t0, open ? 0.12 : 0.03, { amp, rate: 40000, short: true, d: open ? 0.12 : 0.03, hp: 0.97, pan });
}
// Band-passed noise sweep (Chamberlin SVF) for transitions.
function whoosh(target, t0, dur = 0.42, f0 = 250, f1 = 7000, amp = 0.35) {
  const i0 = Math.floor(t0 * SR), i1 = Math.min(N, Math.ceil((t0 + dur) * SR));
  let low = 0, band = 0; const r = rng(Math.floor(t0 * 1000));
  for (let i = i0; i < i1; i++) {
    const p = (i - i0) / (i1 - i0);
    const fc = f0 * Math.pow(f1 / f0, p * p);
    const f = 2 * Math.sin((Math.PI * fc) / SR);
    const x = r() * 2 - 1;
    low += f * band; const high = x - low - 0.6 * band; band += f * high;
    const e = Math.pow(p, 1.6) * (p > 0.92 ? (1 - p) / 0.08 : 1);
    const v = band * amp * e;
    const pan = Math.sin(p * Math.PI * 2) * 0.5;
    target[0][i] += v * (1 - pan) ; target[1][i] += v * (1 + pan);
    verbSend[0][i] += v * 0.3; verbSend[1][i] += v * 0.3;
  }
}

// ------------------------------------------------------------------ harmony
// A minor: Am - F - C - G, one chord per bar (2 s).
const CHORDS = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]];
const chordAt = (t) => CHORDS[Math.floor(t / (4 * beat)) % 4];
const PENTA = [0, 3, 5, 7, 10];
const inAny = (t, ranges) => ranges.some(([a, b]) => t >= a && t < b);

// Pad: soft detuned wave-channel chords, bar by bar.
for (let bar = 0; bar < TL.duration / 2; bar++) {
  const t0 = bar * 2;
  if (inAny(t0, [[50, 54]])) continue;
  const ch = chordAt(t0);
  const amp = t0 < 4 ? 0.03 + 0.012 * bar : t0 >= 54 && t0 < 66 ? 0.05 : t0 >= 68 ? 0.05 : 0.035;
  for (const m of ch) for (const det of [-0.08, 0.08]) {
    note(music, t0, 2, mtof(m + det), { kind: "wave", amp, a: 0.25, d: 0.3, s: 0.8, r: 0.5, pan: det * 5, send: 0.6 });
  }
}

// Drums, bass, arpeggio, lead.
const GROOVE = [[4, 12, 1], [12, 20, 2], [20, 46, 3], [46, 50.4, 3], [54, 66, 4], [66, 68.5, 3]];
for (let k = 0; k < TL.duration / (beat / 4); k++) {
  const t = k * (beat / 4);
  const g = GROOVE.find(([a, b]) => t >= a && t < b);
  if (!g) continue;
  const lvl = g[2];
  const step = k % 16; // 16ths in a bar
  const ch = chordAt(t);
  // kick
  if (lvl >= 2 && lvl !== 4 && step % 4 === 0) kick(music, t, 0.75);
  if (lvl === 4 && (step === 0 || step === 10)) kick(music, t, 0.5);
  // snare on 2 and 4
  if (lvl >= 2 && lvl !== 4 && (step === 4 || step === 12)) snare(music, t, lvl === 3 ? 0.32 : 0.26);
  // hats
  if (lvl === 2 && step % 2 === 0) hat(music, t, step % 4 === 2 ? 0.09 : 0.05);
  if (lvl === 3) hat(music, t, step % 4 === 2 ? 0.1 : 0.045, step === 14, step % 2 ? -0.3 : 0.3);
  if (lvl === 4 && step % 4 === 2) hat(music, t, 0.04);
  // bass: 8ths on the root, octave pops
  if (lvl <= 3 && step % 2 === 0) {
    const root = ch[0] - 24;
    const oct = [0, 0, 12, 0, 0, 12, 0, 12][step / 2] ?? 0;
    note(music, t, beat / 2 - 0.03, mtof(root + oct), { kind: "pulse", duty: 0.25, amp: lvl === 1 ? 0.1 : 0.13, d: 0.12, s: 0.5, r: 0.03 });
  }
  if (lvl === 4 && step === 0) note(music, t, 1.8, mtof(ch[0] - 24), { kind: "tri", amp: 0.2, a: 0.02, d: 0.6, s: 0.6, r: 0.2 });
  // arpeggio: 16ths up through chord tones
  if (lvl >= 2 && lvl !== 4) {
    const tones = [ch[0], ch[1], ch[2], ch[0] + 12];
    note(music, t, beat / 4 - 0.02, mtof(tones[step % 4] + 12), { kind: "pulse", duty: 0.125, amp: lvl === 3 ? 0.045 : 0.035, d: 0.05, s: 0.3, r: 0.02, pan: -0.35, send: 0.2 });
  }
  if (lvl === 4 && step % 2 === 0) {
    const tones = [ch[0], ch[2], ch[1] + 12, ch[2]];
    note(music, t, beat / 2, mtof(tones[(step / 2) % 4] + 12), { kind: "tri", amp: 0.035, d: 0.2, s: 0.3, r: 0.15, pan: 0.3, send: 0.5 });
  }
}
// Lead motif in the build section (bars 10..22), echoing "Once upon a time".
const MOTIF = [[0, 0.5, 12], [0.5, 0.5, 15], [1, 1, 19], [2, 0.5, 17], [2.5, 0.5, 15], [3, 1, 12],
  [4, 0.5, 12], [4.5, 0.5, 15], [5, 0.75, 19], [5.75, 0.25, 22], [6, 1.5, 24]];
for (let t0 = 20; t0 < 46; t0 += 8) {
  for (const [b, len, iv] of MOTIF) {
    const t = t0 + b * beat;
    if (t >= 46) continue;
    note(music, t, len * beat - 0.04, mtof(57 + iv - 12), { kind: "pulse", duty: 0.25, amp: 0.06, d: 0.1, s: 0.55, r: 0.06, pan: 0.2, send: 0.35, vib: 0.004 });
  }
}

// ------------------------------------------------------------------ cues
// Typing blips: pentatonic, pitch from the character code.
for (const { t, ch } of TL.typing) {
  const c = ch.charCodeAt(0);
  const m = 81 + PENTA[c % 5] + (c % 2) * 12;
  note(sfx, t, 0.03, mtof(m), { kind: "pulse", duty: 0.5, amp: ch === " " ? 0.03 : 0.07, d: 0.03, s: 0, r: 0.01, pan: (c % 7) / 7 - 0.5, send: 0.2 });
}
// Slams: sub hit + noise + chord stab. Loop/recap slams climb.
const climb = [46.0, 46.5, 47.0, 47.5, 48.0, 48.5, 49.0, 66.0, 66.5, 67.0, 67.5, 68.0];
for (const t of TL.slams) {
  kick(sfx, t, 0.85, 0.38);
  lfsrNoise(sfx, t, 0.14, { amp: 0.16, rate: 16000, d: 0.14, send: 0.4, hp: 0.8 });
  const ch = chordAt(t);
  const lift = climb.includes(t) ? [0, 2, 3, 5, 7, 8, 10, 0, 3, 5, 7, 12][climb.indexOf(t)] : 0;
  for (const m of ch) note(sfx, t, 0.16, mtof(m + 12 + lift), { kind: "pulse", duty: 0.25, amp: 0.045, d: 0.14, s: 0.2, r: 0.12, send: 0.5 });
}
for (const t of TL.whooshes) whoosh(sfx, t, 0.42);

// Hard QAT snap: a smooth rising tone that becomes ternary {-1, 0, +1}.
{
  const q = TL.qatBar;
  const snap = q.t0 + (q.t1 - q.t0) * q.snapFrac;
  const i0 = Math.floor(q.t0 * SR), i1 = Math.floor((snap + 0.9) * SR);
  let ph = 0;
  for (let i = i0; i < i1; i++) {
    const t = i / SR;
    const f = t < snap ? 110 * Math.pow(2, (t - q.t0) / (snap - q.t0)) : 220;
    ph += f / SR;
    let v = Math.sin(2 * Math.PI * ph) + 0.5 * Math.sin(4 * Math.PI * ph * 1.5);
    if (t >= snap) v = Math.abs(v) < 0.5 ? 0 : Math.sign(v); // ternary
    const e = t < snap ? 0.05 + 0.1 * ((t - q.t0) / (snap - q.t0)) : 0.16 * Math.exp(-(t - snap) * 4);
    sfx[0][i] += v * e; sfx[1][i] += v * e;
    verbSend[0][i] += v * e * 0.3; verbSend[1][i] += v * e * 0.3;
  }
  kick(sfx, snap, 0.7);
}
// The 22-second gag: clock ticks in real seconds, then a fast-forward chirp.
for (let k = 0; k < 4; k++) {
  const t = 50.6 + k;
  lfsrNoise(sfx, t, 0.012, { amp: 0.22, rate: 44000, short: true, d: 0.012, hp: 0.95, pan: k % 2 ? 0.4 : -0.4 });
  note(sfx, t, 0.02, 2400, { kind: "sine", amp: 0.08, d: 0.02, s: 0, r: 0.01 });
}
note(sfx, 50.6, 3.2, mtof(33), { kind: "tri", amp: 0.12, a: 0.6, d: 0.5, s: 0.7, r: 0.3 });
note(sfx, 53.45, 0.5, 0, { kind: "pulse", duty: 0.125, amp: 0.07, a: 0.01, d: 0.5, s: 1, r: 0.05, glide: (t) => 200 * Math.pow(12, t / 0.5) * (1 + 0.08 * Math.sign(Math.sin(t * 2 * Math.PI * 30))) });
// Proof: every generated token plays one note, pitched from its real token ID.
DATA.generatedTokens.forEach(([id], i) => {
  const t = TL.tokens.t0 + i * TL.tokens.dt;
  const m = 69 + PENTA[id % 5] + 12 * (Math.floor(id / 5) % 2);
  note(sfx, t, 0.09, mtof(m), { kind: "pulse", duty: 0.25, amp: 0.06, d: 0.09, s: 0.2, r: 0.2, pan: (i % 2 ? 0.3 : -0.3), send: 0.7 });
  note(sfx, t, 0.09, mtof(m - 12), { kind: "tri", amp: 0.05, d: 0.09, s: 0.2, r: 0.2, send: 0.3 });
});
// Frame reveal: an LCD-ish power-on sweep.
note(sfx, TL.frameReveal, 0.5, 0, { kind: "pulse", duty: 0.5, amp: 0.05, d: 0.5, s: 0, r: 0.05, glide: (t) => 400 + 1400 * t, send: 0.4 });
// "Game Boy." hit and the final chime (an original two-note figure, a fourth apart).
kick(sfx, 70.0, 0.8, 0.4);
for (const m of [69, 76, 81]) note(sfx, 70.0, 0.4, mtof(m), { kind: "pulse", duty: 0.25, amp: 0.05, d: 0.3, s: 0.3, r: 0.4, send: 0.6 });
note(sfx, 71.0, 0.07, mtof(88), { kind: "pulse", duty: 0.5, amp: 0.09, d: 0.07, s: 1, r: 0.01, send: 0.5 });
note(sfx, 71.08, 1.2, mtof(93), { kind: "pulse", duty: 0.5, amp: 0.09, d: 1.2, s: 0, r: 0.3, send: 0.8 });
for (const m of [45, 57, 60, 64, 71]) note(sfx, 71.0, 1.8, mtof(m), { kind: "wave", amp: 0.05, a: 0.02, d: 1.8, s: 0.2, r: 1.0, send: 0.8 });
kick(sfx, 71.0, 0.9, 0.5);

// ------------------------------------------------------------------ tape stop
// Music decelerates to a halt at the "22 seconds" beat, then silence.
{
  const t0 = TL.tapeStop, len = 0.7;
  const src = music.map((c) => c.slice());
  for (let i = Math.floor(t0 * SR); i < Math.floor(54 * SR); i++) {
    const u = i / SR - t0;
    for (let c = 0; c < 2; c++) {
      if (u >= len) { music[c][i] = 0; continue; }
      const pos = (t0 + u - (u * u) / (2 * len)) * SR;
      const j = Math.floor(pos), fr = pos - j;
      music[c][i] = (src[c][j] * (1 - fr) + src[c][j + 1] * fr) * (1 - u / len);
    }
  }
}

// ------------------------------------------------------------------ reverb
function reverb([inL, inR]) {
  const combs = [1557, 1617, 1491, 1422, 1277, 1356].map((d) => ({ d, buf: new Float32Array(d), i: 0, lp: 0 }));
  const aps = [556, 441, 341].map((d) => ({ d, buf: new Float32Array(d), i: 0 }));
  const out = bus();
  for (let c = 0; c < 2; c++) {
    const inp = c ? inR : inL;
    const cs = combs.map((k) => ({ ...k, d: k.d + c * 23, buf: new Float32Array(k.d + c * 23), i: 0, lp: 0 }));
    const as = aps.map((k) => ({ ...k, buf: new Float32Array(k.d), i: 0 }));
    for (let n = 0; n < N; n++) {
      let s = 0;
      for (const k of cs) {
        const y = k.buf[k.i];
        k.lp = y * 0.7 + k.lp * 0.3;
        k.buf[k.i] = inp[n] + k.lp * 0.83;
        k.i = (k.i + 1) % k.d;
        s += y;
      }
      s *= 0.12;
      for (const a of as) {
        const b = a.buf[a.i];
        const y = -s + b;
        a.buf[a.i] = s + b * 0.5;
        a.i = (a.i + 1) % a.d;
        s = y;
      }
      out[c][n] = s;
    }
  }
  return out;
}
const wet = reverb(verbSend);

// ------------------------------------------------------------------ master
const L = new Float32Array(N), R = new Float32Array(N);
let lpL = 0, lpR = 0;
for (let i = 0; i < N; i++) {
  let l = music[0][i] * 0.9 + sfx[0][i] + wet[0][i] * 0.9;
  let r = music[1][i] * 0.9 + sfx[1][i] + wet[1][i] * 0.9;
  lpL += 0.55 * (l - lpL); lpR += 0.55 * (r - lpR); // tame naive-pulse fizz
  const t = i / SR;
  const fade = Math.min(1, t / 0.05) * Math.min(1, (TL.duration - t) / 0.6);
  L[i] = Math.tanh(lpL * 1.1) * fade;
  R[i] = Math.tanh(lpR * 1.1) * fade;
}
let peak = 0;
for (let i = 0; i < N; i++) peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
const gain = 0.89 / peak;
const pcm = Buffer.alloc(44 + N * 4);
pcm.write("RIFF", 0); pcm.writeUInt32LE(36 + N * 4, 4); pcm.write("WAVE", 8);
pcm.write("fmt ", 12); pcm.writeUInt32LE(16, 16); pcm.writeUInt16LE(1, 20); pcm.writeUInt16LE(2, 22);
pcm.writeUInt32LE(SR, 24); pcm.writeUInt32LE(SR * 4, 28); pcm.writeUInt16LE(4, 32); pcm.writeUInt16LE(16, 34);
pcm.write("data", 36); pcm.writeUInt32LE(N * 4, 40);
for (let i = 0; i < N; i++) {
  pcm.writeInt16LE(Math.round(L[i] * gain * 32767), 44 + i * 4);
  pcm.writeInt16LE(Math.round(R[i] * gain * 32767), 46 + i * 4);
}
mkdirSync(path.join(here, "../out"), { recursive: true });
const out = path.join(here, "../out/score.wav");
writeFileSync(out, pcm);
console.log(out, `peak ${peak.toFixed(3)} gain ${gain.toFixed(3)}`);
