#!/bin/bash
# ikari.app をビルドして署名する（ADR 0004 §9.2）。
#
#   macos/scripts/build.sh [--signing team|adhoc] [--release|--debug] [--no-lm] [--skip-rust] [--allow-stub]
#
#   --signing team   Apple Development 証明書（チームID（`lib.sh` の `TEAM_ID`））で署名する。既定
#   --signing adhoc  ad-hoc 署名（署名要件は identifier の一致だけになる）
#   --release/--debug  Swift のビルド種別（Rust は常に release）。既定は release
#   --no-lm          エンジンに llama.cpp（Metal）を入れない（cmake が使えないとき、LM が要らないとき）
#   --skip-rust      cargo と XCFramework の作成を飛ばし、macos/Frameworks/ にあるものを使う
#   --allow-stub     XCFramework がなくてもビルドする（FFI なしのスタブのエンジンになる。インストールしてはいけない）
#
# 出力: target/macos/<release|debug>-<team|adhoc>/ikari.app
# 環境変数: IKARI_SIGN_IDENTITY で署名の ID（ハッシュまたは名前）を指定できる（team のとき。チームIDは検証する）
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

SIGNING=team
CONFIG=release
ENGINE_FEATURES=lm-metal
SKIP_RUST=0
ALLOW_STUB=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signing) SIGNING="${2:-}"; shift ;;
    --signing=*) SIGNING="${1#--signing=}" ;;
    --no-lm) ENGINE_FEATURES="" ;;
    --skip-rust) SKIP_RUST=1 ;;
    --allow-stub) ALLOW_STUB=1 ;;
    -h | --help) sed -n '2,16p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) parse_common_flag "$1" || die "不明な引数: $1" ;;
  esac
  shift
done
validate_signing

OUT="$(bundle_out_dir)"
APP="$OUT/$APP_NAME"
XPC="$APP/Contents/XPCServices/IkariEngine.xpc"
SETTINGS="$APP/Contents/Helpers/$SETTINGS_APP_NAME"
TEMPLATES="$IKARI_SCRIPTS_DIR/templates"
FRAMEWORKS="$IKARI_MACOS_DIR/Frameworks"
RUST_TARGET=aarch64-apple-darwin
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

build_rust() {
  log "cargo build: ikari-shell-ffi"
  (cd "$IKARI_ROOT" && cargo build --release --target "$RUST_TARGET" -p ikari-shell-ffi)

  log "cargo rustc: ikari-engine-ffi ${ENGINE_FEATURES:+(features: $ENGINE_FEATURES)}"
  local features_args=() output
  [[ -z "$ENGINE_FEATURES" ]] || features_args=(--features "$ENGINE_FEATURES")
  output="$(cd "$IKARI_ROOT" && CARGO_TARGET_DIR="$IKARI_ROOT/target/lm" cargo rustc --release \
    --target "$RUST_TARGET" -p ikari-engine-ffi ${features_args[@]+"${features_args[@]}"} \
    --lib --crate-type staticlib -- --print native-static-libs 2>&1)" || { printf '%s\n' "$output" >&2; die "cargo rustc が失敗しました"; }
  printf '%s\n' "$output" | tail -3
  local reported
  reported="$(printf '%s\n' "$output" | sed -n 's/.*native-static-libs: //p' | tail -1)"
  [[ -n "$reported" ]] || die "native-static-libs が出力されませんでした"
  check_native_libs "$reported"

  log "XCFramework"
  mkdir -p "$FRAMEWORKS" "$OUT"
  make_xcframework "$IKARI_ROOT/target/$RUST_TARGET/release/libikari_shell_ffi.a" \
    "$IKARI_ROOT/shell-ffi/include" ikari_shell.h IkariShellFFI "$FRAMEWORKS/IkariShellFFI.xcframework"
  make_xcframework "$IKARI_ROOT/target/lm/$RUST_TARGET/release/libikari_engine_ffi.a" \
    "$IKARI_ROOT/engine-ffi/include" ikari_engine.h IkariEngineFFI "$FRAMEWORKS/IkariEngineFFI.xcframework"
}

# ---------------------------------------------------------------- Swift とバンドル

build_swift() {
  "$IKARI_SCRIPTS_DIR/gen-identity.sh"
  log "swift build ($CONFIG)"
  mkdir -p "$OUT"
  local build_log="$OUT/swift-build.log"
  # 配るプロダクトだけをビルドする。全体のビルドには ikari-shell-bench（@testable。-enable-testing が要る。cargo xtask perf が自前でビルドする）が入り、release で失敗する
  local product_args=() product
  for product in ikari IkariEngine ikari-smoke IkariSettings; do product_args+=(--product "$product"); done
  swift build -c "$CONFIG" --manifest-cache none --package-path "$IKARI_MACOS_DIR" "${product_args[@]}" >"$build_log" 2>&1 \
    || { tail -40 "$build_log" >&2; die "swift build が失敗しました（${build_log}）"; }
  tail -1 "$build_log"
  BIN_DIR="$(swift build -c "$CONFIG" --manifest-cache none --package-path "$IKARI_MACOS_DIR" --show-bin-path)"
  for product in ikari IkariEngine ikari-smoke IkariSettings; do
    [[ -x "$BIN_DIR/$product" ]] || die "$BIN_DIR/$product がありません"
  done
}

