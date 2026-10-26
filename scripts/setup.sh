#!/bin/bash
# 取得 -> 辞書のビルド -> データの配置 -> アプリのビルドとインストール -> smoke を 1 コマンドで通す（macOS）。
#
#   scripts/setup.sh [--signing team|adhoc] [--no-lm] [--fetch] [--rebuild-dict]
#                    [--skip-app] [--skip-smoke] [--no-install]
#
#   --signing S      アプリの署名。team（既定。Apple Development 証明書が要る）か adhoc
#   --no-lm          LM を使わない構成にする（cmake とモデルが要らない。アプリに llama.cpp を入れない）
#   --fetch          足りない元データ（SudachiDict、chiVe、zenz）を取得する。付けないと、足りなければ案内を出して止まる
#   --rebuild-dict   入力が前回と同じでも辞書を作り直す
#   --skip-app       アプリのビルドとインストール（と smoke）を飛ばす（辞書とデータの配置だけ）
#   --skip-smoke     smoke を飛ばす
#   --no-install     アプリを ~/Library/Input Methods に入れない。ビルドまでを実行し、install と smoke は
#                    実行するコマンドを表示するだけにする（ドライラン）
#
# 段:
#   0 前提の確認  1 取得（scripts/fetch-data.sh）  2 辞書（scripts/build-dict.sh）  3 データの配置（scripts/install-data.sh）
#   4 アプリ（macos/scripts/build.sh、install.sh）  5 smoke（macos/scripts/smoke.sh --installed）
# 各段は単独でも呼べる。コマンドの引数の細かい仕様は各スクリプトの --help が正。
# 入力ソースの追加（初回だけ）は自動化しない。README の「入力ソースの追加」を手で行う。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$IKARI_ROOT"

readonly MIN_FREE_KB=1048576  # 1 GB（目安。辞書、vectors、ビルドの生成物）
readonly MACOS_SCRIPTS="$IKARI_ROOT/macos/scripts"

SIGNING=team
WITH_LM=1
DO_FETCH=0
REBUILD_DICT=0
SKIP_APP=0
SKIP_SMOKE=0
NO_INSTALL=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signing) SIGNING="${2:-}"; shift ;;
    --signing=*) SIGNING="${1#--signing=}" ;;
    --no-lm) WITH_LM=0 ;;
    --fetch) DO_FETCH=1 ;;
    --rebuild-dict) REBUILD_DICT=1 ;;
    --skip-app) SKIP_APP=1 ;;
    --skip-smoke) SKIP_SMOKE=1 ;;
    --no-install) NO_INSTALL=1 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done
case "$SIGNING" in
  team | adhoc) ;;
  *) die "--signing は team か adhoc です: $SIGNING" ;;
esac

# 実行する（--no-install のときの install と smoke は表示するだけ）
run() { printf '+ %s\n' "$*"; "$@"; }
show() { printf '[dry-run] %s\n' "$*"; }

# ---------------------------------------------------------------- 0. 前提の確認

# build.sh の resolve_identity と同じ判定（subject.OU がチームID の Apple Development 証明書）。
# 取得とビルドに時間をかけたあとで止まらないよう、先に確かめる
check_team_certificate() {
  [[ -z "${IKARI_SIGN_IDENTITY:-}" ]] || return 0
  local team_id hash name ou
  # lib.sh は別のスクリプトの定義（log、die など）と重なるので、サブシェルで読む
  team_id="$(source "$MACOS_SCRIPTS/lib.sh" && printf '%s' "$TEAM_ID")"
  [[ -n "$team_id" ]] || die "macos/scripts/lib.sh から TEAM_ID を読めません"
  while read -r hash name; do
    ou="$(security find-certificate -c "$name" -p 2>/dev/null | openssl x509 -noout -subject -nameopt RFC2253 \
      | tr ',' '\n' | sed -n 's/^.*OU=//p' | head -1)"
    [[ "$ou" == "$team_id" ]] && return 0
  done < <(security find-identity -v -p codesigning \
    | sed -n 's/^ *[0-9]*) \([0-9A-F]*\) "\(Apple Development:.*\)"$/\1 \2/p')
  die "OU=$team_id の Apple Development 証明書がキーチェーンにありません（--signing adhoc なら不要）"
}

