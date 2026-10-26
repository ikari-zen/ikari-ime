#!/usr/bin/env bash
# リファクタの前後で振る舞いが変わっていないことを確かめるための snapshot（docs/refactor-plan-2026-10.md §6.1、§7.1）。
#
# 取る:
#   snapshot.sh <出力ディレクトリ> [--lm <gguf>] [--ajimee] [--smoke] [--no-ref]
# 取って基準と比べる（--lm / --ajimee / --smoke は基準の meta.env と同じものが既定）:
#   snapshot.sh --compare <基準のディレクトリ> [<出力ディレクトリ>]
#   <出力ディレクトリ> が取得済み（COMPLETE がある）なら、取り直さずに比べるだけ。
#
# 内容（出力ディレクトリの中）:
#   meta.env             コミット、日付、構成、使ったバイナリ
#   dict.sha256          辞書（data/build/dict）の各ファイルの sha256
#   dict-build.txt       辞書を作り直すコマンドと、入力の sha256（作り直した辞書の sha256 と比べる）
#   eval/<名前>.{tsv,md,cmd}   ケース別の結果（--out-cases）、レポート、実行したコマンド
#   scenario/<名前>.md   学習のシナリオ（a-02 / a-03 / a-04 / b23-*）のレポート
#   ref/<名前>.{tsv,md}  LM あり構成を既定の締め切りで流した参考値（比較しない）
#   cli/golden.txt       eval/scripts/cli-golden.tsv の入力に対する ikari の出力
#   smoke.txt            --smoke のときだけ。macos/scripts/smoke.sh --signing team --installed の出力
#   summary.tsv          構成ごとの 1 位正解数、warm p99、LM の時間切れ、取り直した回数
#
# smoke は PASS/FAIL/SKIP の行の集合（括弧の中の pid や時間は除く）で比べる。
# 一致の判定（§7.1）: --out-cases の列 1〜7（id kind context reading expected top1 rank）がバイト単位で一致、
# シナリオのレポートと CLI の出力が一致、辞書の sha256 が一致。warm p99 は基準の 1.1 倍を超えたら警告だけ出す
# （ミリ秒未満の差は揺れるので、絶対差 0.05ms 以下は無視する）。
# LM ありの構成は --lm-budget-ms 2000 --lm-deadline-ms 2000 で流し、時間切れが 0 になるまで最大 3 回流し直す。
# 0 にならなかった構成は INVALID にして、比較の失敗として数える。
#
# 環境変数: IKARI_SNAPSHOT_FLAGS（ikari-eval と ikari に足す引数。基準を語彙集なしで取るときは --no-packs。ADR 0012）、
#           IKARI_DICT_DIR（既定 data/build/dict）、IKARI_EVAL_BIN / IKARI_BIN（バイナリを指定。指定すると build しない）、
#           IKARI_SNAPSHOT_TARGET_DIR（LM つきのバイナリの置き場所。既定 target/snapshot-bin）
# 終了コード: 0 = 取得できた（比較なら差分 0 かつ有効）、1 = 差分または INVALID がある、2 = 実行できなかった。
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${script_dir}/../.." && pwd)"
cd "$root"

