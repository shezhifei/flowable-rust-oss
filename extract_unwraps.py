#!/usr/bin/env python3
"""Extract unwrap/expect call sites from production Rust code (excluding tests).

Skips: */tests/* paths, target/, and #[cfg(test)] modules inside src files.
Output: TSV  file<TAB>line<TAB>col<TAB>snippet (the matched line, stripped)
"""
import os, re, sys

ROOTS = ["modules", "flowable-content-storage", "ui"]
BASE = os.path.dirname(os.path.abspath(__file__))

PAT = re.compile(r"\.(unwrap|unwrap_or|unwrap_or_else|unwrap_or_default|expect)\s*\(")

def strip_test_modules(src: str) -> list[bool]:
    """Return per-line keep flag; False for lines inside #[cfg(test)] items."""
    keep = [True] * (src.count("\n") + 2)
    i, n = 0, len(src)
    # state machine for comments/strings
    line = 1
    cfg_test_positions = []
    while i < n:
        ch = src[i]
        if ch == "\n":
            line += 1; i += 1; continue
        if src.startswith("//", i):
            j = src.find("\n", i); i = n if j < 0 else j; continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2); j = n if j < 0 else j + 2
            line += src.count("\n", i, j); i = j; continue
        if ch == '"':
            # raw string?
            m = re.match(r'r(#+)"', src[max(0,i-2):i+1])
            j = i + 1
            while j < n:
                if src[j] == "\\": j += 2; continue
                if src[j] == '"': break
                if src[j] == "\n": line += 1
                j += 1
            i = j + 1; continue
        if ch == "'":
            # char literal 'x' or escape — skip lifetime names like 'a
            m = re.match(r"'(\\.|[^'\\])'", src[i:i+4])
            if m: i += len(m.group(0)); continue
            i += 1; continue
        if src.startswith("#[cfg(test)]", i):
            cfg_test_positions.append((i, line)); i += 12; continue
        i += 1
    for pos, ln in cfg_test_positions:
        # find next '{' after pos (skipping attribute tokens like `mod tests`)
        j = src.find("{", pos)
        if j < 0: continue
        # balance braces
        depth, k, l2 = 0, j, ln
        while k < n:
            ch = src[k]
            if ch == "\n": l2 += 1
            elif src.startswith("//", k):
                e = src.find("\n", k); k = n if e < 0 else e; continue
            elif src.startswith("/*", k):
                e = src.find("*/", k + 2); e = n if e < 0 else e + 2
                l2 += src.count("\n", k, e); k = e; continue
            elif ch == '"':
                k += 1
                while k < n:
                    if src[k] == "\\": k += 2; continue
                    if src[k] == '"': break
                    if src[k] == "\n": l2 += 1
                    k += 1
            elif ch == "'":
                m = re.match(r"'(\\.|[^'\\])'", src[k:k+4])
                if m: k += len(m.group(0)); continue
            elif ch == "{": depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    k += 1
                    for x in range(ln, l2 + 1):
                        if 0 <= x - 1 < len(keep): keep[x - 1] = False
                    break
            k += 1
    return keep

def main():
    rows = []
    for root in ROOTS:
        rdir = os.path.join(BASE, root)
        if not os.path.isdir(rdir): continue
        for dirpath, dirs, files in os.walk(rdir):
            rel_dir = os.path.relpath(dirpath, BASE)
            parts = rel_dir.replace("\\", "/").split("/")
            if "target" in parts or "tests" in parts: 
                dirs[:] = []
                continue
            for f in files:
                if not f.endswith(".rs"): continue
                path = os.path.join(dirpath, f)
                rel = os.path.relpath(path, BASE).replace("\\", "/")
                try:
                    src = open(path, encoding="utf-8").read()
                except Exception:
                    continue
                keep = strip_test_modules(src)
                for idx, text in enumerate(src.split("\n"), 1):
                    if idx - 1 < len(keep) and not keep[idx - 1]: continue
                    if PAT.search(text):
                        rows.append((rel, idx, text.strip()))
    rows.sort()
    out = open(os.path.join(BASE, "target", "unwrap_sites.tsv"), "w", encoding="utf-8")
    for rel, idx, text in rows:
        out.write(f"{rel}\t{idx}\t{text}\n")
    out.close()
    print(f"total: {len(rows)}")
    # per-file histogram
    from collections import Counter
    c = Counter(r[0] for r in rows)
    for f, n in c.most_common(60):
        print(f"{n:5d}  {f}")

if __name__ == "__main__":
    main()
