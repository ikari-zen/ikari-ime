#!/bin/bash
# proto/ikari/{engine,bridge}/v1/*.proto から Swift のコードを生成する（ADR 0004 §5.2、ADR 0005 §6.3）。
# スキーマを変えたときだけ流し、差分（macos/Sources/IkariProto/{engine,bridge}.pb.swift）をコミットする。
# bridge.proto は engine.proto を import する。Rust 側（proto/build.rs）は engine.proto だけをコンパイルする。
# 必要: protoc と protoc-gen-swift（brew install protobuf swift-protobuf）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/macos/Sources/IkariProto"
PROTOS=("ikari/engine/v1/engine.proto" "ikari/bridge/v1/bridge.proto")

for tool in protoc protoc-gen-swift; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "error: $tool が見つかりません（brew install protobuf swift-protobuf）" >&2
    exit 1
  }
done

mkdir -p "$OUT"
for proto in "${PROTOS[@]}"; do
  protoc \
    --proto_path="$ROOT/proto" \
    --swift_out="$OUT" \
    --swift_opt=Visibility=Public,FileNaming=DropPath \
    "$ROOT/proto/$proto"
done

echo "generated: $OUT/{engine,bridge}.pb.swift ($(protoc --version), $(protoc-gen-swift --version))"
