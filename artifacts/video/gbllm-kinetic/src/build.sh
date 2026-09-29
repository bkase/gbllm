#!/usr/bin/env bash
# Rebuild the film end to end: evidence data -> score -> frames -> final MP4.
# Needs: python3, node >= 22, ffmpeg with libx264, Google Chrome.
set -euo pipefail
cd "$(dirname "$0")"
OUT=../out
FINAL=../gbllm-kinetic.mp4
SHARE=../gbllm-kinetic-share.mp4

if [[ -n "${GBLLM_TOKENIZER:-}" || -f ../../../../training/artifacts/tinystories_bpe_1024.json ]]; then
  python3 make_data.py
elif [[ ! -f data.js ]]; then
  echo "Missing pinned data.js. Set GBLLM_TOKENIZER to regenerate it." >&2
  exit 1
fi
node score.mjs
node render.mjs --fps "${FPS:-60}" --out "$OUT/frames.mp4"
ffmpeg -y -loglevel error -i "$OUT/frames.mp4" -i "$OUT/score.wav" \
  -map 0:v -map 1:a -c:v libx264 -preset slow -crf 19 -maxrate 14M -bufsize 28M \
  -profile:v high -level 4.2 -pix_fmt yuv420p -g 120 \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 -c:a aac -b:a 256k -ar 48000 \
  -movflags +faststart -shortest \
  -metadata title="gbllm — a language model on a Game Boy cartridge" \
  "$FINAL"
ffmpeg -y -loglevel error -i "$FINAL" \
  -c:v libx264 -preset medium -crf 22 -maxrate 8M -bufsize 16M -pix_fmt yuv420p \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 -c:a copy \
  -movflags +faststart "$SHARE"
ffprobe -v error -show_entries format=duration,size,bit_rate:stream=codec_name,profile,width,height,r_frame_rate,pix_fmt,sample_rate,channels -of default=nw=1 "$FINAL"
ffprobe -v error -show_entries format=duration,size,bit_rate:stream=codec_name,profile,width,height,r_frame_rate,pix_fmt,sample_rate,channels -of default=nw=1 "$SHARE"
