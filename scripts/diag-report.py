#!/usr/bin/env python3
"""診断ログ（engine.log）の集計。ADR 0034 §4・§5。

Python 3 の標準ライブラリだけ。`python3 -I scripts/diag-report.py` で動く。ファイルには書かない。
ログには文字（読み、候補、周辺テキスト）が入っていない。このスクリプトも文字を扱わない。

使い方:
  scripts/diag-report.py DIR                      # DIR の engine.log、engine.log.N、engine-*.log(.gz)
  scripts/diag-report.py a.log b.log.gz           # ファイルを直接
  scripts/diag-report.py --tsv DIR                # TSV（節名 TAB 項目 …）
  scripts/diag-report.py --since 2026-10-20 DIR   # UTC の日付で絞る
  scripts/diag-report.py --include-smoke DIR      # smoke のセッションも数える（既定は除く）
  scripts/diag-report.py ~/Library/Application\\ Support/ikari-ime/logs   # 手元のログ（読み取りだけ）

行の形: `{UTCミリ秒} {kind} {所要}us lm={状態} err={種類} k=v k=v …`
知らない kind とキーは無視する（古い・新しいログが混ざっても止まらない）。
"""

import argparse
import bisect
import collections
import datetime
import gzip
import os
import re
import sys

# 回した世代の名前（現行の engine.log.N と、ADR 0034 §3.2 の engine-YYYYMMDD-HHMMSS.log[.gz]）
LOG_NAME = re.compile(r"^engine(\.log(\.\d+)?(\.gz)?|-\d{8}-\d{6}\.log(\.gz)?)$")
HEAD = re.compile(r"^(\d{10,}) ([a-z_0-9]+) (\d+)us (.*)$")
PICK_BUCKETS = 9  # pick は 0〜8 と 9+
SPACES_BUCKETS = 4  # spaces は 0 / 1 / 2 / 3 / 4+
CONVERT_PERCENTILES = (50, 95, 99)
DISTURBED_ENDS = ("commit_composition", "deactivate", "mode_switch", "click")
COMMIT_ENDS = ("commit_cand", "commit_segments")
MS_PER_DAY = 86_400_000
SMOKE_APP_SUFFIX = ".inputmethod.ikari.smoke"  # 頭に依存しない（ADR 0036 §3.1）
SHUTDOWN_REASON = "shutdown"
LM_SWAP_REASONS = ("idle", "settings")  # LM を入れ替えただけ（エンジンは替わらない）
# UNKNOWN_SESSION の行（session= があるもの）の、その番号の状態
SSTATE_NO_BEGIN = "このエンジンの起動で begin_session なし"
SSTATE_ENDED = "end_session のあと"
SSTATE_LIVE = "begin_session のあとで end_session なし"
# UNKNOWN_SESSION の行から、直前の出来事までの時間の区切り（ms）。(上限, 名前)
GAP_BUCKETS = ((1_000, "1 秒未満"), (10_000, "10 秒未満"), (60_000, "1 分未満"), (600_000, "10 分未満"))
GAP_LAST = "10 分以上"
GAP_NONE = "なし"
BURST_GAP_MS = 5_000  # この間隔より空いたら別のかたまり


class Event:
    __slots__ = ("ms", "kind", "us", "lm", "err", "kv", "epoch", "smoke", "sstate")

    def __init__(self, ms, kind, us, lm, err, kv):
        self.ms, self.kind, self.us, self.lm, self.err, self.kv = ms, kind, us, lm, err, kv
        self.epoch = 0  # エンジンの起動（hello の並びか shutdown で区切る）の通し番号
        self.sstate = None  # UNKNOWN_SESSION の行の session= の状態（SSTATE_*）
        self.smoke = False  # smoke のセッション（または smoke だけのエンジンの起動）の行

    def num(self, key, default=0):
        try:
            return int(self.kv[key])
        except (KeyError, ValueError):
            return default


