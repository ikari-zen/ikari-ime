#!/usr/bin/env python3
"""SudachiDict Core 20260723 から fixture (lex.csv, matrix.def) を作る。

使い方:
    python3 testdata/sudachi-mini/extract.py <SudachiDict の展開先ディレクトリ>

<dir> には small_lex.csv, core_lex.csv, matrix.def が必要 (data/raw/sudachi-20260723/)。
標準ライブラリのみ。出力は決定的 (同じ入力から同じ lex.csv と matrix.def)。

手順:
  1. SELECTIONS の規則で、評価セット(ADR 0001 §6.1)の読みに関わる行を抜き出す。
  2. 行の品詞を粗い「クラス」にまとめ、クラスごとに新しい左右 ID (0 = BOS/EOS, 1.. = クラス) を振る。
     左 ID と右 ID は同じクラス ID を使う。表記・読み・品詞・コストは実データのまま。
  3. matrix.def は新 ID 空間の全組み合わせ。各クラスの代表の元 ID (そのクラスの行で最も多い元の右 ID /
     左 ID) を使って、実際の matrix.def の値を写す (連接コストの大まかな傾向を残すため)。
"""
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent

COL_LEFT, COL_RIGHT, COL_COST, COL_SURFACE = 1, 2, 3, 4
COL_POS_FIRST, COL_POS_LAST = 5, 10  # 5..=10 (品詞1..4, 活用型, 活用形)
COL_READING = 11

# (読み(カタカナ), 最大行数, 必ず含める (表記, 品詞1) の集合。空なら上位コスト順に品詞の違うものを優先)
SELECTIONS = [
    ("ヒンシ", 20, {"品詞", "瀕死", "浜死", "賓師", "ひん死", "ひんし", "瀕し"}),
    ("コウセイ", 8, {"構成", "校正", "更正", "厚生", "公正", "後世"}),
    ("カイトウ", 8, {"回答", "解答", "解凍", "怪盗", "快投"}),
    ("シヨウ", 14, {"仕様", "使用", "私用", "試用", "しよう", "為よう", "仕よう"}),
    ("ノ", 6, set()),
    ("コノ", 4, set()),
    ("ハ", 5, set()),
    ("ニ", 5, set()),
    ("ト", 5, set()),
    ("ガ", 3, set()),
    ("ヲ", 3, set()),
    ("ナッ", 10, {"成っ", "鳴っ", "為っ", "なっ"}),
    ("タ", 4, set()),
    ("カキ", 14, {"書き", "書", "かき", "柿", "牡蠣"}),
    ("ススメル", 8, {"進める", "すすめる", "勧める"}),
    ("ススメ", 5, set()),
    ("ヘンカン", 6, {"変換", "返還", "へんかん"}),
    ("スル", 10, {"する", "為る"}),
    ("シ", 4, set()),
    ("テン", 3, {"点"}),
]
# 除外フィルタの確認用に、実データから少数の行を追加で抜き出す
EXTRA_FILTER_ROWS = [
    # 補助記号 / 空白 (builder が除外する)
    lambda c: c[COL_SURFACE] == "、" and c[5] == "補助記号",
    lambda c: c[COL_SURFACE] == " " and c[5] == "空白",
    # 左 ID = -1 (分割要素専用。builder が除外する)
    lambda c: c[COL_LEFT] == "-1" and c[COL_READING] == "カキ" and c[COL_SURFACE] == "堵墻",
    # 読みに英字を含む (索引に入らない)
    lambda c: c[COL_SURFACE] == "profile" and c[COL_READING] == "profile",
]


def pos_key(c):
    return "/".join(c[COL_POS_FIRST : COL_POS_LAST + 1])


def coarse_class(c):
    """品詞から粗いクラス名を決める。"""
    p1, p2, ctype, cform = c[5], c[6], c[9], c[10]
    if p1 == "名詞":
        if p2 in ("普通名詞", "固有名詞"):
            if p2 == "普通名詞" and c[7] == "サ変可能":
                return "名詞-サ変可能"
            return "名詞-" + p2
        return "名詞-その他"
    if p1 == "動詞":
        for key, name in (("未然", "未然"), ("連用", "連用"), ("終止", "終止連体"),
                          ("連体", "終止連体"), ("意志", "意志"), ("仮定", "仮定"), ("命令", "命令")):
            if cform.startswith(key):
                return "動詞-" + name
        return "動詞-その他"
    if p1 == "助詞":
        return "助詞-" + (p2 if p2 in ("格助詞", "係助詞", "接続助詞") else "その他")
    return p1  # 助動詞, 連体詞, 副詞, 接尾辞, 感動詞, 補助記号, 空白, 記号 ...


