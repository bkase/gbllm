#!/usr/bin/env python3
"""Materialize the gbf-debug JSON envelope into review-friendly evidence."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: materialize.py <exec-envelope.json> <output-dir>")

    envelope_path = Path(sys.argv[1])
    output_dir = Path(sys.argv[2])
    envelope = json.loads(envelope_path.read_text(encoding="utf-8"))
    if envelope.get("command") != "exec":
        raise SystemExit(f"not a successful exec envelope: {envelope.get('command')!r}")

    result = envelope.get("result", {})
    if result.get("schema") != "bd_36akp_interactive_acceptance.v1":
        raise SystemExit(f"unexpected acceptance schema: {result.get('schema')!r}")
    if result.get("passed") is not True:
        raise SystemExit("acceptance result did not pass")

    width = int(result["framebufferWidth"])
    height = int(result["framebufferHeight"])
    framebuffer = bytes(255 - min(int(pixel), 3) * 85 for pixel in result["framebuffer"])
    if len(framebuffer) != width * height:
        raise SystemExit(
            f"framebuffer size mismatch: {len(framebuffer)} != {width} * {height}"
        )

    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "framebuffer.pgm").write_bytes(
        f"P5\n{width} {height}\n255\n".encode("ascii") + framebuffer
    )
    (output_dir / "transcript.txt").write_text(
        "\n".join(row.rstrip() for row in result["transcriptRows"]).rstrip() + "\n",
        encoding="utf-8",
    )

    summary = {key: value for key, value in result.items() if key != "framebuffer"}
    (output_dir / "acceptance-result.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    print("bd-36akp interactive acceptance PASS")
    print(f"prompt token ids: {result['promptTokenIds']}")
    print(f"first generated id: {result['firstGeneratedId']}")
    print(f"generated tokens: {len(result['generatedIds'])}")
    print(f"result: {output_dir / 'acceptance-result.json'}")
    print(f"transcript: {output_dir / 'transcript.txt'}")
    print(f"framebuffer: {output_dir / 'framebuffer.pgm'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
