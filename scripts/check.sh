#!/bin/bash
# コミットの前に流す門（CI を置かない代わり。docs/refactor-plan-2026-10.md §5.5、§6.2）。
#
#   scripts/check.sh                 文書とスクリプトの検査、cargo fmt --check、clippy -D warnings、cargo test --workspace、swift test
#   scripts/check.sh --lm            + ikari-lm / ikari-engine-ffi / ikari-stack を feature つきで clippy と test（cmake が要る）
#   scripts/check.sh --real          + 実データの #[ignore] テスト（辞書とモデルを使う。--lm を含む）
#   scripts/check.sh --perf          + 性能の #[ignore] テスト（release）
#   scripts/check.sh --eval <基準>   + eval/scripts/snapshot.sh --compare <基準>（振る舞いが変わっていないことの確認）
#   scripts/check.sh --docs-only     文書とスクリプトの検査だけ（数秒）
#   scripts/check.sh --no-swift      swift test を飛ばす（macOS 以外）
#
# --real の前提（setup.sh と同じ既定の場所。環境変数で変えられる）:
#   IKARI_DICT_DIR   辞書（既定 data/build/dict）
#   IKARI_ZENZ_MODEL zenz のモデル（既定 data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf）
#   IKARI_EDGE_MODEL kanakanji-edge のモデル（既定 data/raw/kanakanji-edge/kanakanji-edge-33m-Q8_0.gguf）
# 前提がなければ失敗する（黙って成功しない）。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$IKARI_ROOT"

readonly EDGE_MODEL_FILE="kanakanji-edge-33m-Q8_0.gguf"
readonly ENGINE_SOURCE="engine/src/lib.rs"

WITH_LM=0
WITH_REAL=0
WITH_PERF=0
DOCS_ONLY=0
WITH_SWIFT=1
EVAL_BASE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --lm) WITH_LM=1 ;;
    --real) WITH_REAL=1; WITH_LM=1 ;;
    --perf) WITH_PERF=1 ;;
    --eval) EVAL_BASE="${2:-}"; [[ -n "$EVAL_BASE" ]] || die "--eval には基準のディレクトリが要ります"; shift ;;
    --docs-only) DOCS_ONLY=1 ;;
    --no-swift) WITH_SWIFT=0 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done

# ---------------------------------------------------------------- 文書とスクリプト

# Markdown の相対リンクが指す先が存在するか（grep だけで調べる。アンカー # 以降と、http(s) などの外部リンクは見ない）
check_links() {
  log "文書の相対リンク"
  local bad=0 file line target path dir
  while IFS= read -r file; do
    dir="$(dirname "$file")"
    while IFS= read -r line; do
      target="${line#\](}"
      target="${target%)}"
      target="${target%% *}"            # [text](path "title")
      target="${target#<}"; target="${target%>}"
      case "$target" in
        '' | '#'* | *://* | mailto:*) continue ;;
      esac
      path="${target%%#*}"
      [[ -n "$path" ]] || continue
      if [[ ! -e "$dir/$path" ]]; then
        printf 'broken link: %s -> %s\n' "$file" "$target" >&2
        bad=1
      fi
    done < <(grep -o '\]([^)]*)' "$file" || true)
  done < <(find . -name '*.md' \
    -not -path './target/*' -not -path './data/*' -not -path './.claude/*' -not -path '*/.build/*' | sort)
  [[ "$bad" -eq 0 ]] || die "切れた相対リンクがあります"
}

# モデルのファイル名が、言語をまたいで一致していること（生成はしない: scripts/lib.sh と engine/src/lib.rs）
check_model_name() {
  log "モデルのファイル名の一致"
  grep -q "MODEL_FILE_NAME: &str = \"$ZENZ_MODEL_FILE\"" "$ENGINE_SOURCE" \
    || die "$ENGINE_SOURCE の MODEL_FILE_NAME が scripts/lib.sh の ZENZ_MODEL_FILE（${ZENZ_MODEL_FILE}）と違います"
}

