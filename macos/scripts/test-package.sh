#!/bin/bash
# 同梱のデータの封印の検査（lib-data.sh の bundled_data_seal_error。build.sh の verify_bundled_data が使う）と、
# build.sh の引数の検査のテスト（#583、ADR 0080 §3.4、§7.3）。
# 小さな偽の束（ad-hoc 署名）を作り、正しい束は通り、署名のあとの変更（1 バイトの書き換え、ファイルの追加・削除、
# 署名し直したうえでの差し替え、モデルの sha256 の食い違い）はすべて落ちることを確かめる。本物のビルドはしない。
# codesign が要るので macOS 以外では飛ばす。
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
# shellcheck source=lib-data.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib-data.sh"

if ! command -v codesign >/dev/null 2>&1; then
  echo "skip: codesign がありません（macOS だけで流す）"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
readonly TMP
failures=0
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }

# ---- 偽のデータの元（<data>/build/dict、<data>/raw/zenz）
MODEL_NAME="$(pinned_model_file)"
[[ -n "$MODEL_NAME" ]] || { echo "モデルのファイル名を表から読めません" >&2; exit 1; }
SRC="$TMP/src"
mkdir -p "$SRC/build/dict" "$SRC/raw/zenz"
for f in "${BUNDLED_DICT_REQUIRED[@]}" emoji.tsv BUILD_INFO; do printf 'dict-%s-payload-0123456789' "$f" >"$SRC/build/dict/$f"; done
printf 'fake-model-payload-0123456789' >"$SRC/raw/zenz/$MODEL_NAME"
FAKE_MODEL_SHA="$(sha256_file "$SRC/raw/zenz/$MODEL_NAME")"

# 偽の束を作って署名する（内側の XPC を先に、外側のアプリを後に）
resign() { # <app>
  local app="$1"
  codesign --force --options runtime --timestamp=none -s - "$app/$BUNDLED_XPC_REL" >/dev/null 2>&1
  codesign --force --options runtime --timestamp=none -s - "$app" >/dev/null 2>&1
}
make_app() { # <出力先の app>
  local app="$1" xpc
  xpc="$app/$BUNDLED_XPC_REL"
  mkdir -p "$app/Contents/MacOS" "$xpc/Contents/MacOS"
  cp /usr/bin/true "$app/Contents/MacOS/fake"
  cp /usr/bin/true "$xpc/Contents/MacOS/fakexpc"
  plutil -create xml1 "$app/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string test.ikari.fake "$app/Contents/Info.plist"
  plutil -insert CFBundleExecutable -string fake "$app/Contents/Info.plist"
  plutil -insert CFBundlePackageType -string APPL "$app/Contents/Info.plist"
  plutil -create xml1 "$xpc/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string test.ikari.fake.xpc "$xpc/Contents/Info.plist"
  plutil -insert CFBundleExecutable -string fakexpc "$xpc/Contents/Info.plist"
  plutil -insert CFBundlePackageType -string XPC! "$xpc/Contents/Info.plist"
  stage_bundled_data "$SRC" "$app/$BUNDLED_DATA_REL" "$app.sha256"
  resign "$app"
}

# 新しい偽の束を $TMP/<名前>/ikari.app に作り、その app のパスを出す
fresh() {
  local name="$1"
  mkdir -p "$TMP/$name"
  make_app "$TMP/$name/ikari.app"
  printf '%s' "$TMP/$name/ikari.app"
}

expect_ok() { # <名前> <app> [モデルの sha256]
  local name="$1" app="$2" problem
  if problem="$(bundled_data_seal_error "$app" "$app.sha256" ${3:+"$3"})"; then
    echo "ok: $name"
  else
    fail "$name が落ちた: $problem"
  fi
}
expect_ng() { # <名前> <理由に含まれる言葉> <app> [モデルの sha256]
  local name="$1" want="$2" app="$3" problem
  if problem="$(bundled_data_seal_error "$app" "$app.sha256" ${4:+"$4"})"; then
    fail "$name が通ってしまった"
  elif [[ "$problem" != *"$want"* ]]; then
    fail "$name の理由が想定と違う（'$want' を含まない）: $problem"
  else
    echo "ok: $name -> $problem"
  fi
}

# 表（resource/src/models.rs）から読めている
if [[ "$(pinned_model_sha256)" =~ ^[0-9a-f]{64}$ ]]; then echo "ok: 表のモデルの sha256 を読める"; else fail "表のモデルの sha256 を読めない"; fi

# ---- 正しい束
app="$(fresh good)"
expect_ok "正しい束（モデルの sha256 を合わせる）" "$app" "$FAKE_MODEL_SHA"
expect_ng "モデルの sha256 が表の値と違う（既定は表の値）" "表の値と違います" "$app"
expect_ng "モデルの sha256 を別の値で指定" "表の値と違います" "$app" "$(printf '%064d' 0)"
[[ -f "$app/$BUNDLED_DATA_REL/NOTICE.txt" ]] && echo "ok: NOTICE.txt を置いた" || fail "NOTICE.txt を置いていない"

