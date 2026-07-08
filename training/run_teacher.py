"""Launch the dense fp32 TEACHER training run (bd-wfghf).

Dense (n_experts=1) d_model=512, d_ff=1024, n_blocks=6, state_slots=512,
vocab=1024 on ds_ts_1024, seq_len=256, lanes=128. Checkpoints (weights +
config) written to artifacts/teacher_d512/ every ckpt_every steps and at the
end -- export is definition-of-done.

    nohup uv run python run_teacher.py > artifacts/teacher_d512.log 2>&1 &
"""

from __future__ import annotations

import time

import mlx.core as mx

from gbtrain.data import Dataset
from gbtrain.model import GBModel, ModelConfig
from gbtrain.tokenizer import BPEModel
from gbtrain.train import TrainConfig, byte_len_table, train


def main() -> None:
    mx.random.seed(0)
    print(f"[teacher] mlx device={mx.default_device()} start={time.ctime()}", flush=True)

    ds = Dataset.load("artifacts/ds_ts_1024")
    bpe = BPEModel.load("artifacts/tinystories_bpe_1024.json")
    byte_lens = byte_len_table(bpe)
    print(
        f"[teacher] train={ds.train.size:,} val={ds.val.size:,} tok vocab={ds.vocab_size} "
        f"mean bytes/token(train)={ds.train.size and byte_lens[ds.train.astype('int64')].mean():.3f}",
        flush=True,
    )

    mcfg = ModelConfig(
        d_model=512, d_ff=1024, n_blocks=6, state_slots=512, n_experts=1,
        vocab=ds.vocab_size, qat_weights=False, qat_acts=False,
    )
    model = GBModel(mcfg)
    mx.eval(model.parameters())
    nparams = sum(v.size for _, v in _flat(model.parameters()))
    print(f"[teacher] model params={nparams:,}  cfg={mcfg.to_dict()}", flush=True)

    tcfg = TrainConfig(
        seq_len=256, lanes=128, steps=30000,
        lr_peak=2e-3, lr_min=2e-4, warmup_steps=500, weight_decay=0.01, grad_clip=1.0,
        eval_every=1000, eval_batches=40, ckpt_every=1000, log_every=20,
        ckpt_dir="artifacts/teacher_d512", seed=0,
    )
    print(f"[teacher] train_cfg={tcfg}", flush=True)

    train(model, ds.train, ds.val, byte_lens, tcfg)
    print(f"[teacher] done {time.ctime()}", flush=True)


def _flat(tree):
    out = []
    if isinstance(tree, dict):
        for v in tree.values():
            out += _flat(v)
    elif isinstance(tree, list):
        for v in tree:
            out += _flat(v)
    else:
        out.append(("", tree))
    return out


if __name__ == "__main__":
    main()
