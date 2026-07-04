# Kernel bake-off (kernel_bakeoff.v1)

Fixture: 64 fan-in x 32 rows = 2048 MACs; weight seed 0xba5e0ff0, activation seed 0xac71.

| variant | zeros (permille) | M-cycles | M-cycles/MAC | program bytes | data bytes |
|---|---|---|---|---|---|
| v1_interpreted | 0 | 63123 | 30.821 | 227 | 576 |
| v2_dispatch | 0 | 27153 | 13.258 | 2699 | 774 |
| v3_weights_as_code | 0 | 16032 | 7.828 | 13943 | 0 |
| v1_interpreted | 400 | 58219 | 28.427 | 227 | 576 |
| v2_dispatch | 400 | 21015 | 10.261 | 2699 | 774 |
| v3_weights_as_code | 400 | 11030 | 5.385 | 9013 | 0 |
| v1_interpreted | 600 | 55755 | 27.224 | 227 | 576 |
| v2_dispatch | 600 | 17922 | 8.750 | 2699 | 774 |
| v3_weights_as_code | 600 | 8297 | 4.051 | 6482 | 0 |
| v1_interpreted | 900 | 51979 | 25.380 | 227 | 576 |
| v2_dispatch | 900 | 13233 | 6.461 | 2699 | 774 |
| v3_weights_as_code | 900 | 2923 | 1.427 | 2236 | 0 |

Projections at 400 permille zeros (matvec floor, no norms/router/decode):

| profile | MACs/token | variant | M-cycles/token | tok/s (100%) | tok/s (70%) |
|---|---|---|---|---|---|
| Toy1 | 12800 | v1_interpreted | 363865 | 2.881 | 2.017 |
| Toy1 | 12800 | v2_dispatch | 131340 | 7.983 | 5.588 |
| Toy1 | 12800 | v3_weights_as_code | 68928 | 15.212 | 10.648 |
| MoeTiny | 87040 | v1_interpreted | 2474286 | 0.423 | 0.296 |
| MoeTiny | 87040 | v2_dispatch | 893117 | 1.174 | 0.821 |
| MoeTiny | 87040 | v3_weights_as_code | 468710 | 2.237 | 1.566 |
| UpperBankCandidate-96 | 192000 | v1_interpreted | 5457984 | 0.192 | 0.134 |
| UpperBankCandidate-96 | 192000 | v2_dispatch | 1970112 | 0.532 | 0.372 |
| UpperBankCandidate-96 | 192000 | v3_weights_as_code | 1033920 | 1.014 | 0.709 |
| UpperBankCandidate-128 | 272384 | v1_interpreted | 7743059 | 0.135 | 0.094 |
| UpperBankCandidate-128 | 272384 | v2_dispatch | 2794932 | 0.375 | 0.262 |
| UpperBankCandidate-128 | 272384 | v3_weights_as_code | 1466787 | 0.714 | 0.500 |
| QualityDense-144x288x6 | 633600 | v1_interpreted | 18011347 | 0.058 | 0.040 |
| QualityDense-144x288x6 | 633600 | v2_dispatch | 6501369 | 0.161 | 0.112 |
| QualityDense-144x288x6 | 633600 | v3_weights_as_code | 3411936 | 0.307 | 0.215 |
| QualityDense-160x320x6 | 780800 | v1_interpreted | 22195801 | 0.047 | 0.033 |
| QualityDense-160x320x6 | 780800 | v2_dispatch | 8011788 | 0.130 | 0.091 |
| QualityDense-160x320x6 | 780800 | v3_weights_as_code | 4204608 | 0.249 | 0.174 |
| QualityDense-192x384x7 | 1305600 | v1_interpreted | 37114291 | 0.028 | 0.019 |
| QualityDense-192x384x7 | 1305600 | v2_dispatch | 13396761 | 0.078 | 0.054 |
| QualityDense-192x384x7 | 1305600 | v3_weights_as_code | 7030656 | 0.149 | 0.104 |

- Single-bank fixture: no ROM bank switching or SRAM paging in the measured region.
- Kernels run with interrupts disabled and SP repurposed (V2/V3); production kernels pay yield/safe-point overhead on top.
- Projections cover matvec MACs only; norms, router, per-row scales, and decode are excluded.
- MACs-per-token formula assumes one d_model^2 state mix plus a 2*d_model*d_ff expert per block plus a tied 80-token head.