# ---- 署名のあとの変更（署名し直さない）
app="$(fresh flip)"
printf 'X' | dd of="$app/$BUNDLED_DATA_REL/dict/system.dict" bs=1 seek=3 count=1 conv=notrunc 2>/dev/null
expect_ng "辞書の 1 バイトを書き換える" "署名を検証できません" "$app" "$FAKE_MODEL_SHA"

app="$(fresh flip-model)"
printf 'X' | dd of="$app/$BUNDLED_DATA_REL/models/$MODEL_NAME" bs=1 seek=0 count=1 conv=notrunc 2>/dev/null
expect_ng "モデルの 1 バイトを書き換える" "署名を検証できません" "$app" "$FAKE_MODEL_SHA"

app="$(fresh add)"
printf 'evil' >"$app/$BUNDLED_DATA_REL/dict/extra.bin"
expect_ng "ファイルを足す" "署名を検証できません" "$app" "$FAKE_MODEL_SHA"

app="$(fresh del)"
rm "$app/$BUNDLED_DATA_REL/dict/vectors.bin"
expect_ng "ファイルを消す" "署名を検証できません" "$app" "$FAKE_MODEL_SHA"

# ---- 署名し直された束（封印は通るので、一覧と表との照合で落ちること）
app="$(fresh resign-flip)"
printf 'X' | dd of="$app/$BUNDLED_DATA_REL/dict/grammar.bin" bs=1 seek=2 count=1 conv=notrunc 2>/dev/null
resign "$app"
expect_ng "書き換えて署名し直す" "sha256 が、置いたときと違います" "$app" "$FAKE_MODEL_SHA"

app="$(fresh resign-add)"
printf 'evil' >"$app/$BUNDLED_DATA_REL/dict/extra.bin"
resign "$app"
expect_ng "ファイルを足して署名し直す" "置いたときと違います" "$app" "$FAKE_MODEL_SHA"

app="$(fresh resign-del)"
rm "$app/$BUNDLED_DATA_REL/dict/vectors.bin"
resign "$app"
expect_ng "ファイルを消して署名し直す" "封印していません" "$app" "$FAKE_MODEL_SHA"

app="$(fresh resign-model)"
printf 'other-model-payload' >"$app/$BUNDLED_DATA_REL/models/$MODEL_NAME"
resign "$app"
expect_ng "モデルを差し替えて署名し直す" "sha256 が、置いたときと違います" "$app" "$FAKE_MODEL_SHA"

# 一覧も差し替えた場合は、表のモデルの sha256 が最後の砦になる
app="$(fresh resign-model-list)"
printf 'other-model-payload' >"$app/$BUNDLED_DATA_REL/models/$MODEL_NAME"
resign "$app"
sed -i '' "s|^[0-9a-f]\{64\}  models/|$(sha256_file "$app/$BUNDLED_DATA_REL/models/$MODEL_NAME")  models/|" "$app.sha256"
expect_ng "モデルと一覧を差し替えて署名し直す（表の sha256 で落ちる）" "表の値と違います" "$app" "$FAKE_MODEL_SHA"
expect_ok "同じ束でも、差し替えたモデルの sha256 を正とすれば通る（検査が値で決まる確認）" "$app" \
  "$(sha256_file "$app/$BUNDLED_DATA_REL/models/$MODEL_NAME")"

# ---- 一覧・場所がない
app="$(fresh nolist)"
rm "$app.sha256"
expect_ng "一覧がない" "一覧がありません" "$app" "$FAKE_MODEL_SHA"
app="$(fresh nodata)"
rm -rf "${app:?}/$BUNDLED_DATA_REL"
expect_ng "データの場所がない" "場所がありません" "$app" "$FAKE_MODEL_SHA"

# ---- データの元の検査（足りなければ落ちる。die は exit するので別のシェルで）
for missing in build/dict/system.dict "raw/zenz/$MODEL_NAME"; do
  src2="$TMP/src-missing"
  rm -rf "$src2"
  cp -R "$SRC" "$src2"
  rm "$src2/$missing"
  if (check_data_source "$src2") >/dev/null 2>&1; then fail "$missing が無い元が通ってしまった"; else echo "ok: $missing が無い元は落ちる"; fi
done
if (check_data_source "$SRC") >/dev/null 2>&1; then echo "ok: 揃った元は通る"; else fail "揃った元が落ちた"; fi

# ---- build.sh の引数（ビルドの前に落ちる）
if out="$("$IKARI_SCRIPTS_DIR/build.sh" --edition nope 2>&1)"; then
  fail "--edition nope が通ってしまった"
elif [[ "$out" == *"--edition は normal か offline"* ]]; then
  echo "ok: --edition nope は落ちる"
else
  fail "--edition nope の理由が想定と違う: $out"
fi
if out="$("$IKARI_SCRIPTS_DIR/build.sh" --data-dir "$TMP/does-not-exist" 2>&1)"; then
  fail "存在しない --data-dir が通ってしまった"
elif [[ "$out" == *"--data-dir がディレクトリではありません"* ]]; then
  echo "ok: 存在しない --data-dir は落ちる"
else
  fail "--data-dir の理由が想定と違う: $out"
fi

if [[ "$failures" -gt 0 ]]; then
  printf '%d 件失敗\n' "$failures" >&2
  exit 1
fi
echo "test-package: すべて通った"
