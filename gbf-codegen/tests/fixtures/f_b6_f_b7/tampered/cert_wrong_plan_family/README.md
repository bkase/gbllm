Deterministic tampered certificate fixture: wrong plan family.

`scripts/review/f-b6-f-b7/run-cert-verify.sh` materializes
`range.cert.json` for this fixture from the packet's passing certificate and
drives it through the real `gbf-verify range-cert verify` CLI.
