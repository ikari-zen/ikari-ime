#!/bin/bash
# 辞書とモデルを ~/Library/Application Support/ikari-ime/ に入れる（ADR 0004 §8.3）。
#
#   scripts/install-data.sh [--src <data のディレクトリ>] [--dest <ikari-ime のディレクトリ>] [--no-model] [--dry-run]
#
#   <src>/build/dict/{grammar.bin,system.dict,vectors.bin}            -> <dest>/dict/
#   <src>/build/dict/emoji.tsv（あれば。なくてもよい）               -> <dest>/dict/
#   <src>/build/dict/postal.tsv（あれば。なくてもよい）              -> <dest>/dict/
#   <src>/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf                         -> <dest>/models/
#
# コピーは同じディレクトリの一時ファイルに書いてから rename する。エンジンが mmap している最中のファイルを
# 直接書き換えない（古い inode はエンジンが閉じるまで生きる）。内容が同じファイルは飛ばす。
# --src の既定: このリポジトリの data/。なければ（git worktree のとき）本体のチェックアウトの data/。
# --dest の既定: ${IKARI_USER_DIR}（Rust 側と同じ名前）、なければ ~/Library/Application Support/ikari-ime。
#   旧名 $IKARI_DATA_DIR も当面は読む（警告を出す）。
set -euo pipefail

# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
ROOT="$IKARI_ROOT"
SRC=""
if [[ -n "${IKARI_USER_DIR:-}" ]]; then
  DEST="$IKARI_USER_DIR"
elif [[ -n "${IKARI_DATA_DIR:-}" ]]; then
  warn "IKARI_DATA_DIR は旧名です。IKARI_USER_DIR を使ってください"
  DEST="$IKARI_DATA_DIR"
else
  DEST="$APP_SUPPORT_DIR"
fi
WITH_MODEL=1
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --src) SRC="${2:-}"; shift ;;
    --dest) DEST="${2:-}"; shift ;;
    --no-model) WITH_MODEL=0 ;;
    --dry-run) DRY_RUN=1 ;;
    -h | --help) sed -n '2,14p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) die "不明な引数: $1" ;;
  esac
  shift
done

if [[ -z "$SRC" ]]; then
  if [[ -d "$ROOT/data/build/dict" ]]; then
    SRC="$ROOT/data"
  else
    # worktree: 共通の .git のあるチェックアウトの data/ を探す
    common="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    if [[ -n "$common" && -d "$(dirname "$common")/data/build/dict" ]]; then
      SRC="$(dirname "$common")/data"
    else
      die "data/build/dict が見つかりません。--src で指定してください"
    fi
  fi
fi
[[ -n "$DEST" ]] || die "--dest が空です"

# tmp に書いて rename する。同じ内容なら何もしない
install_file() { # <元> <先>
  local from="$1" to="$2" tmp
  [[ -f "$from" ]] || die "$from がありません"
  if [[ -f "$to" ]] && cmp -s "$from" "$to"; then
    printf 'unchanged  %s\n' "$to"
    return
  fi
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'would copy %s -> %s\n' "$from" "$to"
    return
  fi
  mkdir -p "$(dirname "$to")"
  tmp="$(mktemp "$to.tmp.XXXXXX")"
  trap 'rm -f "$tmp"' EXIT
  cp "$from" "$tmp"
  chmod 644 "$tmp"
  mv -f "$tmp" "$to"
  trap - EXIT
  printf 'installed  %s\n' "$to"
}

printf 'src:  %s\ndest: %s\n' "$SRC" "$DEST"
for f in "${DICT_FILES[@]}"; do
  install_file "$SRC/build/dict/$f" "$DEST/dict/$f"
done
# 絵文字の表は任意（ない環境では入れない。ショートコードが働かないだけ。ADR 0027 §8）
if [[ -f "$SRC/build/dict/$EMOJI_TABLE_FILE" ]]; then
  install_file "$SRC/build/dict/$EMOJI_TABLE_FILE" "$DEST/dict/$EMOJI_TABLE_FILE"
fi
# 住所と郵便番号の表も任意（ない環境では入れない。郵便番号の変換が働かないだけ。ADR 0032 §3）
if [[ -f "$SRC/build/dict/$POSTAL_TABLE_FILE" ]]; then
  install_file "$SRC/build/dict/$POSTAL_TABLE_FILE" "$DEST/dict/$POSTAL_TABLE_FILE"
fi
if [[ "$WITH_MODEL" -eq 1 ]]; then
  install_file "$SRC/raw/zenz/$ZENZ_MODEL_FILE" "$DEST/models/$ZENZ_MODEL_FILE"
fi