stage_prerequisites() {
  log "0. 前提の確認"
  local tool missing=""
  for tool in cargo shasum unzip tar; do
    command -v "$tool" >/dev/null || missing+=" $tool"
  done
  if [[ "$SKIP_APP" -eq 0 ]]; then
    for tool in swift xcodebuild codesign; do
      command -v "$tool" >/dev/null || missing+=" $tool"
    done
    if [[ "$WITH_LM" -eq 1 ]]; then
      command -v cmake >/dev/null || missing+=" cmake（brew install cmake。LM が要らなければ --no-lm）"
    fi
  fi
  if [[ "$DO_FETCH" -eq 1 ]]; then
    command -v curl >/dev/null || missing+=" curl"
  fi
  [[ -z "$missing" ]] || die "必要なコマンドがありません:$missing"
  if [[ "$SKIP_APP" -eq 0 && "$SIGNING" == team ]]; then check_team_certificate; fi

  local free_kb
  free_kb="$(df -k "$IKARI_ROOT" | awk 'NR == 2 { print $4 }')"
  if [[ -n "$free_kb" && "$free_kb" -lt "$MIN_FREE_KB" ]]; then
    die "ディスクの空きが足りません（$((free_kb / 1024)) MB。目安 $((MIN_FREE_KB / 1024)) MB）"
  fi
  mkdir -p "$IKARI_ROOT/data"
}

# ---------------------------------------------------------------- 1. 取得

stage_fetch() {
  local only=(--only sudachi --only chive) label="SudachiDict、chiVe"
  if [[ "$WITH_LM" -eq 1 ]]; then only+=(--only zenz); label+="、zenz"; fi
  log "1. 元データ（${label}）"
  if [[ "$DO_FETCH" -eq 1 ]]; then
    # ABR の町字マスター（任意の入力。ADR 0030）と町字・郵便番号変換表（任意。ADR 0032）は取得だけして、--check では要求しない
    run "$IKARI_SCRIPTS_DIR/fetch-data.sh" "${only[@]}" --only abr --only postal
  elif ! "$IKARI_SCRIPTS_DIR/fetch-data.sh" --check "${only[@]}"; then
    die "元データが足りません。scripts/setup.sh --fetch で取得できます（通信するのは --fetch を付けたときだけ。eval 用の AJIMEE は scripts/fetch-data.sh --only ajimee）"
  fi
}

# ---------------------------------------------------------------- 2. 辞書

stage_dict() {
  log "2. 辞書（vectors.bin を含む）"
  local args=()
  if [[ "$REBUILD_DICT" -eq 1 ]]; then args+=(--rebuild); fi
  run "$IKARI_SCRIPTS_DIR/build-dict.sh" ${args[@]+"${args[@]}"}
}

# ---------------------------------------------------------------- 3. データの配置

stage_data() {
  log "3. データの配置（~/Library/Application Support/ikari-ime/）"
  local args=(--src "$IKARI_ROOT/data")
  if [[ "$WITH_LM" -eq 0 ]]; then args+=(--no-model); fi
  run "$IKARI_SCRIPTS_DIR/install-data.sh" "${args[@]}"
}

# ---------------------------------------------------------------- 4. アプリ

stage_app() {
  log "4. アプリ（署名: ${SIGNING}）"
  local build_args=(--signing "$SIGNING")
  if [[ "$WITH_LM" -eq 0 ]]; then build_args+=(--no-lm); fi
  run "$MACOS_SCRIPTS/build.sh" "${build_args[@]}"
  if [[ "$NO_INSTALL" -eq 1 ]]; then
    show "$MACOS_SCRIPTS/install.sh --signing $SIGNING"
    show "（--no-install: ~/Library/Input Methods には入れない）"
  else
    run "$MACOS_SCRIPTS/install.sh" --signing "$SIGNING"
  fi
}

# ---------------------------------------------------------------- 5. smoke

stage_smoke() {
  log "5. smoke（インストールしたアプリ）"
  # ikari を使っている間は IME のエンジンが engine.lock を持つので、smoke は終了コード 2 で「実行できない」と出す
  printf '注意: ikari を入力ソースに選んでいると smoke は実行できません（ikari を ABC などに切り替えてから）\n'
  if [[ "$NO_INSTALL" -eq 1 ]]; then
    show "$MACOS_SCRIPTS/smoke.sh --signing $SIGNING --installed"
  else
    run "$MACOS_SCRIPTS/smoke.sh" --signing "$SIGNING" --installed
  fi
}

# ---------------------------------------------------------------- 実行

stage_prerequisites
stage_fetch
stage_dict
stage_data
if [[ "$SKIP_APP" -eq 0 ]]; then
  stage_app
  if [[ "$SKIP_SMOKE" -eq 0 ]]; then stage_smoke; fi
fi

log "完了"
if [[ "$SKIP_APP" -eq 0 && "$NO_INSTALL" -eq 0 ]]; then
  printf '入力ソースへの追加（初回だけ、手動）: README.md の「入力ソースの追加」を参照\n'
fi
