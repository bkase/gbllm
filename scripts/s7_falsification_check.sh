#!/usr/bin/env bash
set -euo pipefail

cargo test -p gbf-experiments --no-default-features --features falsify --test falsification_s7