def load_rows(raw_dir):
    rows = []
    for name in ("small_lex.csv", "core_lex.csv"):
        with open(raw_dir / name, encoding="utf-8") as f:
            for lineno, line in enumerate(f, 1):
                c = line.rstrip("\n").split(",")
                if len(c) != 19:
                    raise SystemExit(f"{name}:{lineno}: 19 columns expected")
                rows.append((name, lineno, c))
    return rows


def select(rows):
    chosen = {}  # (name, lineno) -> cols
    for reading, cap, must in SELECTIONS:
        cand = [(int(c[COL_COST]), n, l, c) for n, l, c in rows
                if c[COL_READING] == reading and c[COL_LEFT] != "-1"
                and not c[COL_SURFACE].isascii()]
        cand.sort(key=lambda t: (t[0], t[1], t[2]))
        picked, seen_pos = [], set()
        # 必須の表記を先に (同じ (表記, 品詞) は最小コストの1行)
        seen_must = set()
        for cost, n, l, c in cand:
            k = (c[COL_SURFACE], pos_key(c))
            if c[COL_SURFACE] in must and k not in seen_must and len(picked) < cap:
                seen_must.add(k)
                picked.append((n, l, c))
        # 残りは品詞の違いを優先してコスト順
        for cost, n, l, c in cand:
            if len(picked) >= cap:
                break
            if (n, l) in {(p[0], p[1]) for p in picked}:
                continue
            k = pos_key(c)
            if k in seen_pos:
                continue
            seen_pos.add(k)
            picked.append((n, l, c))
        for n, l, c in picked:
            chosen[(n, l)] = c
    for i, pred in enumerate(EXTRA_FILTER_ROWS):
        for n, l, c in rows:
            if pred(c):
                chosen[(n, l)] = c
                break
        else:
            raise SystemExit(f"extra row #{i} not found")
    return [chosen[k] for k in sorted(chosen)]


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    raw_dir = Path(sys.argv[1])
    rows = load_rows(raw_dir)
    selected = select(rows)

    classes = sorted({coarse_class(c) for c in selected if c[COL_LEFT] != "-1"})
    class_id = {name: i + 1 for i, name in enumerate(classes)}  # 0 は BOS/EOS
    size = len(classes) + 1

    # クラスごとの代表の元 ID
    lefts, rights = {}, {}
    for c in selected:
        if c[COL_LEFT] == "-1":
            continue
        k = class_id[coarse_class(c)]
        lefts.setdefault(k, Counter())[int(c[COL_LEFT])] += 1
        rights.setdefault(k, Counter())[int(c[COL_RIGHT])] += 1
    rep_left = {0: 0}
    rep_right = {0: 0}
    for k in class_id.values():
        rep_left[k] = sorted(lefts[k].items(), key=lambda t: (-t[1], t[0]))[0][0]
        rep_right[k] = sorted(rights[k].items(), key=lambda t: (-t[1], t[0]))[0][0]

    # lex.csv (左 ID = -1 の行はそのまま -1)
    out = []
    for c in selected:
        c = list(c)
        if c[COL_LEFT] != "-1":
            k = class_id[coarse_class(c)]
            c[COL_LEFT] = c[COL_RIGHT] = str(k)
        out.append(",".join(c))
    (HERE / "lex.csv").write_text("\n".join(out) + "\n", encoding="utf-8")

    # 実際の matrix.def から必要な (右 ID, 左 ID) の値だけを写す
    need = {(rep_right[a], rep_left[b]) for a in range(size) for b in range(size)}
    real = {}
    with open(raw_dir / "matrix.def", encoding="utf-8") as f:
        next(f)  # 先頭行はサイズ
        for line in f:
            a, b, cost = line.split()
            key = (int(a), int(b))
            if key in need:
                real[key] = int(cost)
    lines = [f"{size} {size}"]
    for a in range(size):
        for b in range(size):
            lines.append(f"{a} {b} {real[(rep_right[a], rep_left[b])]}")
    (HERE / "matrix.def").write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"rows={len(selected)} classes={size}")
    for name, k in sorted(class_id.items(), key=lambda t: t[1]):
        print(k, name, "orig_left", rep_left[k], "orig_right", rep_right[k])


if __name__ == "__main__":
    main()