# ---- 定数 -----------------------------------------------------------------
readonly LM_BUDGET_MS=2000
readonly LM_DEADLINE_MS=2000
readonly LM_MAX_ATTEMPTS=3
# LM ありは 1 回が重いので、warm の繰り返しを減らす（p99 は参考値。順位には影響しない）
readonly LM_REPEAT=3
readonly P99_RATIO_LIMIT=1.1
readonly P99_ABS_FLOOR_MS=0.05
readonly DIFF_SHOW_LINES=40
readonly CLI_NUM=12
readonly GOLDEN_TSV="$script_dir/cli-golden.tsv"
readonly AJIMEE_JSON="data/raw/ajimee/evaluation_items.json"
readonly HOMOPHONES="eval/sets/homophones.tsv"
readonly EXT="eval/sets/homophones-ext.tsv"
readonly HIRAGANA="eval/sets/hiragana.tsv"
readonly KATAKANA="eval/sets/katakana-suffix.tsv"
readonly SEGMENTS="eval/sets/segments.tsv"
readonly CONTEXT_COMPOUND="eval/sets/context-compound.tsv"
readonly ADVERBS="eval/sets/adverbs.tsv"
readonly VERBS="eval/sets/verbs.tsv"
readonly NEW_WORDS="eval/sets/new-words.tsv"
readonly COMPOUND_AFFIX="eval/sets/compound-affix.tsv"
readonly RARE_HOMOPHONES="eval/sets/rare-homophones.tsv"
readonly FORMAL_NOUN_SPLIT="eval/sets/formal-noun-split.tsv"
readonly COUNTERS="eval/sets/counters.tsv"
readonly TOSHIYOU="eval/sets/toshiyou.tsv"
readonly SAHEN_SHIYOU="eval/sets/sahen-shiyou.tsv"
readonly PATH_LOSES="eval/sets/path-loses.tsv"
readonly SCENARIOS=(a-02 a-03 a-04 b23-naoru b23-susumeru b23-same-context it-pack)
readonly CHIVE_TXT="data/raw/chive/chive-1.3-mc90/chive-1.3-mc90.txt"
readonly SUDACHI_DIR="data/raw/sudachi-20260723"
readonly CHIVE_POLICY="dict-builder/policy/chive.tsv"
readonly DICT_FILES=(grammar.bin system.dict vectors.bin)
readonly CASE_COLUMNS="1-7"

die() { echo "snapshot: $*" >&2; exit 2; }
log() { echo "snapshot: $*" >&2; }

sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
head_commit() { git rev-parse --short=7 HEAD; }

# ---- 引数 -----------------------------------------------------------------
out=""
lm=""
ajimee=0
smoke=0
ref=1
compare_base=""
lm_set=0 ajimee_set=0 smoke_set=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --lm) lm="${2:-}"; lm_set=1; shift ;;
    --ajimee) ajimee=1; ajimee_set=1 ;;
    --smoke) smoke=1; smoke_set=1 ;;
    --no-ref) ref=0 ;;
    --compare) compare_base="${2:-}"; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) die "不明なオプション: $1" ;;
    *) [[ -z "$out" ]] || die "出力ディレクトリが複数あります: $out, $1"; out="$1" ;;
  esac
  shift
done

if [[ -n "$compare_base" ]]; then
  [[ -f "$compare_base/COMPLETE" ]] || die "基準が取得済みではありません（COMPLETE がありません）: $compare_base"
  # 基準と同じ構成で取る（明示した指定を優先する）
  meta_value() { grep "^$1=" "$compare_base/meta.env" | head -1 | cut -d= -f2-; }
  base_lm="$(meta_value LM)"; base_ajimee="$(meta_value AJIMEE)"; base_smoke="$(meta_value SMOKE)"
  [[ "$lm_set" -eq 1 ]] || lm="$base_lm"
  [[ "$ajimee_set" -eq 1 ]] || ajimee="$base_ajimee"
  [[ "$smoke_set" -eq 1 ]] || smoke="$base_smoke"
  [[ -n "$out" ]] || out="target/refactor-snapshot/$(head_commit)-$(date +%Y%m%d-%H%M%S)"
else
  [[ -n "$out" ]] || die "使い方: snapshot.sh <出力ディレクトリ> [--lm <gguf>] [--ajimee] [--smoke] | snapshot.sh --compare <基準> [<出力>]"
fi

# ---- 取得 -----------------------------------------------------------------
dict_dir="${IKARI_DICT_DIR:-data/build/dict}"
# ikari-eval と ikari に足す引数（語彙集なしの基準を取る --no-packs など）
read -r -a extra_flags <<< "${IKARI_SNAPSHOT_FLAGS:-}"
EVAL=""
CLI=""

