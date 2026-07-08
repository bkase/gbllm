"""Tests for the cross-language conformance-vector parity gate."""

from __future__ import annotations

import json

import pytest

from gbtrain.conformance import (
    CONFORMANCE_TEXTS,
    build_vectors,
    save_vectors,
    verify_vectors,
)
from gbtrain.tokenizer import pretokenize, train_bpe

CORPUS = (
    "the little robot dreamed of electric sheep in snake_case and __init__ "
    "counting 1 2 3 42 don't can't café — end\n"
) * 60


def _model():
    return train_bpe(CORPUS, vocab_size=420)


def test_all_conformance_texts_are_total_and_round_trip() -> None:
    model = _model()
    for text in CONFORMANCE_TEXTS:
        assert "".join(pretokenize(text)) == text, f"dropped chars in {text!r}"
        assert model.decode(model.encode(text)) == text


def test_save_and_verify_round_trip(tmp_path) -> None:
    model = _model()
    path = tmp_path / "conf.json"
    save_vectors(model, path)
    verify_vectors(model, path)  # must not raise


def test_vectors_are_deterministic_across_training() -> None:
    a = build_vectors(train_bpe(CORPUS, vocab_size=420))
    b = build_vectors(train_bpe(CORPUS, vocab_size=420))
    assert a == b


def test_verify_detects_id_mismatch(tmp_path) -> None:
    model = _model()
    path = tmp_path / "conf.json"
    save_vectors(model, path)
    obj = json.loads(path.read_text())
    # corrupt one non-empty vector's ids
    for v in obj["vectors"]:
        if v["ids"]:
            v["ids"][0] = (v["ids"][0] + 1) % model.vocab_size
            break
    path.write_text(json.dumps(obj))
    with pytest.raises(ValueError):
        verify_vectors(model, path)
