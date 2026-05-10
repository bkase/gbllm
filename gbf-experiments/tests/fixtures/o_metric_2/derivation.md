# O-metric-2 hand-counted baseline fixture

Corpus bytes: `ababa`

No BOS, EOS, padding, or document-boundary bytes are added.

Raw counts:

- P1: `a=3`, `b=2`
- P2: `ab=2`, `ba=2`
- P3: `aba=2`, `bab=1`

Smoothing:

- `alpha = 0.01`
- `|Sigma| = 256`
- `P(c | ctx) = (count(ctx,c) + alpha) / (count(ctx) + alpha * 256)`

Selected probabilities:

- `P1(a) = 3.01 / 7.56 = 0.39814814814814814`
- `P1(b) = 2.01 / 7.56 = 0.26587301587301587`
- `P2(b | a) = 2.01 / 4.56 = 0.44078947368421045`
- `P2(a | b) = 2.01 / 4.56 = 0.4407894736842105`
- `P3(a | ab) = 2.01 / 4.56 = 0.4407894736842105`

Reset-context trigram scoring for validation bytes `aba`:

- Position 0 has empty context: `P(a) = P1(a)`
- Position 1 has one byte of context: `P(b | a) = 0.9 * P2(b | a) + 0.1 * P1(b)`
- Position 2 has two bytes of context:
  `P(a | ab) = 0.6 * P3(a | ab) + 0.3 * P2(a | b) + 0.1 * P1(a)`

`bpc = -(log2(P0) + log2(P1) + log2(P2)) / 3`
