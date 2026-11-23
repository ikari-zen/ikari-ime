#!/bin/bash
# ikari.app をビルドして署名する（ADR 0004 §9.2）。
#
#   macos/scripts/build.sh [--signing team|adhoc] [--release|--debug] [--edition normal|offline] [--data-dir <data>] [--dmg]
#                          [--no-lm] [--skip-rust] [--allow-stub] [--frameworks-only] [--distribution]
#
#   --signing team   Apple Development 証明書（チームID（`lib.sh` の `TEAM_ID`））で署名する。既定
#   --signing adhoc  ad-hoc 署名（署名要件は identifier の一致だけになる）
#   --release/--debug  Swift のビルド種別（Rust は常に release）。既定は release
#   --no-lm          llama.cpp（Metal）を入れない（cmake が使えないとき、LM が要らないとき）。LM の子（ikari-lm-host）も、檻役（IkariLMCage.xpc）も作らない
#   --skip-rust      cargo と XCFramework の作成を飛ばし、macos/Frameworks/ にあるものを使う（ikari-data は target/data/ のビルド済みを使う）
#   --allow-stub     XCFramework がなくてもビルドする（FFI なしのスタブのエンジンになる。インストールしてはいけない）。
#                    檻役の bin（ikari-lm-cage）が無いときは、これを付けても失敗する（--no-lm を付けるか、--skip-rust を外す）
#   --distribution   配布用（未署名の公開物。ADR 0069 §6、#40）。--signing adhoc が要る。Smoke を同梱せず、エンジンの許可リストからも外す。出力は <種別>-adhoc-dist/
#   --edition normal|offline  版（ADR 0080 §7.3）。既定は normal。offline は取得の部品（ikari-data）を作らず置かず、Rust の online の feature も付けない。
#                    束の中のどこにも network.* の entitlement が無いことを検査する。出力先は <…>-offline/、dmg の名前に -offline が付く
#   --data-dir <data>  同梱するデータ（辞書とモデル）の元（cargo xtask data install の --src と同じ並び。<data>/build/dict/、<data>/raw/zenz/）。
#                    束の中のエンジンの XPC の Resources/data/ に置き、署名のあと、封印とモデルの sha256 を検査する（ADR 0080 §3.4）。
#                    付けなければデータなしの束（開発用。今までどおり）
#   --dmg            署名と検査のあと、dmg を作る（macos/scripts/make-dmg.sh。出力は <出力先>/<名前>.dmg）
#   --frameworks-only  XCFramework（macos/Frameworks/）だけ作って終わる。Swift のビルド、組み立て、署名はしない（CI の swift test 用）
#
# 出力: target/macos/<release|debug>-<team|adhoc>/ikari.app
# 環境変数: IKARI_SIGN_IDENTITY で署名の ID（ハッシュまたは名前）を指定できる（team のとき。チームIDは検証する）
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
# shellcheck source=lib-data.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib-data.sh"

SIGNING=team
CONFIG=release
# エンジン（ikari-engine-ffi）に llama.cpp は入れない（ADR 0072 §5）。LM は子（ikari-lm-host）だけが持つ
WITH_LM=1
SKIP_RUST=0
ALLOW_STUB=0
FRAMEWORKS_ONLY=0
DISTRIBUTION=0
EDITION=normal
DATA_DIR=""
MAKE_DMG=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signing) SIGNING="${2:-}"; shift ;;
    --signing=*) SIGNING="${1#--signing=}" ;;
    --no-lm) WITH_LM=0 ;;
    --skip-rust) SKIP_RUST=1 ;;
    --allow-stub) ALLOW_STUB=1 ;;
    --frameworks-only) FRAMEWORKS_ONLY=1 ;;
    --distribution) DISTRIBUTION=1 ;;
    --edition) EDITION="${2:-}"; shift ;;
    --edition=*) EDITION="${1#--edition=}" ;;
    --data-dir) DATA_DIR="${2:-}"; shift ;;
    --data-dir=*) DATA_DIR="${1#--data-dir=}" ;;
    --dmg) MAKE_DMG=1 ;;
    -h | --help) sed -n '2,25p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) parse_common_flag "$1" || die "不明な引数: $1" ;;
  esac
  shift
done
validate_signing
case "$EDITION" in
  normal | offline) ;;
  *) die "--edition は normal か offline です: $EDITION" ;;
esac
[[ -z "$DATA_DIR" || -d "$DATA_DIR" ]] || die "--data-dir がディレクトリではありません: $DATA_DIR"
[[ "$FRAMEWORKS_ONLY" -eq 0 || "$SKIP_RUST" -eq 0 ]] || die "--frameworks-only と --skip-rust は同時に使えません"

[[ "$DISTRIBUTION" -eq 0 || "$SIGNING" == adhoc ]] || die "--distribution は --signing adhoc と一緒に使います（配布物は未署名。team の署名は開発用）"
[[ "$DISTRIBUTION" -eq 0 || "$ALLOW_STUB" -eq 0 ]] || die "--distribution は --allow-stub と一緒に使えません（配布物に FFI なしのスタブを入れない）"
OUT="$(bundle_out_dir)"
# 開発用の adhoc のビルドと混ざらないよう、出力先を分ける
[[ "$DISTRIBUTION" -eq 0 ]] || OUT="$OUT-dist"
# 通常版の出力先は今までどおり。オフライン版は別の場所に作る（混ざらないように）
EDITION_SUFFIX=""
[[ "$EDITION" == normal ]] || EDITION_SUFFIX="-$EDITION"
OUT="$OUT$EDITION_SUFFIX"
# エンジンの許可リストに Smoke を含めるか（配布用は含めない）
SHELL_REQ_TARGET=shell
[[ "$DISTRIBUTION" -eq 0 ]] || SHELL_REQ_TARGET=shell-distribution
APP="$OUT/$APP_NAME"
XPC="$APP/Contents/XPCServices/IkariEngine.xpc"
# 檻役（ADR 0077）。LM の子を起動して回収するだけの XPC サービス（App Sandbox だけ。子はこの Sandbox を引き継ぐ）
CAGE="$APP/Contents/XPCServices/IkariLMCage.xpc"
SETTINGS="$APP/Contents/Helpers/$SETTINGS_APP_NAME"
TEMPLATES="$IKARI_SCRIPTS_DIR/templates"
FRAMEWORKS="$IKARI_MACOS_DIR/Frameworks"
RUST_TARGET=aarch64-apple-darwin
# LM の推論を担う別プロセス（ADR 0060 Q4）。檻役の束（IkariLMCage.xpc/Contents/MacOS/）に置き、檻役が起動する（ADR 0077 §3.4）。
# App Sandbox は檻役から引き継ぐ（entitlement は付けない。ネットワークなし）
LM_HOST_NAME=ikari-lm-host
LM_HOST_BUILT="$IKARI_ROOT/target/lm/$RUST_TARGET/release/$LM_HOST_NAME"
# 檻役の実行ファイル（Rust の bin。llama.cpp は持たない）。LM の子とは別の target で作る（feature を混ぜない）
LM_CAGE_NAME=ikari-lm-cage
LM_CAGE_BUILT="$IKARI_ROOT/target/lm-cage/$RUST_TARGET/release/$LM_CAGE_NAME"
# 取得の部品（ADR 0079）。Contents/Helpers/ に置く単体の exe。Info.plist をリンクのときに埋めるので、
# 手元のビルドや LM の target と混ざらないよう、別の target で作る
DATA_NAME=ikari-data
DATA_BUILT="$IKARI_ROOT/target/data/$RUST_TARGET/release/$DATA_NAME"
DATA_DEST="$APP/Contents/Helpers/$DATA_NAME"
# 同梱するデータ（ADR 0080 §3.1）と、置いた物の一覧（封印の検査が使う。束の外に置く）
BUNDLED_DATA_DEST="$APP/$BUNDLED_DATA_REL"
BUNDLED_DATA_LIST="$OUT/bundled-data.sha256"
# どの版の Rust でビルドした XCFramework か（--skip-rust で別の版の物を使わないための印）
FRAMEWORKS_EDITION_MARK="$FRAMEWORKS/.edition"
VERSION="$(workspace_version)"
[[ -n "$VERSION" ]] || die "Cargo.toml から版を読めません"

