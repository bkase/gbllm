# MLX model implementation

> **Status: current implementation inventory.** The executable source of truth
> is `training/gbtrain/model.py`, `training/gbtrain/train.py`, and
> `training/gbtrain/qat_schedule.py`. This document describes those files; it
> does not reserve future model behavior.

## Production artifact

The current interactive cartridge uses the hardened dense student at
`training/artifacts/student_dense_d192`:

| Field | Value |
|---|---:|
| d_model | 192 |
| d_ff | 384 |
| residual blocks | 6 |
| recurrent state slots | 192 |
| experts per block | 1 |
| vocabulary | 1,024 byte-BPE tokens |
| training step | 24,000 |

The configurable MLX implementation can construct more than one expert, but
that is not the topology in
`artifacts/builds/gbllm-dense-d192-interactive.gb`.

## Token forward pass

`GBModel.__call__` in `training/gbtrain/model.py` executes a token window;
`GBModel._forward_per_token` is its per-token reference used by the equality
test. Together they execute this sequence for each input token:

1. Look up a row in the fp32 embedding, producing the residual vector `x`.
2. Run the `LinearStateBlock`:
   - RMS-normalize and clip `x`;
   - optionally apply Int8 fake quantization;
   - project into 192 state slots;
   - update each slot as `state = state * decay + input_delta`;
   - project the state back to d192;
   - optionally fake-quantize that output and add it to the residual.
3. Run six pre-norm residual FFN blocks. In the production dense branch each
   block performs `x += down(actq(gelu(up(actq(rms_norm(x))))))`.
4. Apply final RMS normalization.
5. Compute logits with the embedding transposed. The embedding and head are
   tied; there is no independent output-head parameter or bias.

All MLX tensors are explicitly cast to fp32. RMS normalization clips to
[-8, 8], GELU uses the tanh approximation, and the hard-QAT path uses
round-to-even fake quantization.

## Recurrent state

The state uses four contiguous decay bands:

| Slots | Decay | Q8.8 raw |
|---|---:|---:|
| 0–47 | 0.5 | 128 |
| 48–95 | 0.75 | 192 |
| 96–143 | 0.875 | 224 |
| 144–191 | 0.9375 | 240 |

State values carry across tokens and across truncated-BPTT windows. At a window
boundary `training/gbtrain/train.py` stops gradients through the carried value
but does not zero the value. This gives fixed-size sequence memory without a KV
cache that grows with context length.

## Ternary projections and activation QAT

`TernaryLinear` holds latent fp32 weights and a learned threshold. In hard-QAT
mode it:

1. maps each weight to -1, 0, or +1 using a straight-through estimator;
2. at the Off-to-Hard transition, calibrates each row's threshold and scale
   from the latent row magnitudes, then keeps them as model parameters;
3. snaps the stored scale to the Q8.8 grid with a straight-through estimator;
4. applies the scaled ternary matrix multiplication.

Activation fake quantization is attached at four sites:

1. after state-input RMS normalization, before the state-input projection;
2. after the state-output projection, before its residual add;
3. after each FFN RMS normalization, before the up projection;
4. after GELU, before the down projection.

The residual stream, dense down-projection output, and final normalized head
input are not fake-quantized by the MLX model.

## Dense and MoE behavior

`MoEBlock` has two executable branches:

- `n_experts == 1`: run expert zero directly and return zero router auxiliary
  loss;
- `n_experts > 1`: run the low-rank fp32 router, choose hard top-1 experts with
  lowest-index tie breaking, dispatch tokens to those experts, and compute
  z-loss plus batch load-balance loss.

The implemented `ModelConfig` defaults are
`lambda_zrouter = 1e-3` and `lambda_balance = 1e-2`. The MLX model does not
carry a previous router distribution, compute temporal-smoothness loss, or run
the older phase-specific dropout/jitter schedule described by the superseded
design.

## Training schedule

`training/run_teacher.py` trains the dense d512 teacher with QAT off.
`training/run_student.py` trains the deployable student with teacher
distillation and the `Off -> Hard` schedule:

- QAT is off for the first 40% of steps;
- ternary weights and Int8 activations turn on together for the remaining 60%;
- hard-phase learning rate decays toward 0.1 times peak;
- distillation weight rises from 0.5 to 0.65.

The loss is cross-entropy plus temperature-scaled teacher distillation plus
router auxiliary loss. Router auxiliary loss is zero for the production dense
student.

## Deployment status

Vocabulary 1,024 is deployed today. The Rust loader selects paged logits for a
wide vocabulary, and the exact d192/V1024 ROM stores the complete i24 logit
vector in 8 KiB cartridge SRAM. The old single-page limit of 85 logits remains
relevant only to the non-paged layout; it is not a deployment blocker.

Hardening and schema details live in
[export-schema-spec.md](export-schema-spec.md). The complete current-product
walkthrough is the repository [README](../../README.md).

## Remaining truth gaps

- MLX GPU optimization is not promised to be bit-identical across machines.
- The saved production manifest does not capture the original argv, all input
  hashes, git revision/dirty state, MLX version, or device.
- An MoE-capable code path is not evidence that an MoE model is the current ROM
  or that it beats the dense artifact.
