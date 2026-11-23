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

# ---- シンボリックリンクも一覧の対象（署名し直しても、置いたときの一覧と違えば落ちる）
app="$(fresh symlink)"
ln -s system.dict "$app/$BUNDLED_DATA_REL/dict/link"
resign "$app"
expect_ng "シンボリックリンクを足して署名し直す" "置いたときと違います" "$app" "$FAKE_MODEL_SHA"

# ---- 版の印（macos/Frameworks/.edition。--skip-rust が使う）
mark="$TMP/edition-mark"
for want in normal offline; do
  other=offline
  [[ "$want" == normal ]] || other=normal
  rm -f "$mark"
  if problem="$(frameworks_edition_error "$mark" "$want")"; then fail "印が無いのに $want が通った"; elif [[ "$problem" == *"印（.edition）がありません"* ]]; then echo "ok: 印が無い（${want}）は落ちる"; else fail "印が無い（${want}）の理由が違う: $problem"; fi
  printf '%s\n' "$other" >"$mark"
  if problem="$(frameworks_edition_error "$mark" "$want")"; then fail "版が違うのに $want が通った"; elif [[ "$problem" == *"でビルドされています"* ]]; then echo "ok: 版が違う（印=${other}、要る=${want}）は落ちる"; else fail "版が違う（${want}）の理由が違う: $problem"; fi
  : >"$mark"
  if problem="$(frameworks_edition_error "$mark" "$want")"; then fail "空の印で $want が通った"; else echo "ok: 空の印（${want}）は落ちる"; fi
  printf '%s\n' "$want" >"$mark"
  if frameworks_edition_error "$mark" "$want" >/dev/null; then echo "ok: 版が合う（${want}）は通る"; else fail "版が合うのに $want が落ちた"; fi
done
# build_rust は、先頭で前の版の印を消す（途中で失敗しても印が残らない）
if awk '/^build_rust\(\) \{/{f=1; next} f&&/rm -f "\$FRAMEWORKS_EDITION_MARK"/{found=1} f&&/cargo build/{exit} END{exit !found}' "$IKARI_SCRIPTS_DIR/build.sh"; then
  echo "ok: build_rust は cargo の前に印を消す"
else
  fail "build_rust が cargo の前に印を消していない"
fi

# ---- network.* の検査（fail-closed。本物の束の代わりに、署名した小さな実行ファイルを並べる）
NET_ENT="$TMP/net.entitlements"
cat >"$NET_ENT" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.network.client</key><true/></dict></plist>
PLIST
SANDBOX_ENT="$TMP/sandbox.entitlements"
cat >"$SANDBOX_ENT" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.app-sandbox</key><true/></dict></plist>
PLIST
# 実行ファイルを置いて署名する（<パス> [entitlement のファイル]）
put_exe() {
  mkdir -p "$(dirname "$1")"
  cp /usr/bin/true "$1"
  if [[ -n "${2:-}" ]]; then
    codesign --force -s - --entitlements "$2" "$1" >/dev/null 2>&1
  else
    codesign --force -s - "$1" >/dev/null 2>&1
  fi
}
# 必須の 4 つ（殻、エンジン、設定アプリ、檻役）を置いた束
scan_app() { # <名前>
  local app="$TMP/scan-$1/ikari.app"
  put_exe "$app/Contents/MacOS/ikari"
  put_exe "$app/$BUNDLED_XPC_REL/Contents/MacOS/IkariEngine" "$SANDBOX_ENT"
  put_exe "$app/Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings"
  put_exe "$app/Contents/XPCServices/IkariLMCage.xpc/Contents/MacOS/ikari-lm-cage" "$SANDBOX_ENT"
  printf '%s' "$app"
}
scan_required() { # <app>
  printf '%s\n' "$1/Contents/MacOS/ikari" "$1/$BUNDLED_XPC_REL/Contents/MacOS/IkariEngine" \
    "$1/Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings" "$1/Contents/XPCServices/IkariLMCage.xpc/Contents/MacOS/ikari-lm-cage"
}
# 検査を流す: <名前> <期待（ok か、理由に含まれる言葉）> <app> <許す実行ファイル または 空> [PATH の前に足すディレクトリ]
scan_expect() {
  local name="$1" want="$2" app="$3" allowed="$4" extra_path="${5:-}" req out
  req=()
  while IFS= read -r line; do req+=("$line"); done < <(scan_required "$app")
  if out="$(PATH="${extra_path:+$extra_path:}$PATH" network_entitlement_error "$app" "$allowed" "${req[@]}")"; then
    if [[ "$want" == ok ]]; then echo "ok: ${name}（見た Mach-O: ${out}）"; else fail "$name が通ってしまった"; fi
  elif [[ "$want" == ok ]]; then
    fail "$name が落ちた: $out"
  elif [[ "$out" == *"$want"* ]]; then
    echo "ok: $name -> $out"
  else
    fail "$name の理由が想定と違う（'$want' を含まない）: $out"
  fi
}