# ---------------------------------------------------------------- 署名の ID

resolve_identity() {
  if [[ "$SIGNING" == adhoc ]]; then
    IDENTITY="-"
    return
  fi
  if [[ -n "${IKARI_SIGN_IDENTITY:-}" ]]; then
    IDENTITY="$IKARI_SIGN_IDENTITY"
    return
  fi
  # "Apple Development" の証明書のうち、subject.OU が TEAM_ID と一致するものを選ぶ
  # （CN の括弧内の ID はチームIDではない）
  local hash name ou
  while read -r hash name; do
    ou="$(security find-certificate -c "$name" -p 2>/dev/null | openssl x509 -noout -subject -nameopt RFC2253 \
      | tr ',' '\n' | sed -n 's/^.*OU=//p' | head -1)"
    if [[ "$ou" == "$TEAM_ID" ]]; then
      IDENTITY="$hash"
      return
    fi
  done < <(security find-identity -v -p codesigning \
    | sed -n 's/^ *[0-9]*) \([0-9A-F]*\) "\(Apple Development:.*\)"$/\1 \2/p')
  die "OU=$TEAM_ID の Apple Development 証明書がキーチェーンにありません（--signing adhoc なら不要）"
}

# ---------------------------------------------------------------- Rust

# 静的ライブラリが必要とするシステムのライブラリが、Package.swift の rustNativeLibs と一致することを確かめる
check_native_libs() {
  local reported="$1" declared needed missing="" extra="" item
  declared="$(sed -n '/^let rustNativeLibs/,/^\]/p' "$IKARI_MACOS_DIR/Package.swift" \
    | sed -n -e 's/.*linkedFramework("\([^"]*\)").*/-framework \1/p' -e 's/.*linkedLibrary("\([^"]*\)").*/-l\1/p' | sort -u)"
  # Swift が自動でリンクするもの
  local implicit=" -lSystem -lc -lm -liconv -lobjc -lresolv -lutil -lpthread -ldl "
  needed="$(printf '%s\n' "$reported" | tr ' ' '\n' \
    | awk '$0 == "-framework" { getline n; print "-framework " n; next } /^-l/ { print }' | sort -u)"
  while IFS= read -r item; do
    [[ -z "$item" ]] && continue
    [[ "$implicit" == *" $item "* ]] && continue
    printf '%s\n' "$declared" | grep -qxF -- "$item" || missing+=" [$item]"
  done <<<"$needed"
  while IFS= read -r item; do
    [[ -z "$item" ]] && continue
    printf '%s\n' "$needed" | grep -qxF -- "$item" || extra+=" [$item]"
  done <<<"$declared"
  [[ -z "$extra" ]] || warn "Package.swift の rustNativeLibs にあるが Rust は要求していない:$extra"
  [[ -z "$missing" ]] || die "Rust が要求するライブラリが Package.swift の rustNativeLibs にありません:$missing
  rustc の出力: $reported"
}

# xcodebuild -create-xcframework 用に、ヘッダと module.modulemap を置いたディレクトリを作る
stage_headers() { # <ヘッダのディレクトリ> <ヘッダ名> <モジュール名> <出力>
  rm -rf "${4:?}" && mkdir -p "$4/$3"
  cp "$1/$2" "$4/$3/"
  printf 'module %s {\n    header "%s"\n    export *\n}\n' "$3" "$2" >"$4/$3/module.modulemap"
}

make_xcframework() { # <.a> <ヘッダのディレクトリ> <ヘッダ名> <モジュール名> <出力 .xcframework>
  local staged="$OUT/headers/$4"
  stage_headers "$2" "$3" "$4" "$staged"
  rm -rf "$5"
  xcodebuild -create-xcframework -library "$1" -headers "$staged" -output "$5" >/dev/null
}

# 檻役の bin（llama.cpp なし。ADR 0077 §3.4）。LM の子と feature が違うので、target のディレクトリを分ける
build_cage() {
  log "cargo build: ${LM_CAGE_NAME}（llama.cpp なし）"
  (cd "$IKARI_ROOT" && CARGO_TARGET_DIR="$IKARI_ROOT/target/lm-cage" cargo build --release --locked \
    --target "$RUST_TARGET" -p ikari-lm --bin "$LM_CAGE_NAME") || die "$LM_CAGE_NAME をビルドできません"
  [[ -x "$LM_CAGE_BUILT" ]] || die "$LM_CAGE_BUILT がありません"
}