# ADR の索引の状態が、各 ADR の本文の「状態」の語と一致していること
check_adr_status() {
  log "ADR の索引の状態"
  local adr number body index bad=0
  for adr in docs/adr/[0-9][0-9][0-9][0-9]-*.md; do
    number="$(basename "$adr" | cut -c1-4)"
    # 「（」や空白より前が状態。sed の [^（ ] は POSIX ロケールで「（」をバイトに分けてしまうので、文字列として切る
    body="$(grep -m1 '^- 状態:' "$adr" || true)"
    body="${body#- 状態:}"
    body="${body#"${body%%[! ]*}"}"
    body="${body%%（*}"
    body="${body%% *}"
    index="$(grep "^| \[$number\]" docs/adr/README.md | awk -F'|' '{gsub(/ /, "", $4); print $4}')"
    if [[ -z "$body" || "$body" != "$index" ]]; then
      printf 'ADR %s: 本文の状態「%s」と索引「%s」が違います\n' "$number" "$body" "$index" >&2
      bad=1
    fi
  done
  [[ "$bad" -eq 0 ]] || die "ADR の索引と本文の状態が食い違っています"
}

# 識別子の頭とチームID が、lib.sh と Identity.swift で一致していること（ADR 0036 §3.1。swift test が流せなくても検査できる）
check_identity() {
  log "Identity.swift が lib.sh の既定値から生成したものと同じか"
  local swift="macos/Sources/IkariShellCore/Identity.swift"
  # 環境変数の上書きを外して、既定値で生成し直す
  env -u IKARI_ID_PREFIX -u TEAM_ID macos/scripts/gen-identity.sh --stdout | diff -u "$swift" - >/dev/null \
    || die "$swift が lib.sh の既定値と違います。macos/scripts/gen-identity.sh を流してください"
}

check_leaks() {
  log "秘密と個人情報の検査（gitleaks。コミットの作者のメールも）"
  # コミットの作者・署名者のメールは noreply だけ（gitleaks は作者欄を見ないので、ここで調べる）
  local bad
  bad="$(git log --all --format='%ae%n%ce' 2>/dev/null | sort -u | grep -v -E 'noreply' || true)"
  [[ -z "$bad" ]] || die "noreply 以外のメールがコミットの作者欄にあります（GitHub の設定で Keep my email addresses private をオンに）: $(printf '%s' "$bad" | tr '\n' ' ')"
  if ! command -v gitleaks >/dev/null; then
    warn "gitleaks がありません。検査を飛ばします（brew install gitleaks）"
    return 0
  fi
  local conf
  for conf in .gitleaks.toml .gitleaks.local.toml; do
    [[ -f "$conf" ]] || continue
    gitleaks git . --config "$conf" --no-banner --redact >/dev/null 2>&1 \
      || die "gitleaks が履歴に漏れを見つけました（${conf}）。gitleaks git . --config ${conf} --redact で確かめてください"
    gitleaks git . --config "$conf" --staged --no-banner --redact >/dev/null 2>&1 \
      || die "gitleaks がステージ済みの変更に漏れを見つけました（${conf}）"
  done
}

