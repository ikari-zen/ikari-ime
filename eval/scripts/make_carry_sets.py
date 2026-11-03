#!/usr/bin/env python3
"""homophones-ext から、左文脈の持ち越し（ADR 0046 §7.1）の派生セット C1〜C5 を作る。

各セットは `field`（available）と `history`（同じセッションの確定の表記の連結）の列を足したもの。
`context` は入力欄の左。`ikari-eval --carry-fill N --carry-joint J` で左文脈を組み立てて流す。
決定的（乱数を使わない）。元のセットを替えたら作り直す。

  C1 carry-empty            context=空、history=元の context（送信で空になった入力欄。区切りの最悪の場合）
  C2 carry-overlap          context=元の context、history=関係のある前の文 + 元の context
  C3 carry-unrelated        context=元の context、history=無関係な約 150 字 + 元の context（害）
  C4 carry-mismatch         context=元の context、history=別のケースの文（context で終わらない。不変条件）
  C5 carry-empty-unrelated  context=空、history=無関係な約 150 字（同じアプリの無関係な前の文だけ）

「関係のある前の文」は、同じ expected を持つ別のケースの context を「。」でつないだもの（同じ話題の断片）。
「無関係」は、expected が違う別のケースの context を、文として長いものから「。」でつないだもの。
どちらも元のセットから作るので、チャットの形（前のメッセージが完結した文）は carry-chat.tsv（手書き）で補う。

使い方:
  eval/scripts/make_carry_sets.py [--src eval/sets/homophones-ext.tsv] [--out-dir eval/sets]
"""
import argparse
import os
import sys

SENTENCE_END = "。"
# 句点でつなぐときに、すでに文末として終わっている印
END_MARKS = "。．.！？!?」』）)】"
UNRELATED_CHARS = 150
RELATED_MAX = 2
FIELD_AVAILABLE = "available"
NEW_COLS = ["field", "history"]

SETS = [
    ("carry-empty", "C1", "context を空にし、元の context を history に置く（送信で空になった入力欄）"),
    ("carry-overlap", "C2", "context は元のまま、history を「関係のある前の文 + 元の context」にする（重なりの除去）"),
    ("carry-unrelated", "C3", "context は元のまま、history を「無関係な約 150 字 + 元の context」にする（害）"),
    ("carry-mismatch", "C4", "context は元のまま、history を別のケースの文にする（context で終わらない。不変条件: homophones-ext と 1 件も違わない）"),
    ("carry-empty-unrelated", "C5", "context を空にし、history を無関係な約 150 字にする（--carry-fill 0 は文脈なしと同じ）"),
]


def sentence(text):
    """断片を文として閉じる（文末の印がなければ「。」を足す）"""
    return text if text and text[-1] in END_MARKS else text + SENTENCE_END


def load(path):
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    head, header, rows = [], None, []
    for line in lines:
        if line.startswith("#") or not line.strip():
            if header is None:
                head.append(line)
            continue
        cells = line.split("\t")
        if header is None:
            header = cells
        else:
            cells += [""] * (len(header) - len(cells))
            rows.append(cells)
    return head, header, rows


def build(rows, ctx_i, exp_i):
    """ケースごとの (関係のある前の文, 無関係な文, 別のケースの文) を決める"""
    by_expected = {}
    for r in rows:
        by_expected.setdefault(r[exp_i], []).append(r)
    # 無関係な文の材料: 長い context から順（同じ長さは出現順）
    pool = sorted(range(len(rows)), key=lambda i: (-len(rows[i][ctx_i]), i))
    out = []
    for n, r in enumerate(rows):
        same = [o[ctx_i] for o in by_expected[r[exp_i]] if o is not r and o[ctx_i] != r[ctx_i]]
        same.sort(key=lambda s: (-len(s), s))
        related = "".join(sentence(s) for s in same[:RELATED_MAX])
        # 無関係: expected が違う別のケースの context。ケースごとに開始位置をずらす
        unrelated, k = "", 0
        while len(unrelated) < UNRELATED_CHARS and k < len(pool):
            o = rows[pool[(n * 7 + k) % len(pool)]]
            k += 1
            if o[exp_i] == r[exp_i]:
                continue
            unrelated += sentence(o[ctx_i])
        # 別のケースの文（context の末尾と一致しない）
        mismatch = ""
        for k in range(len(pool)):
            o = rows[pool[(n * 13 + k) % len(pool)]]
            cand = sentence(o[ctx_i])
            if o[exp_i] != r[exp_i] and not cand.endswith(r[ctx_i]):
                mismatch = cand
                break
        out.append((related, unrelated, mismatch))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default="eval/sets/homophones-ext.tsv")
    ap.add_argument("--out-dir", default="eval/sets")
    args = ap.parse_args()

    head, header, rows = load(args.src)
    ctx_i, exp_i = header.index("context"), header.index("expected")
    extra = build(rows, ctx_i, exp_i)
    out_header = header + [c for c in NEW_COLS if c not in header]
    src_name = os.path.basename(args.src)

    for name, label, what in SETS:
        path = os.path.join(args.out_dir, name + ".tsv")
        with open(path, "w", encoding="utf-8") as f:
            f.write(f"# {label} {name}: {src_name} の派生（eval/scripts/make_carry_sets.py。ADR 0046 §7.1）。{what}\n")
            f.write("# 手で直さない。元のセットを替えたら作り直す。\n")
            f.write("\t".join(out_header) + "\n")
            for r, (related, unrelated, mismatch) in zip(rows, extra):
                original = r[ctx_i]
                if label == "C1":
                    ctx, hist = "", original
                elif label == "C2":
                    # 関係のある前の文がないケースは、元の context だけ（重なりのみ）
                    ctx, hist = original, related + original
                elif label == "C3":
                    ctx, hist = original, unrelated + original
                elif label == "C4":
                    ctx, hist = original, mismatch
                else:
                    ctx, hist = "", unrelated
                cells = list(r)
                cells[ctx_i] = ctx
                cells += [FIELD_AVAILABLE, hist]
                f.write("\t".join(cells) + "\n")
        print(path, file=sys.stderr)


if __name__ == "__main__":
    main()
