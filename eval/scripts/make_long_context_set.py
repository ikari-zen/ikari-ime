#!/usr/bin/env python3
"""評価セットの左文脈を、約 N 文字に伸ばした評価セットを作る（ADR 0002 §17.4: 2000 文字の文脈での遅延の確認用）。

元のセットの context の前に、テキストファイルから切り出した文を足す。ケースごとに別の領域を使うので、
解析器の文ごとのキャッシュが効かない（最悪に近い）ケースを作れる。領域が足りなければ最初に戻る。

使い方:
  eval/scripts/make_long_context_set.py eval/sets/homophones.tsv --text docs/adr/0002-phase1-architecture.md \\
      --chars 2000 > /tmp/long.tsv
"""
import argparse
import re
import sys

SENTENCE_END = "。"
KANA_FIRST = 0x3042  # あ
KANA_COUNT = 70


def sentences(paths):
    out = []
    for path in paths:
        with open(path, encoding="utf-8") as f:
            text = f.read()
        # Markdown の記号や表の線を除き、日本語を含む文だけ使う
        text = re.sub(r"[|`#>*\-]+", " ", text)
        text = re.sub(r"\s+", "", text)
        for s in text.split(SENTENCE_END):
            japanese = len(re.findall(r"[ぁ-んァ-ヶ一-龥ー、]", s))
            # コードや表の断片を避けるため、日本語が 8 割以上の文だけ使う
            if s and japanese >= 0.8 * len(s):
                out.append(s + SENTENCE_END)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("set", help="元の評価セット（TSV）")
    ap.add_argument("--text", action="append", required=True, help="文脈の材料にするテキスト（複数可）")
    ap.add_argument("--chars", type=int, default=2000, help="文脈の文字数（既定 2000）")
    args = ap.parse_args()

    pool = sentences(args.text)
    if not pool:
        sys.exit("no sentences in --text")
    pos = 0
    case_no = 0
    with open(args.set, encoding="utf-8") as f:
        lines = f.read().splitlines()
    header = None
    ctx_idx = None
    for line in lines:
        if line.startswith("#") or not line.strip():
            print(line)
            continue
        cells = line.split("\t")
        if header is None:
            header = cells
            ctx_idx = header.index("context")
            print(line)
            continue
        original = cells[ctx_idx]
        need = max(0, args.chars - len(original))
        filler = ""
        while len(filler) < need:
            lap = pos // len(pool)
            # 2周目以降は、同じ文をそのまま使わない（周とケースで異なるかな1字を足す）
            prefix = chr(KANA_FIRST + (lap * 7 + case_no) % KANA_COUNT) if lap else ""
            filler += prefix + pool[pos % len(pool)]
            pos += 1
        case_no += 1
        # 先頭から切ると文の途中から始まるので、末尾 need 文字を使う
        cells[ctx_idx] = filler[-need:] + original if need else original
        print("\t".join(cells))


if __name__ == "__main__":
    main()
