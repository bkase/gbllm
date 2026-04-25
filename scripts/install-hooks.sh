#!/usr/bin/env bash
# Install git hooks for gbllm development.
# Idempotent — safe to run multiple times.
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
HOOKS_SRC="$REPO_ROOT/hooks"
HOOKS_DST="$REPO_ROOT/.git/hooks"

for hook in "$HOOKS_SRC"/*; do
    name="$(basename "$hook")"
    target="$HOOKS_DST/$name"

    # Symlink (or replace existing symlink)
    ln -sf "$hook" "$target"
    echo "installed: $name"
done
