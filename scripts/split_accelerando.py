"""Split the textutil-converted Accelerando into per-chapter and train/eval files.

Input:  corpus/accelerando/raw/accelerando.txt
        (produced by `textutil -convert txt -encoding UTF-8` from
         corpus/accelerando/raw/accelerando.html, which is the canonical
         CC BY-NC-ND 2.5 source from antipope.org)
Output: corpus/accelerando/chapters/<NN-slug>.txt
        corpus/accelerando/splits/{train,eval}.txt
        corpus/accelerando/manifest.json
        corpus/accelerando/README.md

Light cleanup only: strip per-line leading tabs (HTML/RTF indent residue),
remove image placeholders (the legacy SPECIAL_IMAGE-*-REPLACE_ME tokens from
the older RTF path AND U+FFFC object-replacement chars produced by textutil
from the HTML path), normalize blank-line runs to a single blank.
NO unicode-fixup, NO charset folding — that work belongs to the deterministic
normalization step in gbf-data (bd-tmaw / F-G1).
"""
import hashlib
import json
import re
from pathlib import Path

ROOT = Path("/Users/bkase/Documents/gbllm/corpus/accelerando")
RAW = ROOT / "raw" / "accelerando.txt"
CHAPTERS_DIR = ROOT / "chapters"
SPLITS_DIR = ROOT / "splits"

EVAL_CHAPTER = 4  # Halo — mid-book, stylistically representative, ~10% of words

CHAPTER_RE = re.compile(r"^Chapter (\d+):\s*(.+)$")
END_RE = re.compile(r"^\(THE END\b")
IMG_PLACEHOLDER_RE = re.compile(r"SPECIAL_IMAGE-[^\s]+-REPLACE_ME")

# U+FFFC OBJECT REPLACEMENT CHARACTER — textutil emits this for embedded
# <img> tags from the HTML source. Strip every instance.
OBJECT_REPLACEMENT_CHAR = "￼"

