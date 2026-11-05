"""ADR 0039 PoC の共有部品（捨てるスクリプト）: 一覧の読み込み、Sudachi の読み込み、絞り（F1〜F5）、lex.csv の行。"""
import csv
import os
import sys
from collections import Counter

LEX_NAMES = ["small_lex.csv", "core_lex.csv", "notcore_lex.csv"]
SMALL_KANA = set("ァィゥェォッャュョヮ")
BAD_CHARS = set("ヵヶ" + "".join(chr(c) for c in range(0x30F7, 0x30FB)))
COMMON = ("名詞", "普通名詞")
PROPER = "固有名詞"
# F4b: 直後の手がかり（webwords_collect.py の CUES）が付いた割合の閾値の既定
DEFAULT_CUE_RATIO = 0.2


def is_kata_run(s):
    return bool(s) and all("ァ" <= c <= "ヺ" or c == "ー" for c in s)


def to_hira(s):
    return "".join(chr(ord(c) - 0x60) if "ァ" <= c <= "ヶ" else c for c in s)


def read_list(path):
    """-> (header, {表記: [tf, df, cue]})"""
    words = {}
    header = ""
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#"):
                header = line.strip()
                continue
            w, tf, df, cue = line.rstrip("\n").split("\t")
            words[w] = [int(tf), int(df), int(cue)]
    return header, words


def merge(lists):
    out = {}
    for words in lists:
        for w, (tf, df, cue) in words.items():
            e = out.get(w)
            if e is None:
                out[w] = [tf, df, cue]
            else:
                e[0] += tf
                e[1] += df
                e[2] += cue
    return out


class Sudachi:
    """small + core + notcore の表記ごとの品詞の集まり"""

    def __init__(self, dirpath):
        self.all = set()  # F1: 品詞によらない表記の集合（カタカナの連なりだけ持つ）
        self.common = set()  # 普通名詞の entry があるカタカナ表記
        self.proper_only = set()  # 固有名詞の entry だけのカタカナ表記
        self.hidden = set()  # 再現率の対象: 普通名詞・表記も読みもカタカナ・3 字以上
        self.ids = Counter()  # 名詞,普通名詞,一般 でカタカナだけの行の (左, 右) の頻度
        proper, nonproper = set(), set()
        for name in LEX_NAMES:
            with open(os.path.join(dirpath, name), encoding="utf-8", newline="") as f:
                for line in f:
                    row = next(csv.reader([line]))
                    if len(row) < 17:
                        continue
                    s = row[0]
                    if not is_kata_run(s):
                        continue
                    self.all.add(s)
                    pos = row[5:7]
                    if row[6] == PROPER:
                        proper.add(s)
                    else:
                        nonproper.add(s)
                    if tuple(pos) == COMMON:
                        self.common.add(s)
                        if is_kata_run(row[11]) and len(s) >= 3:
                            self.hidden.add(s)
                        if row[5:9] == ["名詞", "普通名詞", "一般", "*"]:
                            self.ids[(row[1], row[2])] += 1
        self.proper_only = proper - nonproper

    def common_ids(self):
        return self.ids.most_common(1)[0][0]


def segmentable(w, parts, min_part=2):
    """w を parts の語 2 個以上に分けられるか（全体 1 語は数えない）"""
    n = len(w)
    ok = [False] * (n + 1)
    ok[0] = True
    for i in range(min_part, n + 1):
        for j in range(0, i - min_part + 1):
            if ok[j] and w[j:i] in parts and not (j == 0 and i == n):
                ok[i] = True
                break
    return ok[n]


def is_reduplication(r):
    n = len(r)
    if n >= 4 and n % 2 == 0 and r[: n // 2] == r[n // 2:]:
        return True
    return n >= 3 and len(set(r)) == 1


def f3_reject(w, min_len):
    """F3 形の規則。理由を返す（通れば None）"""
    if len(w) < min_len:
        return "short"
    if w[0] == "ー" or w[0] in SMALL_KANA:
        return "lead"
    if any(c in BAD_CHARS for c in w):
        return "badchar"
    if is_reduplication(w):
        return "redup"
    return None


def filter_words(words, sud, min_df, min_len, cue_ratio=DEFAULT_CUE_RATIO, f4c=True, f5=False, apply_f1=True, min_tf=0, stats=None):
    """-> [(w, tf, df, cue)]。stats が渡されたら、理由ごとの除いた数を入れる"""
    st = stats if stats is not None else Counter()
    out = []
    for w, (tf, df, cue) in words.items():
        if df < min_df or tf < min_tf:
            continue
        st["after_f2"] += 1
        if apply_f1 and w in sud.all:
            st["f1_in_sudachi"] += 1
            continue
        r = f3_reject(w, min_len)
        if r:
            st["f3_" + r] += 1
            continue
        if cue_ratio and cue / tf >= cue_ratio:
            st["f4b_cue"] += 1
            continue
        if f4c and segmentable(w, sud.proper_only):
            st["f4c_proper_parts"] += 1
            continue
        if f5 and segmentable(w, sud.common):
            st["f5_known_parts"] += 1
            continue
        st["kept"] += 1
        out.append((w, tf, df, cue))
    return out


def lex_row(w, left, right, cost):
    # Sudachi の lex.csv と同じ 19 列（ABR の town_lex.csv と同じ形）。品詞 名詞,普通名詞,一般
    return f"{w},{left},{right},{cost},{w},名詞,普通名詞,一般,*,*,*,{w},{w},*,A,*,*,*,*\n"