check_shell_syntax() {
  log "シェルスクリプトの構文"
  local script
  for script in scripts/*.sh macos/scripts/*.sh eval/scripts/*.sh; do
    bash -n "$script" || die "$script に構文エラーがあります"
  done
}

# ---------------------------------------------------------------- Rust と Swift

run_rust_base() {
  log "cargo fmt --check"
  cargo fmt --all -- --check
  log "cargo clippy --workspace --all-targets -D warnings"
  cargo clippy --workspace --all-targets -- -D warnings
  log "cargo test --workspace"
  cargo test --workspace
}

run_rust_lm() {
  log "LM つきの clippy と test（llama.cpp をビルドする。cmake が要る）"
  command -v cmake >/dev/null || die "cmake がありません（brew install cmake）"
  cargo clippy -p ikari-lm --features metal --all-targets -- -D warnings
  cargo clippy -p ikari-stack --features lm-metal --all-targets -- -D warnings
  cargo clippy -p ikari-engine-ffi --features lm-metal --all-targets -- -D warnings
  cargo clippy -p ikari-eval --features lm-metal --all-targets -- -D warnings
  cargo test -p ikari-lm --features metal
  cargo test -p ikari-stack --features lm-metal
  cargo test -p ikari-engine-ffi --features lm-metal
}

run_swift() {
  log "swift test"
  swift test --manifest-cache none --package-path macos
}

# 実データのテスト。前提（辞書、モデル）がなければ、黙って成功せずここで失敗させる
run_real() {
  local dict="${IKARI_DICT_DIR:-$IKARI_ROOT/data/build/dict}"
  local zenz="${IKARI_ZENZ_MODEL:-$IKARI_ROOT/data/raw/zenz/$ZENZ_MODEL_FILE}"
  local edge="${IKARI_EDGE_MODEL:-$IKARI_ROOT/data/raw/kanakanji-edge/$EDGE_MODEL_FILE}"
  local file
  for file in "$dict/grammar.bin" "$dict/system.dict" "$dict/vectors.bin" "$zenz" "$edge"; do
    [[ -f "$file" ]] || die "$file がありません（scripts/setup.sh --fetch で辞書とモデルを用意する。kanakanji-edge は data/raw/kanakanji-edge/ に手で置く）"
  done
  export IKARI_DICT_DIR="$dict" IKARI_ZENZ_MODEL="$zenz" IKARI_EDGE_MODEL="$edge"

  log "実データのテスト（--ignored）"
  cargo test -p ikari-core --test dict_real -- --ignored
  cargo test --release -p ikari-core --test segment_real -- --ignored
  cargo test --release -p ikari-core --test left_context_real -- --ignored
  cargo test -p ikari-eval --test flags -- --ignored
  cargo test --release -p ikari-eval --test it_pack_real -- --ignored
  cargo test --release -p ikari-eval --test rare_homophones_real -- --ignored
  cargo test -p ikari-engine --features lm-metal -- --ignored
  cargo test -p ikari-lm --features metal --test real_model --test real_model_edge -- --ignored
  cargo test -p ikari-eval --features lm-metal --test scenario_lm -- --ignored
}

run_perf() {
  log "性能のテスト（release、--ignored）"
  cargo test --release -p ikari-core -- --ignored --nocapture perf
  # エンジン内の文節と再変換（p99 ≤ 2ms、LM なし）。実辞書が要る
  local dict="${IKARI_DICT_DIR:-$IKARI_ROOT/data/build/dict}"
  [[ -f "$dict/system.dict" ]] || die "$dict/system.dict がありません（scripts/setup.sh --fetch で辞書を用意する）"
  IKARI_DICT_DIR="$dict" cargo test --release -p ikari-engine -- --ignored --nocapture segment_perf
}

# ---------------------------------------------------------------- 実行

check_links
check_model_name
check_identity
check_leaks
check_adr_status
check_shell_syntax
scripts/test-diag-report.sh
if [[ "$DOCS_ONLY" -eq 1 ]]; then
  log "完了（--docs-only）"
  exit 0
fi

run_rust_base
if [[ "$WITH_LM" -eq 1 ]]; then run_rust_lm; fi
if [[ "$WITH_SWIFT" -eq 1 ]]; then run_swift; fi
if [[ "$WITH_REAL" -eq 1 ]]; then run_real; fi
if [[ "$WITH_PERF" -eq 1 ]]; then run_perf; fi
if [[ -n "$EVAL_BASE" ]]; then
  log "eval の比較: $EVAL_BASE"
  "$IKARI_ROOT/eval/scripts/snapshot.sh" --compare "$EVAL_BASE"
fi
log "完了"
