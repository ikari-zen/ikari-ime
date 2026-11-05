#!/usr/bin/env python3
"""ADR 0039 §2.2 / §7 の PoC: Wikipedia 日本語版の分割ダンプ 1 ファイルから、カタカナの連なりの tf・df を数える（捨てるスクリプト）。

  python3 -I webwords_collect.py --dump <jawiki-…-pages-articlesN.xml-…bz2> --out <出力 tsv> [--min-df 2]

- 標準の名前空間（ns=0）で、リダイレクトでない記事だけ。題が「の一覧」で終わるもの・曖昧さ回避は除く（名前の羅列で df が歪むため）。
- ウィキの記法は自前の正規表現で除く（テンプレート、表、脚注、コメント、HTML のタグ、ファイル・カテゴリのリンク、コード・数式の塊）。
  リンクは表示される文字だけを残す。形態素解析はしない。
- カタカナの連なり: U+30A1〜U+30FA と ー（U+30FC）。「・」「＝」で区切る（連なりに含めない）。
- 出力（タブ区切り）: 表記 tf df cue。cue は、直後に固有名詞らしい手がかり（さん、氏、社、州、市、科、属、目 など）が続いた回数（ADR 0039 F4b）。
  1 行目はヘッダ（#ikari-webwords-poc …）。文・記事の名前は出さない。
"""
import argparse
import bz2
import html
import re
import sys
from collections import Counter

RUN = re.compile("[ァ-ヺー]+")
CUES = ("さん", "氏", "社", "州", "市", "科", "属", "目", "県", "郡", "町", "村", "区", "川", "山", "島", "党", "軍", "王")
PAGE_END = b"</page>"

RE_TITLE = re.compile(r"<title>(.*?)</title>")
RE_NS = re.compile(r"<ns>(\d+)</ns>")
RE_TEXT = re.compile(r"<text[^>]*>(.*?)</text>", re.S)
RE_COMMENT = re.compile(r"<!--.*?-->", re.S)
RE_BLOCK = re.compile(
    r"<(ref|math|code|syntaxhighlight|source|pre|nowiki|gallery|score|timeline|imagemap|poem|chem|templatedata)\b[^>]*?(?:/>|>.*?</\1\s*>)",
    re.S | re.I,
)
RE_TABLE = re.compile(r"\{\|.*?\|\}", re.S)
RE_TEMPLATE = re.compile(r"\{\{[^{}]*\}\}")
RE_FILE = re.compile(r"\[\[(?:ファイル|File|Image|画像|カテゴリ|Category):[^\[\]]*(?:\[\[[^\[\]]*\]\][^\[\]]*)*\]\]", re.I)
RE_LINK_PIPE = re.compile(r"\[\[[^\[\]|]*\|([^\[\]]*)\]\]")
RE_LINK = re.compile(r"\[\[([^\[\]]*)\]\]")
RE_EXT = re.compile(r"\[(?:https?:)?//[^\s\]]+\s*([^\]]*)\]")
RE_URL = re.compile(r"https?://\S+")
RE_TAG = re.compile(r"</?[A-Za-z][^>]*>")
RE_ENTITY = re.compile(r"&[#A-Za-z0-9]+;")
RE_MAGIC = re.compile(r"__[A-Z]+__")


def strip_markup(t: str) -> str:
    t = html.unescape(t)  # XML の &lt; などを戻す（二重の &amp;lt; もここで戻る）
    t = RE_COMMENT.sub("", t)
    t = RE_BLOCK.sub("", t)
    t = RE_TABLE.sub("", t)
    for _ in range(12):
        n = RE_TEMPLATE.sub("", t)
        if n == t:
            break
        t = n
    t = RE_FILE.sub("", t)
    t = RE_LINK_PIPE.sub(r"\1", t)
    t = RE_LINK.sub(r"\1", t)
    t = RE_EXT.sub(r"\1", t)
    t = RE_URL.sub("", t)
    t = RE_TAG.sub("", t)
    t = RE_ENTITY.sub(" ", t)
    t = RE_MAGIC.sub("", t)
    return t.replace("'''", "").replace("''", "")


def pages(path):
    buf = b""
    with bz2.open(path, "rb") as f:
        while True:
            chunk = f.read(1 << 24)
            if not chunk:
                break
            buf += chunk
            while True:
                i = buf.find(PAGE_END)
                if i < 0:
                    break
                yield buf[: i + len(PAGE_END)].decode("utf-8", "replace")
                buf = buf[i + len(PAGE_END):]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dump", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--min-df", type=int, default=2)
    ap.add_argument("--limit", type=int, default=0, help="記事数の上限（試運転用）")
    a = ap.parse_args()

    tf, df, cue = Counter(), Counter(), Counter()
    articles = chars = skipped = 0
    for page in pages(a.dump):
        m = RE_NS.search(page)
        if not m or m.group(1) != "0" or "<redirect" in page:
            continue
        title = (RE_TITLE.search(page) or [None, ""])[1]
        if title.endswith("の一覧") or title.endswith("一覧"):
            skipped += 1
            continue
        m = RE_TEXT.search(page)
        if not m:
            continue
        raw = m.group(1)
        if "{{Aimai" in raw or "{{aimai" in raw or "{{Disambig" in raw or "{{曖昧さ回避" in raw:
            skipped += 1
            continue
        text = strip_markup(raw)
        articles += 1
        chars += len(text)
        seen = set()
        for mm in RUN.finditer(text):
            w = mm.group(0)
            tf[w] += 1
            seen.add(w)
            nxt = text[mm.end(): mm.end() + 2]
            if nxt.startswith(CUES) or nxt.startswith("さん"):
                cue[w] += 1
        for w in seen:
            df[w] += 1
        if a.limit and articles >= a.limit:
            break
        if articles % 20000 == 0:
            print(f"{articles} articles, {len(tf)} distinct", file=sys.stderr, flush=True)

    with open(a.out, "w", encoding="utf-8") as o:
        o.write(f"#ikari-webwords-poc\tdump={a.dump.rsplit('/', 1)[-1]}\tarticles={articles}\tchars={chars}\tskipped={skipped}\tmin_df={a.min_df}\n")
        for w in sorted(tf):
            if df[w] >= a.min_df:
                o.write(f"{w}\t{tf[w]}\t{df[w]}\t{cue[w]}\n")
    print(f"done: articles={articles} chars={chars} distinct={len(tf)} kept={sum(1 for w in tf if df[w] >= a.min_df)}", file=sys.stderr)


if __name__ == "__main__":
    main()
