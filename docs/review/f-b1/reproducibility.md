# Reproducibility

Run:

```bash
scripts/review/f-b1/verify-packet.sh
```

The script regenerates the packet, hashes review files before and after, and fails if regeneration is not deterministic or leaves a diff.

The checked-in packet is intentionally marked `workspace-unpinned` unless the
build environment supplies `GIT_SHA` and `GIT_DIRTY=0`. That avoids claiming a
clean pinned source revision for local review artifacts generated before the PR
commit exists.