app="$(scan_app good)"
scan_expect "network.* が無い束" ok "$app" ""
app="$(scan_app net-settings)"
put_exe "$app/Contents/Helpers/IkariSettings.app/Contents/MacOS/IkariSettings" "$NET_ENT"
scan_expect "設定アプリに network.client" "com.apple.security.network." "$app" ""
app="$(scan_app net-extra)"
put_exe "$app/Contents/Helpers/fake-fetch" "$NET_ENT"
scan_expect "offline の束に network.* を持つ偽の実行ファイル" "com.apple.security.network." "$app" ""
scan_expect "通常版で許した実行ファイルは通る" ok "$app" "$app/Contents/Helpers/fake-fetch"
app="$(scan_app net-other)"
put_exe "$app/Contents/Helpers/fake-fetch" "$NET_ENT"
scan_expect "許した実行ファイル以外の network.* は落ちる" "com.apple.security.network." "$app" "$app/Contents/Helpers/other"
app="$(scan_app net-symlink)"
put_exe "$app/Contents/Helpers/real-fetch" "$NET_ENT"
ln -s real-fetch "$app/Contents/Helpers/link-fetch"
scan_expect "リンクの先の network.*（実体で落ちる）" "com.apple.security.network." "$app" ""
app="$(scan_app missing)"
rm "$app/$BUNDLED_XPC_REL/Contents/MacOS/IkariEngine"
scan_expect "名指しした対象（エンジン）が束に無い" "検査の対象が束にありません" "$app" ""
app="$(scan_app not-macho)"
printf '#!/bin/sh\n' >"$app/Contents/MacOS/ikari"
scan_expect "名指しした対象が Mach-O でない" "Mach-O として見ていません" "$app" ""
app="$(scan_app unsigned)"
cp /usr/bin/true "$app/Contents/Helpers/unsigned-exe"
codesign --remove-signature "$app/Contents/Helpers/unsigned-exe" >/dev/null 2>&1 || true
scan_expect "署名の無い実行ファイル" "読めません" "$app" ""
# plutil / codesign が失敗する入力（偽のコマンドを PATH の先頭に置く）
mkdir -p "$TMP/fakebin-plutil" "$TMP/fakebin-codesign"
printf '#!/bin/sh\nexit 1\n' >"$TMP/fakebin-plutil/plutil"
chmod +x "$TMP/fakebin-plutil/plutil"
cat >"$TMP/fakebin-codesign/codesign" <<'SH'
#!/bin/sh
# entitlement の取得だけ、壊れた出力にする（plist として読めない）。それ以外は本物
case "$*" in
  *"-d --entitlements"*) printf 'this is not a plist'; exit 0 ;;
esac
exec /usr/bin/codesign "$@"
SH
chmod +x "$TMP/fakebin-codesign/codesign"
app="$(scan_app plutil-fail)"
scan_expect "壊れた entitlement（plist として読めない）" "plist として読めません" "$app" "" "$TMP/fakebin-codesign"
scan_expect "plutil が失敗する" "plist として読めません" "$app" "" "$TMP/fakebin-plutil"

# ---- make-dmg.sh は Smoke を含む束を断る
app="$(fresh smoke)"
cp /usr/bin/true "$app/Contents/MacOS/ikari-smoke"
if out="$("$IKARI_SCRIPTS_DIR/make-dmg.sh" --app "$app" --out "$TMP/smoke.dmg" 2>&1)"; then
  fail "Smoke を含む束が dmg になってしまった"
elif [[ "$out" == *"ikari-smoke があります"* && ! -e "$TMP/smoke.dmg" ]]; then
  echo "ok: Smoke を含む束は dmg にしない"
else
  fail "Smoke を含む束の断り方が想定と違う: $out"
fi

# ---- build.sh の引数（ビルドの前に落ちる）
# 値の無いオプションは、使い方を出して 64 で終わる
for opt in --edition --data-dir --signing; do
  rc=0
  out="$("$IKARI_SCRIPTS_DIR/build.sh" "$opt" 2>&1)" || rc=$?
  if [[ "$rc" -eq 64 && "$out" == *"$opt に値がありません"* ]]; then echo "ok: $opt に値が無いと 64"; else fail "$opt に値が無い: 終了コード ${rc}（64 のはず）: $out"; fi
done
rc=0
out="$("$IKARI_SCRIPTS_DIR/build.sh" --edition "" 2>&1)" || rc=$?
[[ "$rc" -eq 64 ]] && echo "ok: --edition に空の値は 64" || fail "--edition に空の値: 終了コード $rc"
# --dmg は --distribution が要る
if out="$("$IKARI_SCRIPTS_DIR/build.sh" --signing adhoc --dmg 2>&1)"; then
  fail "--dmg だけが通ってしまった"
elif [[ "$out" == *"--dmg は --distribution と一緒に使います"* ]]; then
  echo "ok: --dmg だけは落ちる"
else
  fail "--dmg の理由が想定と違う: $out"
fi

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
