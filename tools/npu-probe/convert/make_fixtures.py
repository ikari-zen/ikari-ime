"""測定の 40 件の選び出しと、fixtures の組み立て（ADR 0075 §4.2。スパイク）。

使い方:
  python -I make_fixtures.py select <homophones-ext.tsv> <out-sub40.tsv>
  python -I make_fixtures.py build  <sub40.tsv> <dump.jsonl> <out-fixtures.jsonl>

select: 文脈の長さの順に並べ、等間隔に 40 件を取る（同じ長さは id の順）。
build:  dump-nll.patch を当てた ikari-eval の出力（nll 1 回 = 1 行）から、
        件ごとに「読み全体の、最初の採点」を取り出し、id を付けて書く。
"""

import json
import sys

N_CASES = 40
COL_ID, COL_CONTEXT, COL_READING = 0, 4, 5
# lm/src/prompt.rs のタグ
TAG_INPUT, TAG_OUTPUT, TAG_CONTEXT, TAG_RIGHT = "", "", "", ""
HIRAGANA_FIRST, HIRAGANA_LAST, KATAKANA_OFFSET = 0x3041, 0x3096, 0x60


def to_katakana(s: str) -> str:
    return "".join(
        chr(ord(c) + KATAKANA_OFFSET) if HIRAGANA_FIRST <= ord(c) <= HIRAGANA_LAST else c
        for c in s
    )


def left_context(prompt: str) -> str:
    """プロンプトの左文脈（TAG_CONTEXT の後ろ、次のタグの前）。末尾で切られているので、元の文脈の接尾辞になる"""
    if TAG_CONTEXT not in prompt:
        return ""
    rest = prompt.split(TAG_CONTEXT, 1)[1]
    ends = [i for i, ch in enumerate(rest) if ch in (TAG_RIGHT, TAG_INPUT)]
    return rest[: ends[0]] if ends else rest


def read_tsv(path):
    lines = open(path, encoding="utf-8").read().splitlines()
    comments = [line for line in lines if line.startswith("#")]
    body = [line for line in lines if line and not line.startswith("#")]
    return comments, body[0], [line.split("\t") for line in body[1:]]


def select(src, dst):
    comments, header, rows = read_tsv(src)
    rows.sort(key=lambda r: (len(r[COL_CONTEXT]), r[COL_ID]))
    n = len(rows)
    if n < N_CASES:
        sys.exit(f"rows={n} < {N_CASES}")
    picked = [rows[round(i * (n - 1) / (N_CASES - 1))] for i in range(N_CASES)]
    with open(dst, "w", encoding="utf-8") as f:
        f.write("\n".join(comments + [header] + ["\t".join(r) for r in picked]) + "\n")


def build(sub, dump, dst):
    _, _, rows = read_tsv(sub)
    calls = [json.loads(line) for line in open(dump, encoding="utf-8") if line.strip()]
    out = []
    for r in rows:
        tail = TAG_INPUT + to_katakana(r[COL_READING]) + TAG_OUTPUT
        hit = next(
            (
                c
                for c in calls
                if c["prompt"].endswith(tail) and r[COL_CONTEXT].endswith(left_context(c["prompt"]))
            ),
            None,
        )
        if hit is None:
            sys.exit(f"no LM call for {r[COL_ID]}")
        out.append({"id": r[COL_ID], **hit})
    with open(dst, "w", encoding="utf-8") as f:
        for o in out:
            f.write(json.dumps(o, ensure_ascii=False) + "\n")
    lens = [len(o["prompt_tokens"]) + max(len(t) for t in o["candidate_tokens"]) for o in out]
    ncand = [len(o["candidates"]) for o in out]
    print(f"cases={len(out)} max(P+n)={max(lens)} candidates: min={min(ncand)} max={max(ncand)} total={sum(ncand)}")


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "select":
        select(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 5 and sys.argv[1] == "build":
        build(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
