#!/usr/bin/env python3
"""Classify unwrap sites from target/unwrap_sites.tsv into risk buckets."""
import re, sys
from collections import Counter

BUCKETS = [
    ("lock",        re.compile(r"\.(lock|read|write)\(\)\.(unwrap|expect)")),
    ("regex_lazy",  re.compile(r"(Regex::new|RegexBuilder|Lazy|lazy_static|OnceLock|OnceCell|HashSet::from|HashMap::from)")),
    ("serde_de",    re.compile(r"(from_str|from_value|from_slice|from_reader|from_csv)")),
    ("parse",       re.compile(r"\.parse\s*(::<[^>]*>)?\(\)")),
    ("session_db",  re.compile(r"(create_session|flush_and_commit|flush\(|commit\(|begin_transaction)")),
    ("cmd_execute", re.compile(r"\.execute\s*\(")),
    ("unwrap_or",   re.compile(r"\.unwrap_or")),
    ("expect",      re.compile(r"\.expect\s*\(")),
    ("time",        re.compile(r"(SystemTime|UNIX_EPOCH|duration_since|Duration::|Instant::)")),
    ("num_cast",    re.compile(r"(try_into|try_from|to_string|as_bytes|to_vec)")),
    ("env_config",  re.compile(r"(env::|var\(|current_dir|homedir)")),
    ("string_char", re.compile(r"(chars\(\)\.|\.last\(\)|\.first\(\)|\.next\(\))")),
]

def bucket(line: str) -> str:
    for name, pat in BUCKETS:
        if pat.search(line):
            return name
    return "other"

def main():
    path = "target/unwrap_sites.tsv"
    c = Counter()
    by_file_bucket = Counter()
    rows = []
    for raw in open(path, encoding="utf-8"):
        f, ln, text = raw.rstrip("\n").split("\t", 2)
        b = bucket(text)
        c[b] += 1
        by_file_bucket[(f, b)] += 1
        rows.append((b, f, int(ln), text))
    print("== bucket totals ==")
    for b, n in c.most_common():
        print(f"{n:6d}  {b}")
    print("\n== 'other' bucket by file (top 40) ==")
    of = Counter(f for (f, b) in by_file_bucket if b == "other")
    for f, n in of.most_common(40):
        print(f"{n:5d}  {f}")
    # write classified full list
    out = open("target/unwrap_classified.tsv", "w", encoding="utf-8")
    for b, f, ln, text in sorted(rows, key=lambda r: (r[0], r[1], r[2])):
        out.write(f"{b}\t{f}\t{ln}\t{text}\n")
    out.close()

if __name__ == "__main__":
    main()
