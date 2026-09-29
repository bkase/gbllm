// Shared cue sheet for picture (film.js) and sound (score.mjs).
// 120 BPM: one beat = 0.5 s, one bar = 2 s. Times are seconds.
(function () {
  const typing = (t0, text, dt) => [...text].map((ch, i) => ({ t: t0 + i * dt, ch }));

  const T = {
    bpm: 120,
    duration: 74,
    // Keystrokes that print a character (score: square-wave blips).
    typeOpen: { t0: 1.0, dt: 0.09, text: "Once upon a time" },
    typeCoda: { t0: 68.5, dt: 0.045, text: "Once upon a time," },
    // Heavy word slams (score: kick + noise burst; picture: shake).
    slams: [4.0, 4.5, 5.0, 6.0, 6.5, 7.0, 9.0, 9.5, 10.0, 10.5, 12.0, 13.5, 15.0, 16.5,
      46.0, 46.5, 47.0, 47.5, 48.0, 48.5, 49.0, 49.5,
      66.0, 66.5, 67.0, 67.5, 68.0],
    // Station changes in the build section (score: riser/whoosh).
    whooshes: [3.6, 11.6, 19.7, 23.4, 26.9, 30.9, 34.9, 38.9, 42.4, 45.8, 53.6, 65.8, 70.9],
    // Station start times for the build pipeline.
    stations: [20.0, 23.5, 27.0, 31.0, 35.0, 39.0, 42.5, 46.0],
    // Quantization snap moment (hard QAT at step 9,600 of 24,000).
    qatBar: { t0: 31.4, t1: 34.0, snapFrac: 9600 / 24000 },
    // Tape-stop gag: generation is ~22 s per token.
    tapeStop: 50.0,
    // Time-lapse token reveal in the proof section.
    tokens: { t0: 55.5, dt: 0.25 },
    frameReveal: 54.3,
    // Film-wide flash frames. Never flash while the authentic framebuffer is up.
    flashes: [4.0, 7.0, 12.0, 20.0, 46.0, 66.0, 71.0],
  };
  T.typing = [...typing(T.typeOpen.t0, T.typeOpen.text, T.typeOpen.dt), ...typing(T.typeCoda.t0, T.typeCoda.text, T.typeCoda.dt)];
  globalThis.TIMELINE = T;
})();