prepare_binaries() {
  if [[ -n "${IKARI_EVAL_BIN:-}" && -n "${IKARI_BIN:-}" ]]; then
    EVAL="$IKARI_EVAL_BIN"; CLI="$IKARI_BIN"
    return
  fi
  if [[ -n "$lm" ]]; then
    local tdir="${IKARI_SNAPSHOT_TARGET_DIR:-$root/target/snapshot-bin}"
    log "build (release, lm-metal) -> $tdir"
    CARGO_TARGET_DIR="$tdir" cargo build --release -p ikari-eval -p ikari-cli --features lm-metal >&2
    EVAL="$tdir/release/ikari-eval"; CLI="$tdir/release/ikari"
  else
    log "build (release)"
    cargo build --release -p ikari-eval -p ikari-cli >&2
    EVAL="$root/target/release/ikari-eval"; CLI="$root/target/release/ikari"
  fi
}

# レポート（Markdown）から値を取り出す
report_top1() { awk -F'|' '$2 ~ /^ *全体 *$/ {gsub(/ /,"",$4); print $4; exit}' "$1"; }
report_cases() { awk -F'|' '$2 ~ /^ *全体 *$/ {gsub(/ /,"",$3); print $3; exit}' "$1"; }
report_warm_p99() { awk -F'|' '$2 ~ /^ *warm *$/ {gsub(/ /,"",$5); print $5; exit}' "$1"; }
report_lm_calls() { sed -n 's/^- LM: 呼び出し \([0-9]*\) \/ 時間切れ \([0-9]*\) .*/\1/p' "$1" | head -1; }
report_lm_timeouts() { sed -n 's/^- LM: 呼び出し \([0-9]*\) \/ 時間切れ \([0-9]*\) .*/\2/p' "$1" | head -1; }

add_summary() { # name cases top1 p99 calls timeouts attempts valid
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$@" >> "$out/summary.tsv"
}

# run_eval <名前> <ranker> <lm:0|1> <eval の追加引数...>
run_eval() {
  local name="$1" ranker="$2" use_lm="$3"; shift 3
  local base=("$EVAL" --dict "$dict_dir" --ranker "$ranker" ${extra_flags[@]+"${extra_flags[@]}"})
  local md="$out/eval/$name.md" tsv="$out/eval/$name.tsv"
  local attempts=0 valid=ok timeouts="" calls=""
  if [[ "$use_lm" -eq 1 ]]; then
    base+=(--lm "$lm" --lm-budget-ms "$LM_BUDGET_MS" --lm-deadline-ms "$LM_DEADLINE_MS" --repeat "$LM_REPEAT")
  fi
  while :; do
    attempts=$((attempts + 1))
    printf '%q ' "${base[@]}" "$@" --out-cases "$tsv" > "$out/eval/$name.cmd"; echo >> "$out/eval/$name.cmd"
    "${base[@]}" "$@" --out-cases "$tsv" > "$md" 2> "$out/eval/$name.err" || log "warn: $name の終了コードが 0 ではありません（基準を割る構成は正常。.err を見る）"
    [[ "$use_lm" -eq 1 ]] || break
    calls="$(report_lm_calls "$md")"; timeouts="$(report_lm_timeouts "$md")"
    [[ -n "$timeouts" ]] || { valid="NO-LM-LINE"; break; }
    [[ "$timeouts" -eq 0 ]] && break
    if [[ "$attempts" -ge "$LM_MAX_ATTEMPTS" ]]; then valid="INVALID(timeouts=$timeouts)"; break; fi
    log "$name: LM の時間切れ $timeouts 件。流し直す（$attempts/${LM_MAX_ATTEMPTS}）"
  done
  [[ -s "$out/eval/$name.err" ]] || rm -f "$out/eval/$name.err"
  add_summary "$name" "$(report_cases "$md")" "$(report_top1 "$md")" "$(report_warm_p99 "$md")" "${calls:--}" "${timeouts:--}" "$attempts" "$valid"
  log "$name: $(report_top1 "$md") / $(report_cases "$md") warm p99=$(report_warm_p99 "$md")ms lm_timeouts=${timeouts:--} attempts=$attempts $valid"
}

