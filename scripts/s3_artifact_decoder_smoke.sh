#!/usr/bin/env bash
set -euo pipefail

cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" --test artifact_decoder_argmax_s3