# ikari-data（ADR 0079 §3.2）。Info.plist（DATA_ID。版を埋めたもの）をリンクのときに __TEXT,__info_plist へ埋める
# （埋めない Sandbox の exe は起動の直後に落ちる。ADR 0076 §16）。--frameworks-only では要らない
build_data() {
  local plist="$OUT/$DATA_NAME.Info.plist"
  mkdir -p "$OUT"
  cp "$TEMPLATES/$DATA_NAME.Info.plist" "$plist"
  render_identity "$plist"
  plist_set_string "$plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$plist" CFBundleVersion "$VERSION"
  [[ "$(plutil -extract CFBundleIdentifier raw "$plist")" == "$DATA_ID" ]] \
    || die "$plist の CFBundleIdentifier が $DATA_ID ではありません（templates/$DATA_NAME.Info.plist と lib.sh の DATA_ID を揃える）"
  log "cargo rustc: ${DATA_NAME}（Info.plist を埋め込む）"
  (cd "$IKARI_ROOT" && CARGO_TARGET_DIR="$IKARI_ROOT/target/data" cargo rustc --release --locked \
    --target "$RUST_TARGET" -p ikari-data --bin "$DATA_NAME" -- \
    -C "link-arg=-Wl,-sectcreate,__TEXT,__info_plist,$plist") || die "$DATA_NAME をビルドできません"
  [[ -x "$DATA_BUILT" ]] || die "$DATA_BUILT がありません"
}

build_rust() {
  # 通常版は取得の核を持つ（online。ADR 0080 §7.1）。オフライン版は付けない（付け忘れも通信の少ない側に倒れる）
  local shell_features=()
  [[ "$EDITION" == offline ]] || shell_features=(--features online)
  log "cargo build: ikari-shell-ffi (edition: $EDITION${shell_features[*]:+, features: online})"
  (cd "$IKARI_ROOT" && cargo build --release --locked --target "$RUST_TARGET" -p ikari-shell-ffi ${shell_features[@]+"${shell_features[@]}"})

  log "cargo rustc: ikari-engine-ffi（llama.cpp なし）"
  local output
  output="$(cd "$IKARI_ROOT" && CARGO_TARGET_DIR="$IKARI_ROOT/target/lm" cargo rustc --release --locked \
    --target "$RUST_TARGET" -p ikari-engine-ffi \
    --lib --crate-type staticlib -- --print native-static-libs 2>&1)" || { printf '%s\n' "$output" >&2; die "cargo rustc が失敗しました"; }
  printf '%s\n' "$output" | tail -3
  local reported
  reported="$(printf '%s\n' "$output" | sed -n 's/.*native-static-libs: //p' | tail -1)"
  [[ -n "$reported" ]] || die "native-static-libs が出力されませんでした"
  check_native_libs "$reported"

  if [[ "$WITH_LM" == 1 ]]; then
    log "cargo build: $LM_HOST_NAME (features: metal)"
    (cd "$IKARI_ROOT" && CARGO_TARGET_DIR="$IKARI_ROOT/target/lm" cargo build --release --locked \
      --target "$RUST_TARGET" -p ikari-lm --bin "$LM_HOST_NAME" --features metal)
    [[ -x "$LM_HOST_BUILT" ]] || die "$LM_HOST_BUILT がありません"
  fi

  if [[ "$WITH_LM" == 1 ]]; then build_cage; fi

  log "XCFramework"
  mkdir -p "$FRAMEWORKS" "$OUT"
  make_xcframework "$IKARI_ROOT/target/$RUST_TARGET/release/libikari_shell_ffi.a" \
    "$IKARI_ROOT/shell-ffi/include" ikari_shell.h IkariShellFFI "$FRAMEWORKS/IkariShellFFI.xcframework"
  make_xcframework "$IKARI_ROOT/target/lm/$RUST_TARGET/release/libikari_engine_ffi.a" \
    "$IKARI_ROOT/engine-ffi/include" ikari_engine.h IkariEngineFFI "$FRAMEWORKS/IkariEngineFFI.xcframework"
  # 作り終えた印。check の鮮度の検査が、Cargo.toml だけの変更（cargo が作り直さないもの）を誤検知しないために使う（#296）
  touch "$FRAMEWORKS/.built"
  printf '%s\n' "$EDITION" >"$FRAMEWORKS_EDITION_MARK"
}

# ---------------------------------------------------------------- Swift とバンドル

build_swift() {
  # Package.resolved の版だけを使う（解き直さない）
  local SWIFT_LOCK_FLAGS=(--only-use-versions-from-resolved-file)
  "$IKARI_SCRIPTS_DIR/gen-identity.sh"
  log "swift build ($CONFIG)"
  mkdir -p "$OUT"
  local build_log="$OUT/swift-build.log"
  # 配るプロダクトだけをビルドする
  # `--product` を並べると最後の 1 つしかビルドされない（swift 6.2）ので、1 つずつビルドする
  local product
  : >"$build_log"
  for product in ikari IkariEngine ikari-smoke IkariSettings; do
    swift build -c "$CONFIG" --manifest-cache none "${SWIFT_LOCK_FLAGS[@]}" --package-path "$IKARI_MACOS_DIR" --product "$product" >>"$build_log" 2>&1 \
      || { tail -40 "$build_log" >&2; die "swift build が失敗しました（${build_log}）"; }
  done
  tail -1 "$build_log"
  BIN_DIR="$(swift build -c "$CONFIG" --manifest-cache none "${SWIFT_LOCK_FLAGS[@]}" --package-path "$IKARI_MACOS_DIR" --show-bin-path)"
  for product in ikari IkariEngine ikari-smoke IkariSettings; do
    [[ -x "$BIN_DIR/$product" ]] || die "$BIN_DIR/$product がありません"
  done
}

# 設定アプリ「ikari 設定」を Contents/Helpers/ に置く（ADR 0005 §2、§6.2）。
# Resources: アプリのアイコン（macos/Resources/settings/AppIcon.icns）、ライセンス表示（NFR-43）、
# キー割り当てのプリセット（上書きが「プリセットと同じ」かを画面が判定するため。shell-core/resources/keymap。OS 共通のプリセット）
assemble_settings_app() {
  local resources="$SETTINGS/Contents/Resources"
  cp "$TEMPLATES/IkariSettings.Info.plist" "$SETTINGS/Contents/Info.plist"
  render_identity "$SETTINGS/Contents/Info.plist"
  plist_set_string "$SETTINGS/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$SETTINGS/Contents/Info.plist" CFBundleVersion "$VERSION"
  plist_set_string "$SETTINGS/Contents/Info.plist" IkariSigningMode "$SIGNING"
  local icon="$IKARI_MACOS_DIR/Resources/settings/AppIcon.icns"
  [[ -f "$icon" ]] || die "$icon がありません（swift macos/tools/make-app-icon.swift で作る）"
  cp "$icon" "$resources/AppIcon.icns"
  [[ -f "$IKARI_ROOT/THIRD_PARTY_NOTICES.md" ]] || die "THIRD_PARTY_NOTICES.md がありません"
  # 「ライセンス」画面は THIRD_PARTY_NOTICES を 1 つ表示するので、Rust 依存の一覧（cargo xtask licenses の生成物）を後ろにつないで置く（#242）
  [[ -f "$IKARI_ROOT/THIRD_PARTY_LICENSES_RUST.md" ]] || die "THIRD_PARTY_LICENSES_RUST.md がありません（cargo xtask licenses）"
  { cat "$IKARI_ROOT/THIRD_PARTY_NOTICES.md"; printf "\n\n"; cat "$IKARI_ROOT/THIRD_PARTY_LICENSES_RUST.md"; } > "$resources/THIRD_PARTY_NOTICES.md"
  [[ -d "$IKARI_ROOT/shell-core/resources/keymap" ]] || die "shell-core/resources/keymap がありません"
  cp -R "$IKARI_ROOT/shell-core/resources/keymap" "$resources/keymap"
}