def parse_line(line):
    """1 行を Event にする。形が違えば None。"""
    m = HEAD.match(line.rstrip("\n"))
    if not m:
        return None
    kv = {}
    for token in m.group(4).split(" "):
        key, eq, value = token.partition("=")
        if eq:
            kv[key] = value
    lm = kv.pop("lm", "-")
    err = kv.pop("err", "-")
    return Event(int(m.group(1)), m.group(2), int(m.group(3)), lm, err, kv)


def expand_inputs(paths):
    files = []
    for path in paths:
        if os.path.isdir(path):
            files += [os.path.join(path, n) for n in sorted(os.listdir(path)) if LOG_NAME.match(n)]
        else:
            files.append(path)
    return files


def read_events(files):
    """(events, malformed 行数)。時刻順（同時刻は読んだ順）。"""
    events, malformed = [], 0
    for path in files:
        opener = gzip.open if path.endswith(".gz") else open
        with opener(path, "rt", encoding="utf-8", errors="replace") as f:
            for line in f:
                if not line.strip():
                    continue
                ev = parse_line(line)
                if ev is None:
                    malformed += 1
                else:
                    events.append(ev)
    events.sort(key=lambda e: e.ms)  # 安定ソート
    return events, malformed


def day_of(ms):
    return datetime.datetime.fromtimestamp(ms // 1000, datetime.timezone.utc).strftime("%Y-%m-%d")


def percentile(sorted_values, p):
    """最近順位法。空なら None。"""
    if not sorted_values:
        return None
    rank = max(1, -(-len(sorted_values) * p // 100))
    return sorted_values[rank - 1]


def pct(n, d):
    return "-" if d == 0 else f"{100.0 * n / d:.1f}%"


def fmt(v):
    return "-" if v is None else str(v)


# ---------------------------------------------------------------- セッションの区切り


def split_epochs(events):
    """各行に epoch を付け、smoke の行に印を付ける。(epoch の開始 ms のリスト, セッション数, smoke のセッション数) を返す。

    epoch = ログから分かる「エンジンの起動」の区切り。次の行の手前で区切る。
    - `lm_unload reason=shutdown`（エンジンを閉じた）の次の行。
    - lm が loading でない行から loading の行に変わったとき。ただし直前に `lm_unload reason=idle|settings`
      があれば、LM の入れ替えなので区切らない（起動した直後の LM は loading のため）。
    `hello` は区切りに使えない: 殻が入力モードを有効にするたびに送る（実機のログで 2000 回以上、
    直前は emoji_table / begin_session）ので、エンジンの起動とは対応しない。
    `session=` の番号は殻（IME のプロセス）が採番し、プロセスが替わると 1 から数え直す。
    セッションは (epoch, session=) の組で区別し、さらに次の場合は新しいセッションとして数える。
    - `end_session session=N`（D5 以降の行）のあとの begin_session
    - `app=` が替わる begin_session（end_session に session= がない古い行のための代わり）
    同じ app の begin_session は secure と文脈共有の更新で、続きとして扱う。
    smoke の epoch = begin_session が 1 つ以上あり、すべて app が smoke。その epoch の行は session= がなくても smoke。
    """
    epoch, starts = 0, []
    pending = True
    prev_lm, lm_swapped = None, False
    live = {}  # (epoch, session) -> [通し番号, app, 終わったか, begin_session を見たか]
    instances = 0
    smoke_sessions = set()
    begins = collections.defaultdict(lambda: [0, 0])  # epoch -> [smoke の begin 数, それ以外]
    for e in events:
        if e.lm == "loading" and prev_lm not in (None, "loading") and not lm_swapped:
            pending = True
        if e.lm != "loading":
            lm_swapped = False
        if e.kind == "lm_unload" and e.kv.get("reason") in LM_SWAP_REASONS:
            lm_swapped = True
        prev_lm = e.lm
        if pending:
            epoch += 1
            starts.append(e.ms)
            pending = False
        e.epoch = epoch
        if e.kind == "lm_unload" and e.kv.get("reason") == SHUTDOWN_REASON:
            pending = True
        sid = e.kv.get("session")
        if sid is None:
            continue
        key = (epoch, sid)
        cur = live.get(key)
        if e.kind == "begin_session":
            app = e.kv.get("app")
            if cur is None or cur[2] or (app is not None and cur[1] is not None and app != cur[1]):
                instances += 1
                cur = live[key] = [instances, app, False, True]
            elif cur[1] is None and app is not None:
                cur[1] = app
            if not cur[3]:
                cur[3] = True
            if app.endswith(SMOKE_APP_SUFFIX):
                smoke_sessions.add(cur[0])
                begins[epoch][0] += 1
            else:
                begins[epoch][1] += 1
        elif cur is None:
            instances += 1  # begin_session が見えない（ログの先頭が欠けた、起動後に begin されなかった）
            cur = live[key] = [instances, None, False, False]
        if e.err == "UNKNOWN_SESSION" and e.kind != "begin_session":
            e.sstate = SSTATE_NO_BEGIN if not cur[3] else SSTATE_ENDED if cur[2] else SSTATE_LIVE
        if e.kind == "end_session":
            cur = live.get(key)
            if cur is not None:
                cur[2] = True
        e.smoke = cur is not None and cur[0] in smoke_sessions
    smoke_epochs = {ep for ep, (sm, other) in begins.items() if sm > 0 and other == 0}
    for e in events:
        if e.epoch in smoke_epochs:
            e.smoke = True
    return starts, instances, len(smoke_sessions)


# ---------------------------------------------------------------- 節


def section_correction(events):
    eps = [e for e in events if e.kind == "sh_episode"]
    commits = [e for e in eps if e.kv.get("end") in COMMIT_ENDS]
    fresh = [e for e in commits if e.kv.get("start") != "reconvert"]
    recon = [e for e in commits if e.kv.get("start") == "reconvert"]

    def first_rank(e):
        chosen_ok = e.num("pick") == 0 if e.kv.get("end") == "commit_cand" else e.num("seg_changed") == 0
        return chosen_ok and e.num("seg_resize") == 0 and e.num("esc_back") == 0

    rows = [("sh_episode 全体", len(eps), "")]
    ok = sum(1 for e in fresh if first_rank(e))
    rows.append(("確定した合成（再変換を除く）", len(fresh), ""))
    rows.append(("1 位のまま確定", ok, pct(ok, len(fresh))))
    rows.append(("1 位から外れた", len(fresh) - ok, pct(len(fresh) - ok, len(fresh))))
    rows.append(("再変換から始まった確定", len(recon), ""))
    # 選んだ番号（commit_cand のみ）
    cands = [e for e in fresh if e.kv.get("end") == "commit_cand"]
    pick = collections.Counter(min(e.num("pick"), PICK_BUCKETS) for e in cands)
    for i in range(PICK_BUCKETS + 1):
        label = f"pick={i}" if i < PICK_BUCKETS else f"pick={PICK_BUCKETS}+"
        rows.append((label, pick[i], pct(pick[i], len(cands))))
    spaces = collections.Counter(min(e.num("spaces"), SPACES_BUCKETS) for e in fresh)
    for i in range(SPACES_BUCKETS + 1):
        label = f"spaces={i}" if i < SPACES_BUCKETS else f"spaces={SPACES_BUCKETS}+"
        rows.append((label, spaces[i], pct(spaces[i], len(fresh))))
    for key, label in (
        ("pages", "ページを進めた合成"),
        ("digit_pick", "数字キーで選んだ合成"),
        ("seg_focus", "文節のモード（フォーカス移動あり）の合成"),
        ("seg_resize", "区切り直しあり合成"),
        ("esc_back", "Esc で読みに戻した合成"),
    ):
        n = sum(1 for e in fresh if e.num(key) > 0)
        rows.append((label, n, pct(n, len(fresh))))
        if key in ("seg_focus", "seg_resize", "esc_back"):
            rows.append((f"{key} の合計回数", sum(e.num(key) for e in fresh), ""))
    multi = sum(1 for e in fresh if e.num("segs") > 1)
    rows.append(("2 文節以上で確定", multi, pct(multi, len(fresh))))
    return ("直し方の分布", ("項目", "件数", "率"), rows)


def section_first_rank_by_day(events):
    per = collections.defaultdict(lambda: [0, 0])
    for e in events:
        if e.kind != "sh_episode" or e.kv.get("end") not in COMMIT_ENDS or e.kv.get("start") == "reconvert":
            continue
        good = (e.num("pick") == 0 if e.kv["end"] == "commit_cand" else e.num("seg_changed") == 0) and e.num(
            "seg_resize"
        ) == 0 and e.num("esc_back") == 0
        d = per[day_of(e.ms)]
        d[0] += 1
        d[1] += good
    rows = [(d, n, g, pct(g, n)) for d, (n, g) in sorted(per.items())]
    return ("1 位のまま確定した率（日ごと）", ("日(UTC)", "確定", "1位のまま", "率"), rows)


def section_ends(events):
    eps = [e for e in events if e.kind == "sh_episode"]
    by_end = collections.Counter((e.kv.get("end", "?"), e.kv.get("mode", "?")) for e in eps)
    ends = collections.Counter(e.kv.get("end", "?") for e in eps)
    rows = [(end, mode, n, pct(n, len(eps))) for (end, mode), n in sorted(by_end.items())]
    prev = collections.Counter((e.kv["end"], e.kv.get("prev", "?")) for e in eps if e.kv.get("end") in DISTURBED_ENDS)
    for (end, p), n in sorted(prev.items()):
        rows.append((f"{end} の直前", p, n, pct(n, ends[end])))
    return ("終わった理由の内訳", ("end", "mode", "件数", "率"), rows)


def section_flow(events):
    eps = [e for e in events if e.kind == "sh_episode"]
    rows = []
    for key in ("rlen", "keys_to_conv", "dur_ms", "pick_ms"):
        vals = sorted(e.num(key) for e in eps if key in e.kv)
        rows.append((key, len(vals), fmt(percentile(vals, 50)), fmt(percentile(vals, 90))))
    modes = collections.Counter((e.kv.get("from", "?"), e.kv.get("to", "?")) for e in events if e.kind == "sh_mode")
    for (a, b), n in sorted(modes.items()):
        rows.append((f"sh_mode {a}->{b}", n, "", ""))
    return ("入力の流れ", ("項目", "件数", "p50", "p90"), rows)


def section_usage(events):
    rows = []
    eps = [e for e in events if e.kind == "sh_episode"]
    for k, n in sorted(collections.Counter(e.kv["pick_kind"] for e in eps if "pick_kind" in e.kv).items()):
        rows.append((f"pick_kind={k}", n))
    for k, n in sorted(collections.Counter(e.kv["rw"] for e in events if "rw" in e.kv).items()):
        rows.append((f"rw={k}", n))
    sc = [e for e in eps if e.kv.get("mode", "").startswith("shortcode")]
    rows.append(("mode=shortcode_* の合成", len(sc)))
    picked = sum(1 for e in sc if e.kv.get("end") == "shortcode_pick")
    rows.append(("うち end=shortcode_pick", f"{picked} ({pct(picked, len(sc))})"))
    for k, n in sorted(collections.Counter(e.kv["mode"] for e in eps if "mode" in e.kv).items()):
        rows.append((f"mode={k}", n))
    return ("機能の利用", ("項目", "件数"), rows)


def section_trouble(events):
    rows = []
    eps = [e for e in events if e.kind == "sh_episode"]
    for key in ("ctx", "panel"):
        for k, n in sorted(collections.Counter(e.kv[key] for e in eps if key in e.kv).items()):
            rows.append((f"{key}={k}", n))
    conv = [e for e in events if e.kind in ("convert", "prefetch") and e.err == "-"]
    no_ctx = sum(1 for e in conv if e.kv.get("ctx") == "0")
    rows.append(("convert/prefetch の ctx=0", f"{no_ctx} ({pct(no_ctx, len(conv))})"))
    for key in ("timeout", "interrupted", "invalidated", "fallback_convs", "breaker_open", "reconnect", "resync"):
        rows.append((f"sh_engine {key} 合計", sum(e.num(key) for e in events if e.kind == "sh_engine")))
    errs = collections.Counter((e.kind, e.err) for e in events if e.err != "-")
    for (kind, err), n in sorted(errs.items()):
        rows.append((f"err={err} ({kind})", n))
    return ("不具合の手がかり", ("項目", "件数"), rows)


def gap_bucket(gap_ms):
    """None（直前に該当の行なし）か ms を、区切りの名前にする。"""
    if gap_ms is None:
        return GAP_NONE
    for limit, name in GAP_BUCKETS:
        if gap_ms < limit:
            return name
    return GAP_LAST


def latest_before(times, ms):
    """昇順の times で ms 以下のうち最後の値。なければ None。"""
    i = bisect.bisect_right(times, ms)
    return times[i - 1] if i else None


def section_unknown_session(events):
    """`err=UNKNOWN_SESSION` の内訳。ログの行には session= がないので、時間の前後関係で見る。"""
    unknown = [e for e in events if e.err == "UNKNOWN_SESSION"]
    rows = [("UNKNOWN_SESSION の行", len(unknown), "")]
    for kind, n in sorted(collections.Counter(e.kind for e in unknown).items()):
        rows.append((f"kind={kind}", n, pct(n, len(unknown))))
    loading = sum(1 for e in unknown if e.lm == "loading")
    rows.append(("lm=loading の行", loading, pct(loading, len(unknown))))
    rows.append(("エンジンの起動（epoch）の数", len({e.epoch for e in unknown}), ""))
    ends = sorted(e.ms for e in events if e.kind == "end_session")
    begins = sorted(e.ms for e in events if e.kind == "begin_session")
    first_of = {}
    for e in events:
        first_of.setdefault(e.epoch, e.ms)
    refs = (
        ("直前の end_session ", lambda e: latest_before(ends, e.ms)),
        ("直前の begin_session ", lambda e: latest_before(begins, e.ms)),
        ("エンジンの起動（epoch の先頭）", lambda e: first_of.get(e.epoch)),
    )
    order = [name for _, name in GAP_BUCKETS] + [GAP_LAST, GAP_NONE]
    for label, ref in refs:
        count = collections.Counter(gap_bucket(None if ref(e) is None else e.ms - ref(e)) for e in unknown)
        for name in order:
            if count[name]:
                rows.append((f"{label}からの時間: {name}", count[name], pct(count[name], len(unknown))))
    with_sid = [e for e in unknown if "session" in e.kv]
    rows.append(("session= のある行（end_session と同じ版以降のエンジン）", len(with_sid), pct(len(with_sid), len(unknown))))
    for state, n in sorted(collections.Counter(e.sstate for e in with_sid).items(), key=lambda t: str(t[0])):
        rows.append((f"session= の状態: {state}", n, pct(n, len(with_sid))))
    # かたまり（BURST_GAP_MS 以内に続く行）。先頭の行の直前に何があったか
    bursts, last = [], None
    for e in unknown:
        if last is None or e.ms - last > BURST_GAP_MS:
            bursts.append(e)
        last = e.ms
    rows.append(("かたまり（行の間隔が 5 秒以内）の数", len(bursts), ""))
    index = {id(e): i for i, e in enumerate(events)}
    before = collections.Counter()
    silence = []
    for b in bursts:
        i = index[id(b)]
        prev = events[i - 1] if i else None
        before[prev.kind if prev else "-"] += 1
        silence.append(None if prev is None else b.ms - prev.ms)
        # 先頭の行の直前の行が別の epoch か
    for kind, n in sorted(before.items()):
        rows.append((f"かたまりの直前の行の kind={kind}", n, pct(n, len(bursts))))
    quiet = collections.Counter(gap_bucket(g) for g in silence)
    for name in order:
        if quiet[name]:
            rows.append((f"かたまりの直前の無音: {name}", quiet[name], pct(quiet[name], len(bursts))))
    new_epoch = sum(1 for b in bursts if b.ms == first_of.get(b.epoch))
    rows.append(("かたまりの先頭がエンジンの起動の最初の行", new_epoch, pct(new_epoch, len(bursts))))
    ended = sum(
        1
        for b in bursts
        if (latest_before(ends, b.ms) or -1) >= (latest_before(begins, b.ms) or -1) and latest_before(ends, b.ms) is not None
    )
    rows.append(("かたまりの前で最後の出来事が end_session", ended, pct(ended, len(bursts))))
    return ("UNKNOWN_SESSION の内訳", ("項目", "件数", "率"), rows)


def section_perf(events):
    rows = []

    def add(label, us_values):
        v = sorted(us_values)
        rows.append((label, len(v), *(fmt(percentile(v, p)) for p in CONVERT_PERCENTILES)))

    conv = [e for e in events if e.kind == "convert" and e.err == "-"]
    add("convert（全体）", [e.us for e in conv])
    groups = collections.defaultdict(list)
    for e in conv:
        groups[(e.kv.get("cache", "-"), e.kv.get("lm_applied", "-"))].append(e.us)
    for (cache, lm), vals in sorted(groups.items()):
        add(f"convert cache={cache} lm_applied={lm}", vals)
    for kind in ("segment_page", "page", "prefetch_done"):
        add(kind, [e.us for e in events if e.kind == kind and e.err == "-"])
    add("sh_episode wait_us", [e.num("wait_us") for e in events if e.kind == "sh_episode" and "wait_us" in e.kv])
    perf = ("性能（us）", ("項目", "件数", "p50", "p95", "p99"), rows)

    cache = collections.Counter(e.kv.get("cache", "-") for e in conv)
    lm_on = sum(1 for e in conv if e.kv.get("lm_applied") == "1")
    misses = collections.Counter(e.kv["miss"] for e in conv if "miss" in e.kv)
    r2 = [("convert 件数", len(conv), "")]
    for k, n in sorted(cache.items()):
        r2.append((f"cache={k}", n, pct(n, len(conv))))
    r2.append(("lm_applied=1", lm_on, pct(lm_on, len(conv))))
    for k, n in sorted(misses.items()):
        r2.append((f"miss={k}", n, pct(n, len(conv))))
    stats = sorted(e.num("rss_kb") for e in events if e.kind == "stat" and "rss_kb" in e.kv)
    r2.append(("stat rss_kb 中央値", fmt(percentile(stats, 50)), ""))
    r2.append(("stat rss_kb 最大", fmt(stats[-1] if stats else None), ""))
    loads = sorted(e.us for e in events if e.kind == "lm_load")
    r2.append(("lm_load 回数", len(loads), f"p50={fmt(percentile(loads, 50))}us"))
    for k, n in sorted(collections.Counter(e.kv["reason"] for e in events if e.kind == "lm_unload" and "reason" in e.kv).items()):
        r2.append((f"lm_unload reason={k}", n, ""))
    return perf, ("cache と LM", ("項目", "件数", "率など"), r2)


def section_health(events, malformed, files, info):
    rows = [
        ("読んだファイル", len(files)),
        ("読めた行", info["read"]),
        ("形の違う行（欠け）", malformed),
        ("log_dropped 合計", sum(e.num("n") for e in events if e.kind == "log_dropped")),
        ("diag_reject 合計", sum(e.num("n") for e in events if e.kind == "diag_reject")),
    ]
    rows.append(("エンジンの起動（epoch）", info["epochs"]))
    rows.append(("セッション（epoch, session= の組）", info["sessions"]))
    rows.append(("smoke のセッション", info["smoke_sessions"]))
    rows.append(("除いた smoke の行" if not info["include_smoke"] else "smoke の行（除かない）", info["smoke_lines"]))
    if events:
        rows.append(("期間(UTC)", f"{day_of(events[0].ms)} 〜 {day_of(events[-1].ms)}"))
    per = collections.Counter(day_of(e.ms) for e in events)
    for d, n in sorted(per.items()):
        rows.append((f"行数 {d}", n))
    return ("ログの健全性", ("項目", "値"), rows)


def build_report(events, malformed, files, info):
    perf, cache = section_perf(events)
    return [
        section_correction(events),
        section_first_rank_by_day(events),
        section_ends(events),
        section_flow(events),
        section_usage(events),
        section_trouble(events),
        section_unknown_session(events),
        perf,
        cache,
        section_health(events, malformed, files, info),
    ]


# ---------------------------------------------------------------- 出力


def render_markdown(report):
    out = []
    for title, header, rows in report:
        out.append(f"## {title}\n")
        out.append("| " + " | ".join(header) + " |")
        out.append("|" + "|".join("---" for _ in header) + "|")
        for row in rows:
            out.append("| " + " | ".join(str(c) for c in row) + " |")
        out.append("")
    return "\n".join(out)


def render_tsv(report):
    out = []
    for title, header, rows in report:
        out.append("\t".join((title, *header)))
        for row in rows:
            out.append("\t".join((title, *(str(c) for c in row))))
    return "\n".join(out) + "\n"


def date_to_ms(text, end_of_day):
    d = datetime.datetime.strptime(text, "%Y-%m-%d").replace(tzinfo=datetime.timezone.utc)
    return int(d.timestamp() * 1000) + (MS_PER_DAY if end_of_day else 0)


def main(argv=None):
    ap = argparse.ArgumentParser(description="診断ログの集計（ADR 0034 §5）")
    ap.add_argument("paths", nargs="+", help="ログのディレクトリ、またはファイル（.log / .log.N / .gz）")
    ap.add_argument("--tsv", action="store_true", help="Markdown でなく TSV で出す")
    ap.add_argument("--since", help="この日(UTC, YYYY-MM-DD)以降")
    ap.add_argument("--include-smoke", action="store_true", help="smoke のセッションの行も数える（既定は除く）")
    ap.add_argument("--until", help="この日(UTC, YYYY-MM-DD)まで（その日を含む）")
    args = ap.parse_args(argv)

    files = expand_inputs(args.paths)
    if not files:
        print("ログのファイルが見つかりません", file=sys.stderr)
        return 1
    try:
        events, malformed = read_events(files)
    except (OSError, EOFError, ValueError) as err:
        print(f"読めません: {err}", file=sys.stderr)
        return 1
    if args.since:
        events = [e for e in events if e.ms >= date_to_ms(args.since, False)]
    if args.until:
        events = [e for e in events if e.ms < date_to_ms(args.until, True)]
    read = len(events)
    starts, sessions, smoke_sessions = split_epochs(events)
    smoke_lines = sum(1 for e in events if e.smoke)
    if not args.include_smoke:
        events = [e for e in events if not e.smoke]
    info = {
        "read": read,
        "epochs": len(starts),
        "sessions": sessions,
        "smoke_sessions": smoke_sessions,
        "smoke_lines": smoke_lines,
        "include_smoke": args.include_smoke,
    }
    report = build_report(events, malformed, files, info)
    sys.stdout.write(render_tsv(report) if args.tsv else render_markdown(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
