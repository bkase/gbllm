"""CLI: generate text from a trained subword MoE student.

Runs on the CPU device by default so it can sample beside a live GPU training
job without contending for Metal. Loads either a hardened export (deployable
math) or a raw trainable checkpoint, autodetected.

Example:
  uv run python run_sample.py --ckpt artifacts/student_moe_d192x8 \
    --tokenizer artifacts/tinystories_bpe_1024.json \
    --prompt "Once upon a time" --max-new-tokens 200 --temperature 0.8 --top-k 40
"""

from __future__ import annotations

import argparse

import mlx.core as mx

from gbtrain.generate import SampleConfig, generate, load_model
from gbtrain.tokenizer import BPEModel


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ckpt", required=True, help="checkpoint dir (hardened export or trainable)")
    ap.add_argument("--tokenizer", default="artifacts/tinystories_bpe_1024.json")
    ap.add_argument("--prompt", action="append", default=None, help="repeatable; one sample per prompt")
    ap.add_argument("--max-new-tokens", type=int, default=200)
    ap.add_argument("--temperature", type=float, default=0.8)
    ap.add_argument("--top-k", type=int, default=40)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--greedy", action="store_true", help="temperature=0 (argmax)")
    ap.add_argument("--device", choices=["cpu", "gpu"], default="cpu")
    args = ap.parse_args()

    mx.set_default_device(mx.cpu if args.device == "cpu" else mx.gpu)

    tok = BPEModel.load(args.tokenizer)
    model = load_model(args.ckpt)

    prompts = args.prompt or ["Once upon a time", "The little robot", "One day"]
    temperature = 0.0 if args.greedy else args.temperature

    for i, prompt in enumerate(prompts):
        cfg = SampleConfig(
            max_new_tokens=args.max_new_tokens,
            temperature=temperature,
            top_k=args.top_k,
            seed=args.seed + i,
        )
        text = generate(model, tok, prompt, cfg)
        print(f"\n=== sample {i} (prompt={prompt!r}, T={temperature}, top_k={args.top_k}) ===")
        print(text)


if __name__ == "__main__":
    main()