# 同梱するデータ（辞書、モデル、NOTICE）を、エンジンの XPC の束の中に置く（ADR 0080 §3.1、§3.4。--data-dir のときだけ）。
# 署名（sign_bundle のエンジンの署名）が封印する
assemble_bundled_data() {
  if [[ -z "$DATA_DIR" ]]; then
    warn "--data-dir がありません: 辞書とモデルを同梱しない束です（開発用。配る物には付ける）"
    rm -f "$BUNDLED_DATA_LIST"
    return
  fi
  log "同梱のデータを置く: $DATA_DIR -> $BUNDLED_DATA_DEST"
  stage_bundled_data "$DATA_DIR" "$BUNDLED_DATA_DEST" "$BUNDLED_DATA_LIST"
}

assemble_bundle() {
  log "バンドルの組み立て: $APP"
  rm -rf "$APP"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$XPC/Contents/MacOS" \
    "$SETTINGS/Contents/MacOS" "$SETTINGS/Contents/Resources"
  [[ "$WITH_LM" == 0 ]] || mkdir -p "$CAGE/Contents/MacOS"

  cp "$BIN_DIR/ikari" "$APP/Contents/MacOS/ikari"
  mkdir -p "$APP/Contents/Helpers"
  if [[ "$EDITION" == normal ]]; then
    [[ -x "$DATA_BUILT" ]] || die "$DATA_BUILT がありません（--skip-rust を外してビルドする）"
    cp "$DATA_BUILT" "$DATA_DEST"
  fi
  [[ "$DISTRIBUTION" -eq 1 ]] || cp "$BIN_DIR/ikari-smoke" "$APP/Contents/MacOS/ikari-smoke"
  cp "$BIN_DIR/IkariEngine" "$XPC/Contents/MacOS/IkariEngine"
  # 檻役と、その束の中の LM の子（檻役は自分の束の中の ikari-lm-host だけを起動する。エンジンの束には置かない。ADR 0077 §3.4）。
  # 檻役が無ければ、エンジンは「口が来ない → LM なし」で動く
  if [[ "$WITH_LM" == 1 ]]; then
    [[ -x "$LM_HOST_BUILT" ]] || die "$LM_HOST_BUILT がありません（--skip-rust を外してビルドするか、--no-lm を付ける）"
    [[ -x "$LM_CAGE_BUILT" ]] || die "$LM_CAGE_BUILT がありません（--skip-rust を外してビルドするか、--no-lm を付ける）"
    cp "$LM_HOST_BUILT" "$CAGE/Contents/MacOS/$LM_HOST_NAME"
    cp "$LM_CAGE_BUILT" "$CAGE/Contents/MacOS/$LM_CAGE_NAME"
  fi
  cp "$BIN_DIR/IkariSettings" "$SETTINGS/Contents/MacOS/IkariSettings"

  cp "$TEMPLATES/ikari.Info.plist" "$APP/Contents/Info.plist"
  render_identity "$APP/Contents/Info.plist"
  plist_set_string "$APP/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$APP/Contents/Info.plist" CFBundleVersion "$VERSION"
  plist_set_string "$APP/Contents/Info.plist" IkariSigningMode "$SIGNING"

  # XPC サービスが相手に求める署名要件は、IkariXPC の SigningRequirements が唯一の出どころ
  local peer
  peer="$("$BIN_DIR/ikari-smoke" --print-requirement "$SHELL_REQ_TARGET" "$SIGNING")"
  [[ -n "$peer" ]] || die "署名要件を取得できませんでした"
  cp "$TEMPLATES/engine.Info.plist" "$XPC/Contents/Info.plist"
  render_identity "$XPC/Contents/Info.plist"
  plist_set_string "$XPC/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$XPC/Contents/Info.plist" CFBundleVersion "$VERSION"
  plist_set_string "$XPC/Contents/Info.plist" IkariSigningMode "$SIGNING"
  plist_set_string "$XPC/Contents/Info.plist" IkariPeerRequirement "$peer"
  plist_set_string "$XPC/Contents/Info.plist" IkariCagePeerRequirement "$("$BIN_DIR/ikari-smoke" --print-requirement lm-cage "$SIGNING")"

  if [[ "$WITH_LM" == 1 ]]; then
    # 檻役の名前の口は殻（開発では smoke も）、匿名の口はエンジンだけ。要件の出どころは SigningRequirements
    cp "$TEMPLATES/lm-cage.Info.plist" "$CAGE/Contents/Info.plist"
    render_identity "$CAGE/Contents/Info.plist"
    plist_set_string "$CAGE/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
    plist_set_string "$CAGE/Contents/Info.plist" CFBundleVersion "$VERSION"
    plist_set_string "$CAGE/Contents/Info.plist" IkariSigningMode "$SIGNING"
    plist_set_string "$CAGE/Contents/Info.plist" IkariPeerRequirement "$peer"
    plist_set_string "$CAGE/Contents/Info.plist" IkariCageEnginePeerRequirement "$("$BIN_DIR/ikari-smoke" --print-requirement engine "$SIGNING")"
  fi

  assemble_settings_app
  assemble_bundled_data

  # Resources: keymap のプリセット（OS 共通。shell-core/resources/keymap）と、macOS 固有のもの。icons/ は直下に平らに置く（Info.plist のアイコン名は直下を指す）
  local res="$IKARI_MACOS_DIR/Resources"
  cp -R "$IKARI_ROOT/shell-core/resources/keymap" "$APP/Contents/Resources/keymap"
  if [[ -d "$res" ]]; then
    [[ ! -d "$res/icons" ]] || find "$res/icons" -type f -exec cp {} "$APP/Contents/Resources/" \;
    find "$res" -maxdepth 1 -type f -exec cp {} "$APP/Contents/Resources/" \;
    # モードの表示名（ja.lproj / en.lproj の InfoPlist.strings）はディレクトリごと置く（ADR 0005 §3.3）
    local lproj
    for lproj in "$res"/*.lproj; do
      [[ ! -d "$lproj" ]] || cp -R "$lproj" "$APP/Contents/Resources/"
    done
    local strings
    for strings in "$APP"/Contents/Resources/*.lproj/InfoPlist.strings; do
      [[ ! -f "$strings" ]] || render_identity "$strings"
    done
  fi
  local icon
  for icon in ikari.pdf ikari-japanese.pdf ikari-english.pdf; do
    [[ -f "$APP/Contents/Resources/$icon" ]] || warn "$icon がありません（macos/Resources/icons/ に置く）"
  done
}

# ---------------------------------------------------------------- 署名と検証

sign_bundle() {
  log "署名: $SIGNING (${IDENTITY})"
  local common=(--force --options runtime --timestamp=none -s "$IDENTITY")
  # 内側から
  [[ "$DISTRIBUTION" -eq 1 ]] || codesign "${common[@]}" -i "$SMOKE_ID" "$APP/Contents/MacOS/ikari-smoke"
  # 取得の部品: Hardened Runtime、App Sandbox、通信（client）、fetch/ と inbox/ の一時例外（ADR 0079 §3.2、§3.3）。ikari.app の署名が封印する
  if [[ "$EDITION" == normal ]]; then
    codesign "${common[@]}" -i "$DATA_ID" --entitlements "$TEMPLATES/$DATA_NAME.entitlements" "$DATA_DEST"
  fi
  # LM の子: Hardened Runtime。entitlement は付けない（起動した檻役の Sandbox をそのまま引き継ぐ）
  if [[ -f "$CAGE/Contents/MacOS/$LM_HOST_NAME" ]]; then
    codesign "${common[@]}" -i "$LM_HOST_ID" "$CAGE/Contents/MacOS/$LM_HOST_NAME"
  fi
  # 檻役: Hardened Runtime、App Sandbox だけ（network も一時例外もなし）。子の署名を封印する。エンジンより先に署名する
  if [[ -d "$CAGE" ]]; then
    codesign "${common[@]}" -i "$LM_CAGE_ID" --entitlements "$TEMPLATES/lm-cage.entitlements" "$CAGE"
  fi
  codesign "${common[@]}" --entitlements "$TEMPLATES/engine.entitlements" "$XPC"
  # 設定アプリ: Hardened Runtime（--options runtime）、Sandbox なし。identifier は署名要件（SigningRequirements.settingsApp）が見る
  codesign "${common[@]}" -i "$SETTINGS_ID" --entitlements "$TEMPLATES/settings.entitlements" "$SETTINGS"
  codesign "${common[@]}" "$APP"
}

entitlements_of() { codesign -d --entitlements - --xml "$1" 2>/dev/null; }

verify_bundle() {
  log "検証"
  codesign --verify --deep --strict --verbose=2 "$APP"

  # エンジン: app-sandbox があり、network がない
  local ents
  ents="$(entitlements_of "$XPC")"
  [[ -n "$ents" ]] || die "エンジンの entitlement を読めません"
  local sandbox
  sandbox="$(printf '%s' "$ents" | plutil -extract 'com\.apple\.security\.app-sandbox' raw - 2>/dev/null || true)"
  [[ "$sandbox" == true ]] || die "エンジンに com.apple.security.app-sandbox がありません"
  if printf '%s' "$ents" | plutil -convert xml1 -o - - | grep -q 'com\.apple\.security\.network'; then
    die "エンジンに com.apple.security.network.* があります"
  fi
  printf '%s' "$ents" | plutil -extract 'com\.apple\.security\.temporary-exception\.files\.home-relative-path\.read-write' xml1 -o - - \
    | grep -qF '<string>/Library/Application Support/ikari-ime/</string>' || die "エンジンのデータ用の一時例外がありません"
  # キーはちょうど 2 つ、一時例外は 1 要素だけ、Hardened Runtime が付いている
  local engine_keys engine_exc_count engine_info
  engine_keys="$(printf '%s' "$ents" | plutil -convert xml1 -o - - | grep -c '<key>' || true)"
  [[ "$engine_keys" -eq 2 ]] || die "エンジンの entitlement が 2 つではありません（キーが $engine_keys 個）"
  engine_exc_count="$(printf '%s' "$ents" | plutil -extract 'com\.apple\.security\.temporary-exception\.files\.home-relative-path\.read-write' raw - 2>/dev/null || true)"
  [[ "$engine_exc_count" == 1 ]] || die "エンジンの一時例外が 1 つではありません: '$engine_exc_count'"
  engine_info="$(codesign -dv "$XPC" 2>&1)"
  printf "%s\n" "$engine_info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || die "エンジンに Hardened Runtime（runtime）が付いていません"
  echo "engine entitlements: app-sandbox=true, network=none, temporary-exception=~/Library/Application Support/ikari-ime/ (2 keys)"

  # エンジンに llama.cpp が入っていない（ADR 0072 §5）。入っていれば、子が無くても LM がエンジンの中で動く
  if grep -aqE 'llama_model_load|ggml_backend' "$XPC/Contents/MacOS/IkariEngine"; then
    die "エンジン（IkariEngine）に llama.cpp が入っています（LM は子だけが持つ）"
  fi
  echo "engine: no llama.cpp linked"

  # エンジンの束に LM の子が無い（檻役の束に移した。ADR 0077 §3.4）
  [[ ! -e "$XPC/Contents/MacOS/$LM_HOST_NAME" ]] || die "エンジンの束に $LM_HOST_NAME があります（檻役の束だけに置く）"

  if [[ "$WITH_LM" == 1 ]]; then verify_cage; fi

  # 殻は Sandbox なし
  if entitlements_of "$APP" | grep -q 'com\.apple\.security\.app-sandbox'; then
    die "殻（ikari.app）に app-sandbox が付いています"
  fi
  local shell_info
  # 殻は entitlement を持たず、Hardened Runtime が付いている
  [[ -z "$(entitlements_of "$APP")" ]] || die "殻（ikari.app）に entitlement があります（付けない）"
  shell_info="$(codesign -dv "$APP" 2>&1)"
  printf "%s\n" "$shell_info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || die "殻に Hardened Runtime（runtime）が付いていません"

  # 署名要件の文字列が、実際の署名に当てはまること
  local engine_req shell_req
  engine_req="$("$BIN_DIR/ikari-smoke" --print-requirement engine "$SIGNING")"
  shell_req="$("$BIN_DIR/ikari-smoke" --print-requirement "$SHELL_REQ_TARGET" "$SIGNING")"
  codesign --verify -R="$engine_req" "$XPC" || die "エンジンが署名要件を満たしません: $engine_req"
  codesign --verify -R="$shell_req" "$APP" || die "殻が署名要件を満たしません: $shell_req"
  if [[ "$DISTRIBUTION" -eq 1 ]]; then
    # 配布用: Smoke が無く、エンジンと檻役の許可リスト（IkariPeerRequirement）にも smoke の識別子が無い
    [[ ! -e "$APP/Contents/MacOS/ikari-smoke" ]] || die "配布用のバンドルに ikari-smoke があります"
    local plists=("$APP/Contents/Info.plist" "$XPC/Contents/Info.plist")
    [[ ! -d "$CAGE" ]] || plists+=("$CAGE/Contents/Info.plist")
    ! grep -qF "$SMOKE_ID" "${plists[@]}" \
      || die "配布用のバンドルの Info.plist に smoke の識別子があります"
    ! printf '%s' "$shell_req" | grep -qF smoke || die "配布用の署名要件に smoke があります: $shell_req"
    echo "signing requirements satisfied (engine, shell); distribution: no smoke in bundle or allow-list"
  else
    codesign --verify -R="$shell_req" "$APP/Contents/MacOS/ikari-smoke" || die "smoke が署名要件を満たしません: $shell_req"
    echo "signing requirements satisfied (engine, shell, smoke)"
  fi

  if [[ "$SIGNING" == team ]]; then
    local team
    for target in "$APP" "$XPC" "$APP/Contents/MacOS/ikari-smoke"; do
      team="$(codesign -dv "$target" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$team" == "$TEAM_ID" ]] || die "$target のチームID が $TEAM_ID ではありません: '$team'"
    done
    echo "TeamIdentifier=$TEAM_ID (app, engine, smoke)"
  fi
}

# 檻役（ADR 0077 §3.4）: App Sandbox だけ（network も一時例外もなし）、LM の子は entitlement なしで檻役の署名に封印されている、
# 署名要件が殻（名前の口）とエンジン（匿名の口）を指す
verify_cage() {
  local lm_host="$CAGE/Contents/MacOS/$LM_HOST_NAME" cage_exe="$CAGE/Contents/MacOS/$LM_CAGE_NAME"
  [[ -x "$cage_exe" ]] || die "檻役（${LM_CAGE_NAME}）が $CAGE にありません"
  [[ -x "$lm_host" ]] || die "LM の子（${LM_HOST_NAME}）が檻役の束にありません"
  codesign --verify --deep --strict "$CAGE" || die "檻役の署名を検証できません"

  local info identifier host_info
  info="$(codesign -dv "$CAGE" 2>&1)"
  identifier="$(printf '%s\n' "$info" | sed -n 's/^Identifier=//p')"
  [[ "$identifier" == "$LM_CAGE_ID" ]] || die "檻役の identifier が $LM_CAGE_ID ではありません: '$identifier'"
  printf '%s\n' "$info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || die "檻役に Hardened Runtime（runtime）が付いていません"

  # entitlement は app-sandbox の 1 つだけ（network も一時例外もない）
  local ents keys
  ents="$(entitlements_of "$CAGE")"
  [[ -n "$ents" ]] || die "檻役の entitlement を読めません"
  [[ "$(printf '%s' "$ents" | plutil -extract 'com\.apple\.security\.app-sandbox' raw - 2>/dev/null || true)" == true ]] \
    || die "檻役に com.apple.security.app-sandbox がありません"
  keys="$(printf '%s' "$ents" | plutil -convert xml1 -o - - | grep -c '<key>')"
  [[ "$keys" -eq 1 ]] || die "檻役の entitlement が app-sandbox の 1 つだけではありません（キーが $keys 個）"
  echo "lm cage entitlements: app-sandbox=true only (no network, no temporary-exception)"

  # LM の子: entitlement なし、Hardened Runtime、檻役の署名に封印されている
  [[ -z "$(entitlements_of "$lm_host")" ]] || die "LM の子に entitlement があります（檻役の Sandbox を引き継ぐので、付けない）"
  codesign --verify --strict "$lm_host" || die "LM の子の署名を検証できません"
  host_info="$(codesign -dv "$lm_host" 2>&1)"
  printf '%s\n' "$host_info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || die "LM の子に Hardened Runtime（runtime）が付いていません"
  grep -q "MacOS/$LM_HOST_NAME" "$CAGE/Contents/_CodeSignature/CodeResources" || die "檻役の署名が $LM_HOST_NAME を封印していません"
  grep -q "XPCServices/IkariLMCage.xpc" "$APP/Contents/_CodeSignature/CodeResources" || die "ikari.app の署名が IkariLMCage.xpc を封印していません"
  # 檻役は llama.cpp を持たない（LM は子だけが持つ）
  if grep -aqE 'llama_model_load|ggml_backend' "$cage_exe"; then die "檻役（${LM_CAGE_NAME}）に llama.cpp が入っています"; fi
  echo "lm host: no entitlements (inherits the cage sandbox), sealed by the cage; cage: no llama.cpp"

  # 署名要件: 檻役が、自分の名前の口（殻）と匿名の口（エンジン）の要件を持ち、相手が檻役の要件を満たす
  local cage_req engine_req
  cage_req="$("$BIN_DIR/ikari-smoke" --print-requirement lm-cage "$SIGNING")"
  engine_req="$("$BIN_DIR/ikari-smoke" --print-requirement engine "$SIGNING")"
  codesign --verify -R="$cage_req" "$CAGE" || die "檻役が署名要件を満たしません: $cage_req"
  [[ "$(plutil -extract IkariPeerRequirement raw "$CAGE/Contents/Info.plist")" == "$("$BIN_DIR/ikari-smoke" --print-requirement "$SHELL_REQ_TARGET" "$SIGNING")" ]] \
    || die "檻役の IkariPeerRequirement（名前の口の相手 = 殻）が署名要件と違います"
  [[ "$(plutil -extract IkariCageEnginePeerRequirement raw "$CAGE/Contents/Info.plist")" == "$engine_req" ]] \
    || die "檻役の IkariCageEnginePeerRequirement（匿名の口の相手 = エンジン）が署名要件と違います"
  [[ "$(plutil -extract IkariCagePeerRequirement raw "$XPC/Contents/Info.plist")" == "$cage_req" ]] \
    || die "エンジンの IkariCagePeerRequirement（口の先 = 檻役）が署名要件と違います"
  # 読む側（檻役の bin、エンジンの殻）が、ここで埋めたキーと同じ名前を読む（キーの名前の食い違いを防ぐ）
  local key
  for key in IkariPeerRequirement IkariCageEnginePeerRequirement; do
    grep -aqF "$key" "$cage_exe" || die "檻役（${LM_CAGE_NAME}）が Info.plist のキー $key を読みません"
  done
  grep -aqF IkariCagePeerRequirement "$XPC/Contents/MacOS/IkariEngine" || die "エンジンが Info.plist のキー IkariCagePeerRequirement を読みません"
  echo "signing requirements satisfied (lm cage); cage allow-lists: shell (named), engine (anonymous)"

  if [[ "$SIGNING" == team ]]; then
    local target team
    for target in "$CAGE" "$lm_host"; do
      team="$(codesign -dv "$target" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$team" == "$TEAM_ID" ]] || die "$target のチームID が $TEAM_ID ではありません: '$team'"
    done
    echo "TeamIdentifier=$TEAM_ID (lm cage, lm host)"
  fi
}

# ikari-data（ADR 0079 §3.7）: 署名と Info.plist の identifier が DATA_ID、Hardened Runtime、entitlement はちょうど 3 つ
# （app-sandbox、network.client、fetch/ と inbox/ だけの一時例外）、束の中で network.* を持つのは ikari-data だけ、ikari.app の署名が封印している
verify_data() {
  [[ -x "$DATA_DEST" ]] || die "ikari-data が $DATA_DEST にありません"
  codesign --verify --strict "$DATA_DEST" || die "ikari-data の署名を検証できません"

  local info identifier
  info="$(codesign -dv "$DATA_DEST" 2>&1)"
  identifier="$(printf '%s\n' "$info" | sed -n 's/^Identifier=//p')"
  [[ "$identifier" == "$DATA_ID" ]] || die "ikari-data の identifier が $DATA_ID ではありません: '$identifier'"
  printf '%s\n' "$info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || die "ikari-data に Hardened Runtime（runtime）が付いていません"

  # 埋めた Info.plist（署名の identifier と同じ CFBundleIdentifier）
  printf '%s\n' "$info" | grep -Eq '^Info\.plist entries=[1-9][0-9]*' || die "ikari-data に Info.plist が埋まっていません（Sandbox の exe は起動の直後に落ちる）"
  local embedded
  # otool は 4 バイトごとに（リトルエンディアンで）16 進に出すので、バイトの並びを戻してから読む（arm64 のビルドだけ。RUST_TARGET）
  embedded="$(otool -X -s __TEXT __info_plist "$DATA_DEST" | sed 's/^[0-9a-f]*[[:space:]]*//' | xxd -r -p \
    | xxd -p -c4 | sed 's/\(..\)\(..\)\(..\)\(..\)/\4\3\2\1/' | xxd -r -p \
    | plutil -extract CFBundleIdentifier raw - 2>/dev/null || true)"
  [[ "$embedded" == "$DATA_ID" ]] || die "ikari-data の埋めた Info.plist の CFBundleIdentifier が $DATA_ID ではありません: '$embedded'"

  # entitlement: キーはちょうど 3 つ、一時例外は fetch/ と inbox/ だけ（検査は lib.sh。test-data-entitlements.sh で外れ方を確かめている）
  local problem
  problem="$(entitlements_of "$DATA_DEST" | data_entitlements_error)" || die "ikari-data の $problem"
  echo "data entitlements: app-sandbox=true, network.client=true, temporary-exception=fetch/ and inbox/ only (3 keys)"

  # 束の中で network.* を持つのは ikari-data だけ。一覧は手で並べず、Contents/ の下の Mach-O を全部見る
  local file file_ents scanned=0
  while IFS= read -r -d '' file; do
    file -b "$file" | grep -q 'Mach-O' || continue
    scanned=$((scanned + 1))
    [[ "$file" != "$DATA_DEST" ]] || continue
    file_ents="$(entitlements_of "$file")"
    [[ -z "$file_ents" ]] || ! printf '%s' "$file_ents" | plutil -convert xml1 -o - - | grep -q '<key>com\.apple\.security\.network\.' \
      || die "$file に com.apple.security.network.* があります（束の中で持てるのは ikari-data の network.client だけ）"
  done < <(find "$APP/Contents" -type f -print0)
  [[ "$scanned" -ge 2 ]] || die "束の中の署名されたコードを見つけられません（検査が空振りしている）"
  echo "network.*: only ikari-data (scanned $scanned Mach-O files)"

  grep -q "Helpers/$DATA_NAME" "$APP/Contents/_CodeSignature/CodeResources" || die "ikari.app の署名が Helpers/$DATA_NAME を封印していません"

  if [[ "$SIGNING" == team ]]; then
    local team
    team="$(printf '%s\n' "$info" | sed -n 's/^TeamIdentifier=//p')"
    [[ "$team" == "$TEAM_ID" ]] || die "ikari-data のチームID が $TEAM_ID ではありません: '$team'"
    echo "TeamIdentifier=$TEAM_ID (data)"
  fi
}