# 既定の締め切りでの参考値（比較しない）
run_ref() {
  local name="$1" ranker="$2"; shift 2
  "$EVAL" --dict "$dict_dir" --ranker "$ranker" ${extra_flags[@]+"${extra_flags[@]}"} --lm "$lm" --repeat "$LM_REPEAT" "$@" --out-cases "$out/ref/$name.tsv" > "$out/ref/$name.md" 2>/dev/null || true
  log "ref $name: $(report_top1 "$out/ref/$name.md") lm_timeouts=$(report_lm_timeouts "$out/ref/$name.md")"
}

run_scenario() { # name scenario lm
  local name="$1" file="$2" use_lm="$3"
  local args=("$EVAL" --dict "$dict_dir" --ranker full ${extra_flags[@]+"${extra_flags[@]}"} --scenario "$file")
  [[ "$use_lm" -eq 1 ]] && args+=(--lm "$lm" --lm-budget-ms "$LM_BUDGET_MS" --lm-deadline-ms "$LM_DEADLINE_MS")
  "${args[@]}" > "$out/scenario/$name.md" 2> /dev/null || true
  log "scenario $name: $(sed -n 's/^- steps: .* \/ 失敗: \([0-9]*\)$/失敗 \1/p' "$out/scenario/$name.md")"
}

record_dict() {
  : > "$out/dict.sha256"
  local f
  for f in "${DICT_FILES[@]}"; do
    [[ -f "$dict_dir/$f" ]] || die "辞書がありません: $dict_dir/$f"
    printf '%s  %s\n' "$(sha256_of "$dict_dir/$f")" "$f" >> "$out/dict.sha256"
  done
  {
    echo "# 辞書を作り直して、dict.sha256 と比べる（ビルドは決定的。docs/refactor-plan-2026-10.md §7.1）"
    echo "# builder: HEAD:dict-builder=$(git rev-parse HEAD:dict-builder) HEAD:dict-format=$(git rev-parse HEAD:dict-format)"
    echo "# commit: $(git rev-parse HEAD)"
    echo
    echo "## コマンド（リポジトリのルートで。出力先は data/build/dict.new などに変える）"
    echo "cargo run --release -p ikari-dict-builder -- --policy $CHIVE_POLICY --chive-vocab $CHIVE_TXT --lex $SUDACHI_DIR/small_lex.csv --lex $SUDACHI_DIR/core_lex.csv --notcore-lex $SUDACHI_DIR/notcore_lex.csv --matrix $SUDACHI_DIR/matrix.def --label sudachi-full-<版> --out <OUT>"
    echo "cargo run --release -p ikari-dict-builder -- vectors --chive $CHIVE_TXT --dict <OUT> --out <OUT>/vectors.bin --lex $SUDACHI_DIR/small_lex.csv --lex $SUDACHI_DIR/core_lex.csv --lex $SUDACHI_DIR/notcore_lex.csv"
    echo "cargo run --release -p ikari-dict-builder -- --verify <OUT>"
    echo
    echo "## 入力（sha256  サイズ  パス）"
    local p
    for p in "$CHIVE_POLICY" "$CHIVE_TXT" "$SUDACHI_DIR/small_lex.csv" "$SUDACHI_DIR/core_lex.csv" "$SUDACHI_DIR/notcore_lex.csv" "$SUDACHI_DIR/matrix.def"; do
      if [[ -f "$p" ]]; then
        printf '%s  %s  %s\n' "$(sha256_of "$p")" "$(wc -c < "$p" | tr -d " ")" "$p"
      else
        printf 'MISSING  -  %s\n' "$p"
      fi
    done
  } > "$out/dict-build.txt"
}

