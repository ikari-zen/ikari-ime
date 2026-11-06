#!/usr/bin/env python3
"""ADR 0055 §4.2 の守る群 phr: Sudachi の普通名詞（コスト -5000〜7500、読み 2〜5 文字のカタカナ）から乱数で作る連なり（ADR 0030 §4 の phr と同じ型）。

  python3 -I webwords_phr.py <data/raw/sudachi-20260723> <出力ディレクトリ>

phr.tsv（11,000 件、seed 7）と phrk.tsv（表記がカタカナで読み 3 文字以上の名詞だけ、6,000 件、seed 11。外来語の追加で動きやすい）を作る。
期待は連なりの表記で、意味のない並びが多い。見るのは「足す前から 1 位が変わった件数」だけ（webwords_compare.py の 1 位の表記が変わった列）。
"""
import random
import re
import sys

SUD = sys.argv[1].rstrip("/") + "/"
OUT = sys.argv[2].rstrip("/") + "/"
KATA = re.compile(r"^[ァ-ヶー]+$")


def hira(s):
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


nouns = []
for name in ("small_lex.csv", "core_lex.csv", "notcore_lex.csv"):
    for line in open(SUD + name, encoding="utf-8"):
        c = line.rstrip("\n").split(",")
        if len(c) < 12 or c[5:8] != ["名詞", "普通名詞", "一般"]:
            continue
        try:
            cost = int(c[3])
        except ValueError:
            continue
        if not (-5000 <= cost <= 7500):
            continue
        r = c[11]
        if not (2 <= len(r) <= 5) or not KATA.match(r):
            continue
        nouns.append((c[4], hira(r)))
nouns = sorted(set(nouns))
print("nouns", len(nouns), file=sys.stderr)

FORMS = [
    ("a+b", lambda a, b, c: (a[0] + b[0], a[1] + b[1])),
    ("anob", lambda a, b, c: (a[0] + "の" + b[0], a[1] + "の" + b[1])),
    ("atob", lambda a, b, c: (a[0] + "と" + b[0], a[1] + "と" + b[1])),
    ("awosuru", lambda a, b, c: (a[0] + "をする", a[1] + "をする")),
    ("anobdesu", lambda a, b, c: (a[0] + "の" + b[0] + "です", a[1] + "の" + b[1] + "です")),
    ("a+b+c", lambda a, b, c: (a[0] + b[0] + c[0], a[1] + b[1] + c[1])),
]


def make(path, pool, n, seed):
    rnd = random.Random(seed)
    rows = []
    for i in range(n):
        form, f = FORMS[i % len(FORMS)]
        a, b, c = rnd.choice(pool), rnd.choice(pool), rnd.choice(pool)
        surf, rd = f(a, b, c)
        rows.append(("phr-%05d" % i, "連なり", "phr," + form, "", "", rd, surf, "", "dev", "phr"))
    with open(path, "w", encoding="utf-8") as fo:
        fo.write("id\tkind\ttags\tprofile\tcontext\treading\texpected\tnote\tsplit\tsource\n")
        for r in rows:
            fo.write("\t".join(r) + "\n")


make(OUT + "phr.tsv", nouns, 11000, 7)
kpool = [x for x in nouns if KATA.match(x[0]) and len(x[1]) >= 3]
print("katakana nouns", len(kpool), file=sys.stderr)
make(OUT + "phrk.tsv", kpool, 6000, 11)