# オフライン版（ADR 0080 §7.3、§7.4）: 取得の部品が束のどこにも無く、束の中のどの実行ファイルにも network.* の entitlement が無い
verify_offline() {
  [[ ! -e "$DATA_DEST" ]] || die "オフライン版の束に $DATA_NAME があります"
  [[ ! -e "$APP/Contents/Helpers/$DATA_NAME" ]] || die "オフライン版の束に Helpers/$DATA_NAME があります"
  ! grep -q "Helpers/$DATA_NAME" "$APP/Contents/_CodeSignature/CodeResources" || die "オフライン版の ikari.app の署名が Helpers/$DATA_NAME を指しています"
  local file file_ents scanned=0
  while IFS= read -r -d '' file; do
    file -b "$file" | grep -q 'Mach-O' || continue
    scanned=$((scanned + 1))
    file_ents="$(entitlements_of "$file")"
    [[ -z "$file_ents" ]] || ! printf '%s' "$file_ents" | plutil -convert xml1 -o - - | grep -q '<key>com\.apple\.security\.network\.' \
      || die "オフライン版の $file に com.apple.security.network.* があります"
  done < <(find "$APP/Contents" -type f -print0)
  [[ "$scanned" -ge 2 ]] || die "束の中の署名されたコードを見つけられません（検査が空振りしている）"
  echo "offline edition: no $DATA_NAME, no network.* in any of $scanned Mach-O files"
}