run_cli_golden() {
  [[ -f "$GOLDEN_TSV" ]] || die "golden がありません: $GOLDEN_TSV"
  local tmp ctx reading
  tmp="$(mktemp -d)"
  : > "$out/cli/golden.txt"
  local line
  # タブは IFS の空白として先頭が詰められるので、read ではなく文字列操作で分ける（context が空の行がある）
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ "$line" == \#* || "$line" != *$'\t'* ]] && continue
    ctx="${line%%$'\t'*}"; reading="${line#*$'\t'}"
    [[ -n "$reading" ]] || continue
    rm -rf "$tmp/user"; mkdir -p "$tmp/user"
    {
      printf '### context=%s reading=%s\n' "$ctx" "$reading"
      "$CLI" --dict "$dict_dir" --user-dir "$tmp/user" ${extra_flags[@]+"${extra_flags[@]}"} --format tsv -n "$CLI_NUM" --context "$ctx" "$reading" 2>&1 || echo "(exit $?)"
    } >> "$out/cli/golden.txt"
  done < "$GOLDEN_TSV"
  rm -rf "$tmp"
  log "cli golden: $(grep -c '^### ' "$out/cli/golden.txt") 入力"
}

take_snapshot() {
  [[ ! -e "$out/COMPLETE" ]] || die "取得済みです（上書きしません）: $out"
  [[ -f "$dict_dir/system.dict" ]] || die "辞書がありません: $dict_dir"
  [[ -z "$lm" || -f "$lm" ]] || die "LM のモデルがありません: $lm"
  [[ "$ajimee" -eq 0 || -f "$AJIMEE_JSON" ]] || die "AJIMEE がありません: ${AJIMEE_JSON}（eval/scripts/fetch_ajimee.sh）"
  mkdir -p "$out/eval" "$out/scenario" "$out/cli" "$out/ref"
  : > "$out/summary.tsv"
  local started=$SECONDS
  prepare_binaries

  {
    echo "COMMIT=$(git rev-parse HEAD)"
    echo "DIRTY_FILES=$(git status --porcelain | wc -l | tr -d ' ')"
    echo "DATE=$(date '+%Y-%m-%dT%H:%M:%S%z')"
    echo "LM=$lm"
    echo "AJIMEE=$ajimee"
    echo "SMOKE=$smoke"
    echo "DICT_DIR=$dict_dir"
    echo "EVAL_BIN=$EVAL"
    echo "CLI_BIN=$CLI"
    echo "LM_BUDGET_MS=$LM_BUDGET_MS"
    echo "LM_DEADLINE_MS=$LM_DEADLINE_MS"
    echo "LM_REPEAT=$LM_REPEAT"
    [[ -z "$lm" ]] || echo "LM_SHA256=$(sha256_of "$lm")"
  } > "$out/meta.env"

  record_dict

  # 1. homophones
  run_eval homophones-cost cost 0 --set "$HOMOPHONES"
  run_eval homophones-topic topic 0 --set "$HOMOPHONES"
  run_eval homophones-full full 0 --set "$HOMOPHONES"
  # 2. ext（dev / test）
  local split
  for split in dev test; do
    run_eval "ext-$split-topic" topic 0 --set "$EXT" --split "$split"
    run_eval "ext-$split-full" full 0 --set "$EXT" --split "$split"
  done
  # 3. hiragana
  run_eval hiragana-topic topic 0 --set "$HIRAGANA"
  # 3b. katakana-suffix（接尾の漢字とカタカナ。dev / test をまとめて）
  run_eval katakana-topic topic 0 --set "$KATAKANA"
  run_eval katakana-full full 0 --set "$KATAKANA"
  # 3d. context-compound（左文脈の語との複合語の加点の副作用。ADR 0007 §8。dev / test をまとめて）
  run_eval context-compound-full full 0 --set "$CONTEXT_COMPOUND"
  # 3e. adverbs（副詞と、こう／そう／どう／もう で始まる語の自然な入力。ADR 0008 §6.2。dev / test をまとめて）
  run_eval adverbs-full full 0 --set "$ADVERBS"
  # 3f. verbs（同音の動詞の表記。ADR 0009 §4。dev / test をまとめて）
  run_eval verbs-full full 0 --set "$VERBS"
  # 3g. compound-affix（辞書の1語の熟語と、同音の語＋接辞の分割。ADR 0011 §6。dev / test をまとめて）
  run_eval compound-affix-full full 0 --set "$COMPOUND_AFFIX"
  # 3h. new-words（新語と IT の語彙集。ADR 0012 §9。dev / test をまとめて）
  run_eval new-words-full full 0 --set "$NEW_WORDS"
  # 3i. rare-homophones（まれな同音語と サ変でない名詞＋する。ADR 0016。dev / test をまとめて）
  run_eval rare-homophones-full full 0 --set "$RARE_HOMOPHONES"
  # 3j. formal-noun-split（形式名詞の漢字の加点の漏れと、守る側の句。ADR 0017。dev / test をまとめて）
  run_eval formal-noun-split-full full 0 --set "$FORMAL_NOUN_SPLIT"
  # 3k. counters（数詞のあとの助数詞。ADR 0019 §2.2 の規則 C。dev / test をまとめて）
  run_eval counters-full full 0 --set "$COUNTERS"
  # 3l. toshiyou（助詞＋しよう が 名詞の 使用 に負ける誤りと、守る側。issue #110。dev / test をまとめて）
  run_eval toshiyou-full full 0 --set "$TOSHIYOU"
  run_eval sahen-shiyou-full full 0 --set "$SAHEN_SHIYOU"
  # 3m. path-loses（経路が負ける型と、その補正の守り。ADR 0035。dev / test をまとめて）
  run_eval path-loses-full full 0 --set "$PATH_LOSES"
  # 3c. segments（文節の区切り。ケース別の --out-cases がないので、レポートを比べる）
  "$EVAL" segments --dict "$dict_dir" --ranker full ${extra_flags[@]+"${extra_flags[@]}"} --set "$SEGMENTS" --split all > "$out/eval/segments.md" 2> /dev/null || log "warn: segments の終了コードが 0 ではありません"
  log "segments: $(awk -F'|' '$2 ~ /^ *全体 *$/ {gsub(/ /,"",$4); gsub(/ /,"",$7); print "完全一致 " $4 " F1 " $7; exit}' "$out/eval/segments.md")"
  # 4. AJIMEE
  if [[ "$ajimee" -eq 1 ]]; then
    run_eval ajimee-topic topic 0 --set "$AJIMEE_JSON" --set-format ajimee
  fi
  # 5. LM あり
  if [[ -n "$lm" ]]; then
    run_eval homophones-full-lm full 1 --set "$HOMOPHONES"
    for split in dev test; do
      run_eval "ext-$split-topic-lm" topic 1 --set "$EXT" --split "$split"
    done
    run_eval hiragana-topic-lm topic 1 --set "$HIRAGANA"
    run_eval verbs-full-lm full 1 --set "$VERBS"
    if [[ "$ajimee" -eq 1 ]]; then
      run_eval ajimee-topic-lm topic 1 --set "$AJIMEE_JSON" --set-format ajimee
    fi
  fi
  # 6. シナリオ
  local s
  for s in "${SCENARIOS[@]}"; do
    run_scenario "$s" "eval/scenarios/$s.tsv" 0
    [[ -z "$lm" ]] || run_scenario "$s-lm" "eval/scenarios/$s.tsv" 1
  done
  # 7. CLI
  run_cli_golden
  # 8. 既定の締め切りでの参考値
  if [[ -n "$lm" && "$ref" -eq 1 ]]; then
    run_ref homophones-full-lm full --set "$HOMOPHONES"
    for split in dev test; do run_ref "ext-$split-topic-lm" topic --set "$EXT" --split "$split"; done
    run_ref hiragana-topic-lm topic --set "$HIRAGANA"
    [[ "$ajimee" -eq 0 ]] || run_ref ajimee-topic-lm topic --set "$AJIMEE_JSON" --set-format ajimee
  fi
  # 9. smoke
  if [[ "$smoke" -eq 1 ]]; then
    log "smoke (インストール済みのアプリを使う。再インストールはしない)"
    local rc=0
    macos/scripts/smoke.sh --signing team --installed > "$out/smoke.txt" 2>&1 || rc=$?
    echo "(exit $rc)" >> "$out/smoke.txt"
    log "smoke: exit $rc"
  fi
  rmdir "$out/ref" 2>/dev/null || true

  echo "ELAPSED_SECONDS=$((SECONDS - started))" >> "$out/meta.env"
  date '+%Y-%m-%dT%H:%M:%S%z' > "$out/COMPLETE"
  log "完了: ${out}（$((SECONDS - started)) 秒）"
}

