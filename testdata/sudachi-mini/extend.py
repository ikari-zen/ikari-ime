#!/usr/bin/env python3
"""fixture (lex.csv) に ADR 0002 §18 の行を追加する。既存の行は変更・削除しない（追加だけ）。

使い方:
    python3 testdata/sudachi-mini/extend.py <SudachiDict の展開先ディレクトリ>

追加する行（SudachiDict Core 20260723 の実データから、表記・読み・品詞・コストを変えずに抜き出す）:
  - 表記ゆれ・ASCII・数字・人名（ポリシーのテスト用）: コウ/セイ/4/relax/hennkann/ナッタ/スズメ/ユーザ ほか
  - 活用の一族（活用規則の導出用）: 書き進める 一族ほか。活用型ごとに 3 語以上
左右ID は extract.py と同じ粗いクラスの ID（既存の行から読み取る）。matrix.def は変えない。
辞書形ID（13列目）は、この lex.csv の中の行番号（0始まり）に付け替える（基準形の行が fixture にある場合）。
既存の行の辞書形ID は元の辞書の行番号のままで、fixture の中の行を指さない（無害）。
冪等: 同じ内容の行が既にあれば追加しない。
"""
import sys
from pathlib import Path

from extract import (COL_COST, COL_LEFT, COL_READING, COL_RIGHT, COL_SURFACE,
                     coarse_class)

HERE = Path(__file__).resolve().parent
COL_LEMMA, COL_DICT_FORM, COL_CTYPE, COL_CFORM, COL_POS2 = 12, 13, 9, 10, 6

# (表記, 読み(カタカナ), 品詞1, 品詞2, lemma または None)。条件に合う最小コストの行を1つ選ぶ
PLAIN_ROWS = [
    ("コウ", "コウ", "名詞", "普通名詞", "こう"),
    ("セイ", "セイ", "名詞", "固有名詞", None),
    ("ナッタ", "ナッタ", "名詞", "固有名詞", None),
    ("4", "ヨウ", "名詞", "数詞", None),
    ("relax", "リラックス", "名詞", "固有名詞", None),
    ("hennkann", "ヘンカン", "名詞", "普通名詞", None),
    ("iPhone", "アイフォーン", "名詞", "固有名詞", None),
    ("スズメ", "スズメ", "名詞", "普通名詞", "雀"),
    ("ユーザ", "ユーザ", "名詞", "普通名詞", None),
    ("ユーザー", "ユーザー", "名詞", "普通名詞", None),
    ("ラーメン", "ラーメン", "名詞", "普通名詞", None),
    ("更生", "コウセイ", "名詞", "普通名詞", None),
    ("品詞", "ヒンシ", "名詞", "普通名詞", None),
]

# 活用の一族: 基準形（終止形）の元の行番号。その辞書形ID を指す行のうち WANTED_FORMS を取る
FAMILY_BASES = [
    # 下一段-マ行: 書き進める 一族
    506987, 684274, 337033, 666523, 717726,
    # 五段-ラ行（一般）
    278210, 335068, 695737, 762176, 113407,
    # 五段-ラ行（非自立可能）: なる/なっ ほか
    113411, 617535, 12715, 34384,
]
WANTED_FORMS = {"終止形-一般", "連体形-一般", "連用形-一般", "連用形-促音便", "未然形-一般"}


def load_raw(raw_dir):
    rows = []
    for name in ("small_lex.csv", "core_lex.csv"):
        with open(raw_dir / name, encoding="utf-8") as f:
            for line in f:
                rows.append(line.rstrip("\n").split(","))
    return rows


def class_ids(fixture_rows):
    ids = {}
    for c in fixture_rows:
        if c[COL_LEFT] == "-1":
            continue
        name = coarse_class(c)
        if ids.setdefault(name, c[COL_LEFT]) != c[COL_LEFT]:
            raise SystemExit(f"class {name} has two ids")
    return ids


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    raw = load_raw(Path(sys.argv[1]))
    path = HERE / "lex.csv"
    existing = path.read_text(encoding="utf-8").splitlines()
    fixture_rows = [l.split(",") for l in existing]
    ids = class_ids(fixture_rows)
    have = set(existing)

    def remap(c):
        c = list(c)
        name = coarse_class(c)
        if name not in ids:
            raise SystemExit(f"no class id for {name}: {','.join(c)}")
        c[COL_LEFT] = c[COL_RIGHT] = ids[name]
        return c

    new_rows = []  # (元の行番号 または None, cols)

    for surface, reading, p1, p2, lemma in PLAIN_ROWS:
        cands = [
            (int(c[COL_COST]), i, c) for i, c in enumerate(raw)
            if c[COL_SURFACE] == surface and c[COL_READING] == reading and c[5] == p1
            and c[COL_POS2] == p2 and c[COL_LEFT] != "-1" and c[COL_CTYPE] == "*"
            and (lemma is None or c[COL_LEMMA] == lemma)
        ]
        if not cands:
            raise SystemExit(f"not found: {surface} {reading} {p1} {p2} {lemma}")
        cost, i, c = min(cands, key=lambda t: (t[0], t[1]))
        c = remap(c)
        c[COL_DICT_FORM] = "*"
        new_rows.append((i, c))

    for base in FAMILY_BASES:
        b = raw[base]
        if b[COL_CFORM] != "終止形-一般":
            raise SystemExit(f"base {base} is not a terminal form")
        fam = [(i, c) for i, c in enumerate(raw)
               if c[COL_DICT_FORM] == str(base) and c[COL_CFORM] in WANTED_FORMS
               and c[COL_LEFT] != "-1"]
        if not any(i == base for i, _ in fam):
            fam.append((base, b))
        for i, c in sorted(fam):
            new_rows.append((i, remap(c)))

    # 行番号を決める（既存の行の後ろに追加）。基準形の行が fixture にあれば付け替える
    local = {}
    out = list(existing)
    pending = []
    for orig, c in new_rows:
        pending.append((orig, c))
    # 先に行番号を割り当てる
    next_index = len(existing)
    plan = []
    for orig, c in pending:
        line_wo_dict = ",".join(c[:COL_DICT_FORM] + ["@"] + c[COL_DICT_FORM + 1:])
        key = (orig, line_wo_dict)
        if any(k == key for k, _ in plan):
            continue
        duplicate_existing = any(
            ",".join(e[:COL_DICT_FORM] + ["@"] + e[COL_DICT_FORM + 1:]) == line_wo_dict
            for e in fixture_rows)
        if duplicate_existing:
            continue
        plan.append((key, next_index))
        if orig is not None:
            local[orig] = next_index
        next_index += 1
    by_key = dict(plan)
    for orig, c in pending:
        line_wo_dict = ",".join(c[:COL_DICT_FORM] + ["@"] + c[COL_DICT_FORM + 1:])
        key = (orig, line_wo_dict)
        if key not in by_key:
            continue
        c = list(c)
        if c[COL_DICT_FORM] != "*":
            target = int(c[COL_DICT_FORM])
            c[COL_DICT_FORM] = str(local[target]) if target in local else "*"
        line = ",".join(c)
        if line in have:
            continue
        have.add(line)
        out.append(line)
    path.write_text("\n".join(out) + "\n", encoding="utf-8")
    print(f"rows: {len(existing)} -> {len(out)}")


if __name__ == "__main__":
    main()
