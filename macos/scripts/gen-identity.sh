#!/bin/bash
# lib.sh の IKARI_ID_PREFIX と TEAM_ID から macos/Sources/IkariXPC/Identity.swift を作る（ADR 0036 §3）。
# 使い方: gen-identity.sh           ... Identity.swift を書く（build.sh が呼ぶ）
#         gen-identity.sh --stdout  ... 標準出力へ出すだけ（scripts/check.sh が違いを調べる）
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

generate() {
  cat <<SWIFT
// 自動生成: macos/scripts/gen-identity.sh（値の出どころは macos/scripts/lib.sh の IKARI_ID_PREFIX と TEAM_ID。ADR 0036 §3）。
// 手で書き換えない。他の人が自分の値で使うときは、環境変数 IKARI_ID_PREFIX と TEAM_ID を与えて build.sh を流す。
import Foundation

public enum Identity {
    /// Bundle ID の頭（逆ドメインの形）
    public static let idPrefix = "$IKARI_ID_PREFIX"
    /// Apple Development 証明書のチームID（\`certificate leaf[subject.OU]\`）
    public static let teamID = "$TEAM_ID"

    /// 製品（殻）の ID。他の識別子はこの後ろに足す
    public static let productID = "\\(idPrefix).inputmethod.ikari"
}
SWIFT
}

if [[ "${1:-}" == "--stdout" ]]; then
  generate
else
  generate >"$IKARI_MACOS_DIR/Sources/IkariXPC/Identity.swift"
fi
