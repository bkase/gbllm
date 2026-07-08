"""Tests for the byte-level BPE tokenizer — the subword foundation.

The tokenizer is a build-time artifact shared by the MLX trainer and the Rust
deploy pipeline, so the properties that matter are: losslessness (round-trip),
determinism (byte-identical merges from the same corpus), and a portable
artifact that replays to the same vocab.
"""

from __future__ import annotations

import pytest

import json
import random

from gbtrain.tokenizer import BASE_VOCAB, BPEModel, pretokenize, train_bpe

SAMPLE = (
    "Once upon a time, there was a little robot named Bolt. "
    "Bolt loved to count: 1, 2, 3, and 42! "
    "The machines dreamed of electric sheep.\n\tTabs\tand   spaces.  "
)


def _tiny_model(vocab_size: int = 320) -> BPEModel:
    corpus = SAMPLE * 50
    return train_bpe(corpus, vocab_size=vocab_size)


@pytest.mark.parametrize(
    "text",
    [
        "",
        "a",
        "hello world",
        SAMPLE,
        "punctuation!?;:'\"()[]{}...",
        "   leading and trailing   ",
        "digits 1234567890 mixed w0rds",
        "unicode: café résumé naïve — em‑dash “quotes”",
        "\n\n\t\t newlines and tabs \r\n",
        "emoji 🤖🎮 survive as bytes",
    ],
)
def test_round_trip_is_lossless(text: str) -> None:
    model = _tiny_model()
    assert model.decode(model.encode(text)) == text


def test_base_bytes_alone_round_trip_before_any_merge() -> None:
    # A model with no merges is exactly byte identity.
    model = BPEModel(merges=[])
    assert model.vocab_size == BASE_VOCAB
    for text in ["", "abc", "café 🤖", SAMPLE]:
        assert model.decode(model.encode(text)) == text


def test_training_is_deterministic() -> None:
    a = train_bpe(SAMPLE * 30, vocab_size=340)
    b = train_bpe(SAMPLE * 30, vocab_size=340)
    assert a.merges == b.merges
    assert a.to_json() == b.to_json()


def test_vocab_size_is_respected() -> None:
    model = train_bpe(SAMPLE * 30, vocab_size=300)
    assert model.vocab_size <= 300
    assert model.vocab_size >= BASE_VOCAB


def test_merges_actually_compress() -> None:
    model = _tiny_model(vocab_size=400)
    # repetitive text should encode to far fewer tokens than raw bytes
    ids = model.encode(SAMPLE * 5)
    raw = len((SAMPLE * 5).encode("utf-8"))
    assert len(ids) < raw
    assert raw / len(ids) > 1.5  # >1.5 bytes/token on repetitive text


def test_ids_are_in_range() -> None:
    model = _tiny_model()
    for tid in model.encode(SAMPLE):
        assert 0 <= tid < model.vocab_size
        assert isinstance(model.token_bytes(tid), bytes)


def test_artifact_json_round_trip_and_cross_check() -> None:
    model = _tiny_model()
    restored = BPEModel.from_json(model.to_json())
    assert restored.merges == model.merges
    assert restored.vocab == model.vocab
    # encodes identically
    assert restored.encode(SAMPLE) == model.encode(SAMPLE)


def test_artifact_detects_corruption() -> None:
    import json

    model = _tiny_model()
    obj = json.loads(model.to_json())
    # corrupt one explicit id->bytes entry; from_json must catch the mismatch
    some_merged_id = str(BASE_VOCAB + 0)
    obj["id_bytes_hex"][some_merged_id] = "00"
    with pytest.raises(ValueError):
        BPEModel.from_json(json.dumps(obj))


@pytest.mark.parametrize(
    "text",
    [
        "hello_world",
        "__init__",
        "snake_case_name",
        "x_1 a_b c_d_e",
        "under_score‿connector",  # U+203F connector punctuation
        "mix_of/every.kind! of123boundary\tand\nnewline",
    ],
)
def test_pretokenizer_is_total_no_dropped_chars(text: str) -> None:
    # regression for the underscore-dropping bug: joining all chunks must
    # reconstruct the input exactly (nothing skipped by re.findall).
    assert "".join(pretokenize(text)) == text


def test_underscore_round_trips_losslessly() -> None:
    model = _tiny_model()
    for text in ["hello_world", "__init__", "a_b_c", "snake_case"]:
        assert model.decode(model.encode(text)) == text


def test_pretokenizer_total_on_random_fuzz() -> None:
    rng = random.Random(1234)
    alphabet = "abcXYZ_ .!?\t\n0129/‿café—🤖"
    for _ in range(500):
        text = "".join(rng.choice(alphabet) for _ in range(rng.randint(0, 40)))
        assert "".join(pretokenize(text)) == text


def test_from_json_rejects_incomplete_id_table() -> None:
    model = _tiny_model()
    obj = json.loads(model.to_json())
    del obj["id_bytes_hex"][str(model.vocab_size - 1)]  # drop one entry
    with pytest.raises(ValueError):
        BPEModel.from_json(json.dumps(obj))


def test_artifact_exports_max_token_len() -> None:
    model = _tiny_model()
    obj = json.loads(model.to_json())
    assert obj["max_token_len"] == model.max_token_len
    assert obj["max_token_len"] >= 1


def test_first_merge_is_most_frequent_pair() -> None:
    # "ab" repeated makes (97,98) the dominant adjacent pair.
    model = train_bpe("ab " * 200, vocab_size=BASE_VOCAB + 1)
    assert model.merges[0] == (ord("a"), ord("b"))
    assert model.vocab[BASE_VOCAB] == b"ab"