# 同梱のデータの封印（ADR 0080 §3.4）。署名のあとに 1 バイトでも変わると落ちる。モデルの sha256 は表（resource/src/models.rs）と照らす
verify_bundled_data() {
  if [[ -z "$DATA_DIR" ]]; then
    [[ ! -e "$BUNDLED_DATA_DEST" ]] || die "--data-dir が無いのに束に data/ があります"
    return
  fi
  local problem
  problem="$(bundled_data_seal_error "$APP" "$BUNDLED_DATA_LIST")" || die "同梱のデータ: $problem"
  echo "bundled data: sealed by the engine signature, $(wc -l <"$BUNDLED_DATA_LIST" | tr -d ' ') files match the staged sha256, model matches the pinned sha256"
}

# 設定アプリ（ADR 0005 §6.2、§10.1「署名」）: Hardened Runtime、Sandbox なし、network の権限なし（NFR-17）、
# 殻の署名要件（SigningRequirements.settingsApp）を満たす、外側の署名に封印されている
verify_settings_app() {
  codesign --verify --deep --strict --verbose=2 "$SETTINGS"

  local info identifier
  info="$(codesign -dv "$SETTINGS" 2>&1)"
  identifier="$(printf '%s\n' "$info" | sed -n 's/^Identifier=//p')"
  [[ "$identifier" == "$SETTINGS_ID" ]] || die "設定アプリの identifier が $SETTINGS_ID ではありません: '$identifier'"
  printf '%s\n' "$info" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' \
    || die "設定アプリに Hardened Runtime（runtime）が付いていません"

  local ents
  ents="$(entitlements_of "$SETTINGS")"
  if printf '%s' "$ents" | grep -Eq 'com\.apple\.security\.(app-sandbox|network)'; then
    die "設定アプリに com.apple.security.app-sandbox または network.* があります（Sandbox なし、通信の権限なしが決定）"
  fi
  # キーは 0 個（templates/settings.entitlements は空の dict）
  if [[ -n "$ents" ]]; then
    local settings_keys
    settings_keys="$(printf '%s' "$ents" | plutil -convert xml1 -o - - | grep -c '<key>' || true)"
    [[ "$settings_keys" -eq 0 ]] || die "設定アプリに entitlement があります（キーが $settings_keys 個。付けない）"
  fi
  echo "settings entitlements: none, hardened-runtime=on"

  local requirement
  # ad-hoc の要件は cdhash を含む（#269）。同じバンドルから読んだ値で組み立てる
  requirement="$("$BIN_DIR/ikari-smoke" --print-requirement settings "$SIGNING" "$SETTINGS")"
  codesign --verify -R="$requirement" "$SETTINGS" || die "設定アプリが署名要件を満たしません: $requirement"
  local bridge_shell_req
  bridge_shell_req="$("$BIN_DIR/ikari-smoke" --print-requirement bridge-shell "$SIGNING" "$APP")"
  codesign --verify -R="$bridge_shell_req" "$APP" || die "殻（ブリッジの相手）が署名要件を満たしません: $bridge_shell_req"
  echo "signing requirement satisfied (settings, bridge shell)"

  # 外側の署名の封印に含まれる（差し替えられたら ikari.app の検証が通らなくなる）
  grep -q "Helpers/$SETTINGS_APP_NAME" "$APP/Contents/_CodeSignature/CodeResources" \
    || die "ikari.app の署名が Helpers/$SETTINGS_APP_NAME を封印していません"

  if [[ "$SIGNING" == team ]]; then
    local team
    team="$(printf '%s\n' "$info" | sed -n 's/^TeamIdentifier=//p')"
    [[ "$team" == "$TEAM_ID" ]] || die "設定アプリのチームID が $TEAM_ID ではありません: '$team'"
    echo "TeamIdentifier=$TEAM_ID (settings)"
  fi
}