def slugify(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")

def clean_line(line: str) -> str:
    line = line.rstrip("\n")
    # Strip leading tabs (HTML/RTF indent residue) but preserve internal structure.
    line = line.lstrip("\t")
    line = IMG_PLACEHOLDER_RE.sub("", line)
    line = line.replace(OBJECT_REPLACEMENT_CHAR, "")
    return line.rstrip()

def collapse_blank_runs(lines):
    out = []
    blank = False
    for ln in lines:
        if ln == "":
            if not blank:
                out.append(ln)
            blank = True
        else:
            out.append(ln)
            blank = False
    # trim trailing blanks
    while out and out[-1] == "":
        out.pop()
    return out

def main():
    raw_text = RAW.read_text(encoding="utf-8")
    raw_lines = raw_text.splitlines()

    # Identify chapter spans.
    chapter_indices = []  # list of (chapter_num, title, start_line_idx)
    end_idx = len(raw_lines)
    for i, ln in enumerate(raw_lines):
        m = CHAPTER_RE.match(ln)
        if m:
            chapter_indices.append((int(m.group(1)), m.group(2).strip(), i))
        elif END_RE.match(ln):
            end_idx = i  # exclusive

    if not chapter_indices:
        raise SystemExit("no chapters found")

    CHAPTERS_DIR.mkdir(parents=True, exist_ok=True)
    SPLITS_DIR.mkdir(parents=True, exist_ok=True)

    chapter_files = []
    for idx, (num, title, start) in enumerate(chapter_indices):
        stop = chapter_indices[idx + 1][2] if idx + 1 < len(chapter_indices) else end_idx
        # body excludes the chapter heading line itself
        body = raw_lines[start + 1 : stop]
        body = [clean_line(ln) for ln in body]
        body = collapse_blank_runs(body)

        slug = f"{num:02d}-{slugify(title)}"
        out_path = CHAPTERS_DIR / f"{slug}.txt"
        # Each chapter file: title line, blank, then body.
        text = f"{title}\n\n" + "\n".join(body) + "\n"
        out_path.write_text(text, encoding="utf-8")
        chapter_files.append((num, title, slug, out_path, text))

    # Build splits.
    train_chunks = []
    eval_chunks = []
    for num, title, slug, path, text in chapter_files:
        if num == EVAL_CHAPTER:
            eval_chunks.append(text)
        else:
            train_chunks.append(text)

    (SPLITS_DIR / "train.txt").write_text(
        "".join(train_chunks), encoding="utf-8"
    )
    (SPLITS_DIR / "eval.txt").write_text(
        "".join(eval_chunks), encoding="utf-8"
    )

    # Manifest.
    def hash_file(p: Path) -> str:
        return hashlib.sha256(p.read_bytes()).hexdigest()

    def stats(p: Path):
        text = p.read_text(encoding="utf-8")
        words = len(text.split())
        return {
            "bytes": p.stat().st_size,
            "lines": text.count("\n"),
            "words": words,
            "sha256": hash_file(p),
        }

    manifest = {
        "source": {
            "path": str(RAW.relative_to(ROOT)),
            "stats": stats(RAW),
            "provenance": {
                "origin_url": "https://www.antipope.org/charlie/blog-static/fiction/accelerando/accelerando.html",
                "license": "CC BY-NC-ND 2.5",
                "author": "Charles Stross",
                "year": 2005,
                "source_format": "HTML (ISO-8859-1, with named HTML entities)",
                "conversion_tool": "macOS textutil -convert txt -encoding UTF-8",
                "note": "HTML source preserves proper unicode (©, é, –, smart quotes, etc.). The earlier RTF source was lossy and was replaced. Charset folding to the Tier 2 80-token vocab is deferred to gbf-data NormalizationSpec under bd-tmaw / F-G1.",
            },
        },
        "chapters": [
            {
                "number": num,
                "title": title,
                "slug": slug,
                "path": str(path.relative_to(ROOT)),
                "stats": stats(path),
                "in_split": "eval" if num == EVAL_CHAPTER else "train",
            }
            for num, title, slug, path, _ in chapter_files
        ],
        "splits": {
            "eval_chapter_number": EVAL_CHAPTER,
            "eval_chapter_title": next(t for n, t, *_ in chapter_files if n == EVAL_CHAPTER),
            "train": stats(SPLITS_DIR / "train.txt"),
            "eval": stats(SPLITS_DIR / "eval.txt"),
        },
        "post_processing_applied": [
            "strip leading tabs from each line (HTML/RTF indent residue)",
            "remove SPECIAL_IMAGE-*-REPLACE_ME placeholders (legacy RTF path)",
            "remove U+FFFC OBJECT REPLACEMENT CHARACTER (HTML <img> placeholders)",
            "collapse runs of blank lines to a single blank",
            "trim trailing blank lines per chapter",
        ],
        "post_processing_NOT_applied": [
            "unicode normalization (NFC, accent strip)",
            "smart-quote / em-dash / ellipsis folding",
            "case folding",
            "unmappable codepoint handling",
            "all of the above belong to gbf-data NormalizationSpec under bd-tmaw/F-G1, hashed into LexicalSpec identity",
        ],
    }
    (ROOT / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )

    # README
    readme_lines = [
        "# Accelerando corpus",
        "",
        "Charles Stross, *Accelerando* (2005). Released under CC BY-NC-ND 2.5.",
        "Canonical source: https://www.antipope.org/charlie/blog-static/fiction/accelerando/accelerando.html",
        "",
        "**Do not commit the contents of this directory to git.** The license permits redistribution under attribution, but checking copyrighted prose into a public repo is messy. `corpus/` is in `.gitignore`. The conversion script (in version control) plus the original HTML (locally on disk) reproduce everything here.",
        "",
        "## Layout",
        "",
        "- `raw/accelerando.html` — fetched HTML, ISO-8859-1, untouched.",
        "- `raw/accelerando.txt` — textutil output from the HTML, UTF-8. Untouched apart from textutil's own decoding.",
        "- `chapters/NN-slug.txt` — per-chapter files. Light cleanup only: leading-tab strip, image-placeholder removal (`SPECIAL_IMAGE-*-REPLACE_ME` and U+FFFC), blank-line normalization. **No** charset folding or unicode normalization — that step lives in `gbf-data` (bd-tmaw / F-G1) so it stays deterministic and identity-hashed.",
        "- `splits/train.txt` and `splits/eval.txt` — derived. The eval split is **Chapter 4: Halo** (mid-book, stylistically representative, ~10% of body words). It is the held-out set: never used for training, never fed to Gemini for synthetic expansion, never shown to the model during training. Reserved as the perplexity gold-standard.",
        "- `manifest.json` — per-file SHA-256, byte/line/word counts, provenance, and the list of cleanup steps applied vs deferred.",
        "",
        "## Why split this way",
        "",
        "Per-chapter splitting preserves narrative coherence (versus line-level random splits which destroy it) and gives ~10% chapter-granularity holdout (versus per-line which leaks context across the split). Chapter 4 was chosen because it is mid-book — early enough to share Stross's style and characters with the training set, late enough to be after the world-state has accelerated.",
        "",
        "## How to regenerate",
        "",
        "1. `curl -sL -o corpus/accelerando/raw/accelerando.html https://www.antipope.org/charlie/blog-static/fiction/accelerando/accelerando.html`",
        "2. `textutil -convert txt -encoding UTF-8 -output corpus/accelerando/raw/accelerando.txt corpus/accelerando/raw/accelerando.html`",
        "3. `python3 scripts/split_accelerando.py`",
        "",
        "The conversion is deterministic; the manifest's SHA-256s let you verify your local copy matches.",
    ]
    (ROOT / "README.md").write_text("\n".join(readme_lines) + "\n", encoding="utf-8")

    # Print summary.
    print("Chapters written:")
    for num, title, slug, path, text in chapter_files:
        s = stats(path)
        flag = "  [EVAL]" if num == EVAL_CHAPTER else ""
        print(f"  Ch {num} {title:14s} {s['lines']:5d} lines, {s['words']:6d} words{flag}")
    print()
    train_s = stats(SPLITS_DIR / "train.txt")
    eval_s = stats(SPLITS_DIR / "eval.txt")
    total = train_s["words"] + eval_s["words"]
    print(f"Train: {train_s['lines']:5d} lines, {train_s['words']:6d} words ({100*train_s['words']/total:.1f}%)")
    print(f"Eval:  {eval_s['lines']:5d} lines, {eval_s['words']:6d} words ({100*eval_s['words']/total:.1f}%)")

if __name__ == "__main__":
    main()