# ---- 比較 -----------------------------------------------------------------
fail_count=0
note_fail() { fail_count=$((fail_count + 1)); }

# 2 つのファイルを diff して、先頭だけ見せる。差分があれば 1
show_diff() { # label a b
  local label="$1" a="$2" b="$3"
  if diff -q "$a" "$b" > /dev/null 2>&1; then
    printf '  %-28s 一致\n' "$label"
    return 0
  fi
  local n
  n="$({ diff "$a" "$b" || true; } | grep -c '^[<>]' || true)"
  printf '  %-28s 差分あり（%s 行）\n' "$label" "$n"
  diff "$a" "$b" | head -n "$DIFF_SHOW_LINES" | sed 's/^/      /' || true
  return 1
}

compare_cases() { # name base other
  local name="$1" a="$2/eval/$1.tsv" b="$3/eval/$1.tsv"
  [[ -f "$a" && -f "$b" ]] || { printf '  %-28s ファイルがない\n' "$name"; note_fail; return; }
  local ca cb
  ca="$(mktemp)"; cb="$(mktemp)"
  cut -f"$CASE_COLUMNS" "$a" > "$ca"; cut -f"$CASE_COLUMNS" "$b" > "$cb"
  if cmp -s "$ca" "$cb"; then
    printf '  %-28s 一致（%s 行）\n' "$name" "$(wc -l < "$ca" | tr -d ' ')"
  else
    printf '  %-28s 差分あり\n' "$name"
    "$EVAL" compare "$a" "$b" 2>&1 | head -n "$DIFF_SHOW_LINES" | sed 's/^/      /' || true
    diff "$ca" "$cb" | head -n "$DIFF_SHOW_LINES" | sed 's/^/      /' || true
    note_fail
  fi
  rm -f "$ca" "$cb"
}

