"""Tests for the TinyStories token dataset + BPTT batching."""

from __future__ import annotations

import numpy as np
import pytest

from gbtrain.data import (
    Dataset,
    iter_bptt_batches,
    tokenize_text,
)
from gbtrain.tokenizer import train_bpe

TEXT = (
    "Once upon a time a small robot learned to read. "
    "It counted 1 2 3 and dreamed of electric sheep. "
) * 40


def _model():
    return train_bpe(TEXT, vocab_size=400)


def test_tokenize_is_uint16_and_in_range() -> None:
    m = _model()
    arr = tokenize_text(m, TEXT)
    assert arr.dtype == np.uint16
    assert arr.max() < m.vocab_size


def test_tokenize_round_trips_through_array() -> None:
    m = _model()
    arr = tokenize_text(m, TEXT)
    assert m.decode(arr.tolist()) == TEXT


def test_bptt_batches_shapes_and_target_shift() -> None:
    m = _model()
    arr = tokenize_text(m, TEXT)
    seq_len, lanes = 8, 4
    batches = list(iter_bptt_batches(arr, seq_len, lanes))
    assert batches, "expected at least one batch"
    for x, y in batches:
        assert x.shape == (lanes, seq_len)
        assert y.shape == (lanes, seq_len)
    # y is x shifted by one within each lane's contiguous stream
    per_lane = arr.size // lanes
    grid = arr[: lanes * per_lane].reshape(lanes, per_lane).astype(np.int32)
    x0, y0 = batches[0]
    assert np.array_equal(x0, grid[:, 0:seq_len])
    assert np.array_equal(y0, grid[:, 1 : seq_len + 1])


def test_bptt_is_deterministic() -> None:
    m = _model()
    arr = tokenize_text(m, TEXT)
    a = [(x.copy(), y.copy()) for x, y in iter_bptt_batches(arr, 8, 4)]
    b = [(x.copy(), y.copy()) for x, y in iter_bptt_batches(arr, 8, 4)]
    assert len(a) == len(b)
    for (xa, ya), (xb, yb) in zip(a, b):
        assert np.array_equal(xa, xb) and np.array_equal(ya, yb)


def test_bptt_step_count() -> None:
    m = _model()
    arr = tokenize_text(m, TEXT)
    seq_len, lanes = 8, 4
    per_lane = arr.size // lanes
    expected = (per_lane - 1) // seq_len
    assert len(list(iter_bptt_batches(arr, seq_len, lanes))) == expected


def test_bptt_rejects_too_short_stream() -> None:
    arr = np.arange(10, dtype=np.uint16)
    with pytest.raises(ValueError):
        list(iter_bptt_batches(arr, seq_len=8, lanes=4))


def test_dataset_save_load_round_trip(tmp_path) -> None:
    m = _model()
    ds = Dataset(
        train=tokenize_text(m, TEXT),
        val=tokenize_text(m, TEXT[:500]),
        vocab_size=m.vocab_size,
    )
    ds.save(tmp_path)
    back = Dataset.load(tmp_path)
    assert np.array_equal(back.train, ds.train)
    assert np.array_equal(back.val, ds.val)
    assert back.vocab_size == ds.vocab_size