# ---------------------------------------------------------------- 実行

if [[ "$FRAMEWORKS_ONLY" -eq 1 ]]; then
  build_rust
  log "完了（--frameworks-only）: $FRAMEWORKS"
  exit 0
fi
resolve_identity
if [[ "$SKIP_RUST" -eq 0 ]]; then
  build_rust
  [[ "$EDITION" == offline ]] || build_data
else
  # XCFramework の版が違うと、オフライン版に通信の核が入る（逆なら通常版に入らない）。作ったときの印と合わせる。
  # 印が無い（この印を足す前に作った）物は、通常版なら警告だけ、オフライン版なら止める
  if [[ -d "$FRAMEWORKS/IkariShellFFI.xcframework" ]]; then
    built_edition="$(cat "$FRAMEWORKS_EDITION_MARK" 2>/dev/null || true)"
    if [[ -n "$built_edition" && "$built_edition" != "$EDITION" ]]; then
      die "--skip-rust ですが、macos/Frameworks は edition=$built_edition でビルドされています（要るのは $EDITION）。--skip-rust を外してください"
    elif [[ -z "$built_edition" ]]; then
      [[ "$EDITION" == normal ]] || die "--skip-rust ですが、macos/Frameworks の edition が分かりません。オフライン版は --skip-rust を外してください"
      warn "macos/Frameworks に edition の印（.edition）がありません。通常版（online）でビルドした物と見なします"
    fi
  fi
  # XCFramework がないと Package.swift が FFI なしの形になり、動かない「スタブのエンジン」の ikari.app ができる
  for framework in IkariShellFFI IkariEngineFFI; do
    if [[ ! -d "$FRAMEWORKS/$framework.xcframework" ]]; then
      if [[ "$ALLOW_STUB" -eq 1 ]]; then
        warn "$framework.xcframework がありません（--allow-stub: FFI なしのスタブになる。インストールしないこと）"
      else
        die "--skip-rust ですが $framework.xcframework がありません。--skip-rust を外すか、スタブでよければ --allow-stub を付けてください"
      fi
    fi
  done
fi
mkdir -p "$OUT"
build_swift
assemble_bundle
sign_bundle
verify_bundle
verify_settings_app
if [[ "$EDITION" == normal ]]; then verify_data; else verify_offline; fi
verify_bundled_data
log "完了: $APP（edition: $EDITION）"
if [[ "$MAKE_DMG" -eq 1 ]]; then
  "$IKARI_SCRIPTS_DIR/make-dmg.sh" --app "$APP" --out "$OUT/ikari-$VERSION-macos-arm64-$SIGNING$EDITION_SUFFIX.dmg"
fi
