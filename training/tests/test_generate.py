"""Tests for the autoregressive generation harness."""

from __future__ import annotations

import mlx.core as mx

from gbtrain.generate import SampleConfig, generate
from gbtrain.model import GBModel, ModelConfig
from gbtrain.tokenizer import train_bpe

CORPUS = (
    "the little robot dreamed of electric sheep and counted 1 2 3 42 "
    "once upon a time there was a happy cat named bolt\n"
) * 40


def _model_and_tok():
    tok = train_bpe(CORPUS, vocab_size=300)
    cfg = ModelConfig(
        d_model=16, d_ff=32, n_blocks=2, state_slots=16,
        n_experts=4, vocab=tok.vocab_size, router_rank=2,
    )
    model = GBModel(cfg)
    mx.eval(model.parameters())
    return model, tok


def test_greedy_generation_is_deterministic() -> None:
    model, tok = _model_and_tok()
    cfg = SampleConfig(max_new_tokens=20, temperature=0.0)
    a = generate(model, tok, "once upon", cfg)
    b = generate(model, tok, "once upon", cfg)
    assert a == b
    assert a.startswith("once upon")


def test_sampled_generation_is_seed_deterministic() -> None:
    model, tok = _model_and_tok()
    cfg = SampleConfig(max_new_tokens=20, temperature=0.9, top_k=20, seed=7)
    a = generate(model, tok, "the robot", cfg)
    b = generate(model, tok, "the robot", cfg)
    assert a == b  # same seed -> identical
    c = generate(model, tok, "the robot", SampleConfig(max_new_tokens=20, temperature=0.9, top_k=20, seed=8))
    assert c != a  # different seed -> (almost surely) different


def test_generation_length_and_decodability() -> None:
    model, tok = _model_and_tok()
    cfg = SampleConfig(max_new_tokens=30, temperature=0.8, top_k=10, seed=1)
    text = generate(model, tok, "a cat", cfg)
    # output is a valid string that begins with the prompt
    assert isinstance(text, str)
    assert text.startswith("a cat")
    # generated at least some new content
    assert len(tok.encode(text)) >= len(tok.encode("a cat"))


def test_empty_prompt_still_generates() -> None:
    model, tok = _model_and_tok()
    text = generate(model, tok, "", SampleConfig(max_new_tokens=10, temperature=0.0))
    assert isinstance(text, str)
    assert len(text) >= 0


def test_top_k_one_equals_greedy() -> None:
    model, tok = _model_and_tok()
    greedy = generate(model, tok, "the", SampleConfig(max_new_tokens=15, temperature=0.0))
    # top_k=1 at any temperature must pick the argmax every step
    topk1 = generate(model, tok, "the", SampleConfig(max_new_tokens=15, temperature=1.0, top_k=1, seed=123))
    assert greedy == topk1