# 設定アプリ「ikari 設定」を Contents/Helpers/ に置く（ADR 0005 §2、§6.2）。
# Resources: アプリのアイコン（macos/Resources/settings/AppIcon.icns）、ライセンス表示（NFR-43）、
# キー割り当てのプリセット（上書きが「プリセットと同じ」かを画面が判定するため。殻の Resources/keymap と同じファイル）
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
  cp "$IKARI_ROOT/THIRD_PARTY_NOTICES.md" "$resources/THIRD_PARTY_NOTICES.md"
  [[ -d "$IKARI_MACOS_DIR/Resources/keymap" ]] || die "macos/Resources/keymap がありません"
  cp -R "$IKARI_MACOS_DIR/Resources/keymap" "$resources/keymap"
}

assemble_bundle() {
  log "バンドルの組み立て: $APP"
  rm -rf "$APP"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$XPC/Contents/MacOS" \
    "$SETTINGS/Contents/MacOS" "$SETTINGS/Contents/Resources"

  cp "$BIN_DIR/ikari" "$APP/Contents/MacOS/ikari"
  cp "$BIN_DIR/ikari-smoke" "$APP/Contents/MacOS/ikari-smoke"
  cp "$BIN_DIR/IkariEngine" "$XPC/Contents/MacOS/IkariEngine"
  cp "$BIN_DIR/IkariSettings" "$SETTINGS/Contents/MacOS/IkariSettings"

  cp "$TEMPLATES/ikari.Info.plist" "$APP/Contents/Info.plist"
  render_identity "$APP/Contents/Info.plist"
  plist_set_string "$APP/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$APP/Contents/Info.plist" CFBundleVersion "$VERSION"
  plist_set_string "$APP/Contents/Info.plist" IkariSigningMode "$SIGNING"

  # XPC サービスが相手に求める署名要件は、IkariXPC の SigningRequirements が唯一の出どころ
  local peer
  peer="$("$APP/Contents/MacOS/ikari-smoke" --print-requirement shell "$SIGNING")"
  [[ -n "$peer" ]] || die "署名要件を取得できませんでした"
  cp "$TEMPLATES/engine.Info.plist" "$XPC/Contents/Info.plist"
  render_identity "$XPC/Contents/Info.plist"
  plist_set_string "$XPC/Contents/Info.plist" CFBundleShortVersionString "$VERSION"
  plist_set_string "$XPC/Contents/Info.plist" CFBundleVersion "$VERSION"
  plist_set_string "$XPC/Contents/Info.plist" IkariSigningMode "$SIGNING"
  plist_set_string "$XPC/Contents/Info.plist" IkariPeerRequirement "$peer"

  assemble_settings_app

  # Resources: keymap のプリセットなど。icons/ は直下に平らに置く（Info.plist のアイコン名は直下を指す）
  local res="$IKARI_MACOS_DIR/Resources"
  if [[ -d "$res" ]]; then
    [[ ! -d "$res/keymap" ]] || cp -R "$res/keymap" "$APP/Contents/Resources/keymap"
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
  codesign "${common[@]}" -i "$SMOKE_ID" "$APP/Contents/MacOS/ikari-smoke"
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
  echo "engine entitlements: app-sandbox=true, network=none, temporary-exception=~/Library/Application Support/ikari-ime/"

  # 殻は Sandbox なし
  if entitlements_of "$APP" | grep -q 'com\.apple\.security\.app-sandbox'; then
    die "殻（ikari.app）に app-sandbox が付いています"
  fi

  # 署名要件の文字列が、実際の署名に当てはまること
  local engine_req shell_req
  engine_req="$("$APP/Contents/MacOS/ikari-smoke" --print-requirement engine "$SIGNING")"
  shell_req="$("$APP/Contents/MacOS/ikari-smoke" --print-requirement shell "$SIGNING")"
  codesign --verify -R="$engine_req" "$XPC" || die "エンジンが署名要件を満たしません: $engine_req"
  codesign --verify -R="$shell_req" "$APP" || die "殻が署名要件を満たしません: $shell_req"
  codesign --verify -R="$shell_req" "$APP/Contents/MacOS/ikari-smoke" || die "smoke が署名要件を満たしません: $shell_req"
  echo "signing requirements satisfied (engine, shell, smoke)"

  if [[ "$SIGNING" == team ]]; then
    local team
    for target in "$APP" "$XPC" "$APP/Contents/MacOS/ikari-smoke"; do
      team="$(codesign -dv "$target" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$team" == "$TEAM_ID" ]] || die "$target のチームID が $TEAM_ID ではありません: '$team'"
    done
    echo "TeamIdentifier=$TEAM_ID (app, engine, smoke)"
  fi
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
  echo "settings entitlements: app-sandbox=none, network=none, hardened-runtime=on"

  local requirement
  requirement="$("$APP/Contents/MacOS/ikari-smoke" --print-requirement settings "$SIGNING")"
  codesign --verify -R="$requirement" "$SETTINGS" || die "設定アプリが署名要件を満たしません: $requirement"
  echo "signing requirement satisfied (settings)"

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

resolve_identity
if [[ "$SKIP_RUST" -eq 0 ]]; then
  build_rust
else
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
log "完了: $APP"