summary_field() { # file name column
  awk -F'\t' -v n="$2" -v c="$3" '$1 == n {print $c; exit}' "$1"
}

compare_snapshots() {
  local base="$1" other="$2"
  echo "== 比較: 基準 $base ($(grep '^COMMIT=' "$base/meta.env" | cut -d= -f2 | cut -c1-7))"
  echo "        今回 $other ($(grep '^COMMIT=' "$other/meta.env" | cut -d= -f2 | cut -c1-7), 未コミットの変更 $(grep '^DIRTY_FILES=' "$other/meta.env" | cut -d= -f2) 件)"

  echo "-- 辞書の sha256"
  show_diff dict.sha256 "$base/dict.sha256" "$other/dict.sha256" || note_fail

  echo "-- eval（--out-cases の列 ${CASE_COLUMNS}）"
  local names n
  names="$(cut -f1 "$base/summary.tsv")"
  for n in $names; do compare_cases "$n" "$base" "$other"; done
  # 基準にない構成が今回にだけあるのは、構成が違うということ
  for n in $(cut -f1 "$other/summary.tsv"); do
    grep -q "^$n	" "$base/summary.tsv" || { printf '  %-28s 基準にない構成\n' "$n"; note_fail; }
  done

  echo "-- segments（区切りのレポート）"
  show_diff eval/segments.md "$base/eval/segments.md" "$other/eval/segments.md" || note_fail

  echo "-- LM の有効性（時間切れ 0）"
  local bad=0 v
  while IFS=$'\t' read -r n _ _ _ _ _ _ v; do
    [[ "$v" == ok ]] || { printf '  %-28s %s（今回）\n' "$n" "$v"; bad=1; }
  done < "$other/summary.tsv"
  while IFS=$'\t' read -r n _ _ _ _ _ _ v; do
    [[ "$v" == ok ]] || { printf '  %-28s %s（基準）\n' "$n" "$v"; bad=1; }
  done < "$base/summary.tsv"
  if [[ "$bad" -eq 0 ]]; then echo "  すべて有効"; else note_fail; fi

  echo "-- シナリオ"
  local f
  for f in "$base"/scenario/*.md; do
    n="$(basename "$f")"
    if [[ -f "$other/scenario/$n" ]]; then
      show_diff "$n" "$f" "$other/scenario/$n" || note_fail
    else
      printf '  %-28s 今回にない\n' "$n"; note_fail
    fi
  done

  echo "-- CLI golden"
  show_diff cli/golden.txt "$base/cli/golden.txt" "$other/cli/golden.txt" || note_fail

  echo "-- smoke"
  if [[ -f "$base/smoke.txt" && -f "$other/smoke.txt" ]]; then
    local pa pb
    pa="$(grep -E '^(PASS|FAIL|SKIP)|^\(exit' "$base/smoke.txt" | sed -E 's/  \(.*//' | sort)"
    pb="$(grep -E '^(PASS|FAIL|SKIP)|^\(exit' "$other/smoke.txt" | sed -E 's/  \(.*//' | sort)"
    if [[ "$pa" == "$pb" ]]; then echo "  PASS/FAIL の行の集合: 一致"; else echo "  PASS/FAIL の行の集合: 差分あり"; { diff <(echo "$pa") <(echo "$pb") || true; } | sed 's/^/      /'; note_fail; fi
  else
    echo "  どちらかに smoke.txt がない（比較しない）"
  fi

  echo "-- 速さ（warm p99。基準の ${P99_RATIO_LIMIT} 倍超かつ ${P99_ABS_FLOOR_MS}ms 超の悪化だけ警告。失敗にはしない）"
  local warned=0 pa pb
  for n in $names; do
    pa="$(summary_field "$base/summary.tsv" "$n" 4)"; pb="$(summary_field "$other/summary.tsv" "$n" 4)"
    [[ -n "$pa" && -n "$pb" ]] || continue
    if awk -v a="$pa" -v b="$pb" -v r="$P99_RATIO_LIMIT" -v f="$P99_ABS_FLOOR_MS" 'BEGIN{exit !(b > a*r && b-a > f)}'; then
      printf '  警告 %-24s %s ms -> %s ms\n' "$n" "$pa" "$pb"; warned=1
    fi
  done
  [[ "$warned" -eq 1 ]] || echo "  悪化なし"

  echo
  if [[ "$fail_count" -eq 0 ]]; then
    echo "結果: 差分 0（一致）"
  else
    echo "結果: 差分または無効が ${fail_count} 件"
  fi
}

# ---- 実行 -----------------------------------------------------------------
if [[ -f "$out/COMPLETE" && -n "$compare_base" ]]; then
  log "取得済みの $out を使う"
  EVAL="${IKARI_EVAL_BIN:-}"
  if [[ -z "$EVAL" ]]; then
    # shellcheck disable=SC2046
    EVAL="$(grep '^EVAL_BIN=' "$out/meta.env" | cut -d= -f2-)"
  fi
else
  take_snapshot
fi

if [[ -n "$compare_base" ]]; then
  compare_snapshots "$compare_base" "$out"
  [[ "$fail_count" -eq 0 ]] || exit 1
fi
