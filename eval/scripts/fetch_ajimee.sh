#!/usr/bin/env bash
# AJIMEE-Bench（azooKey/AJIMEE-Bench）の評価データを取得する（ADR 0002 §15.2, §21.1 #4）。
#
# データ（JWTD_v2/v1/evaluation_items.json）は CC BY-SA 3.0。リポジトリには入れない。
# 出力先は data/raw/ajimee/（gitignore 済み）。環境変数 IKARI_AJIMEE_DIR で変えられる。
#
# 使い方:
#   eval/scripts/fetch_ajimee.sh
#   cargo run --release -p ikari-eval -- --set data/raw/ajimee/evaluation_items.json --set-format ajimee
set -euo pipefail

# 再現性のため、取得するコミットを固定する（2025-01-13 時点の main）
AJIMEE_COMMIT="401666cd56d1a570c2021798b64b6da4396bfd45"
AJIMEE_PATH="JWTD_v2/v1/evaluation_items.json"
AJIMEE_URL="https://raw.githubusercontent.com/azooKey/AJIMEE-Bench/${AJIMEE_COMMIT}/${AJIMEE_PATH}"
# 評価コード（CC0）のライセンス表示の確認用に README も保存する
AJIMEE_README_URL="https://raw.githubusercontent.com/azooKey/AJIMEE-Bench/${AJIMEE_COMMIT}/README.md"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
out_dir="${IKARI_AJIMEE_DIR:-${repo_root}/data/raw/ajimee}"
out_file="${out_dir}/evaluation_items.json"

mkdir -p "${out_dir}"

fetch() {
  local url="$1" dest="$2"
  # 途中で切れたファイルを残さないよう、一時ファイルに落としてから移す
  local tmp
  tmp="$(mktemp "${dest}.XXXXXX")"
  if ! curl --fail --silent --show-error --location --retry 3 --output "${tmp}" "${url}"; then
    rm -f "${tmp}"
    echo "error: failed to download ${url}" >&2
    exit 1
  fi
  mv "${tmp}" "${dest}"
}

fetch "${AJIMEE_URL}" "${out_file}"
fetch "${AJIMEE_README_URL}" "${out_dir}/README.md"

# 形だけ確かめる（JSON の配列で、200 件前後あること）
if command -v python3 >/dev/null 2>&1; then
  python3 - "${out_file}" <<'PY'
import json, sys
items = json.load(open(sys.argv[1], encoding="utf-8"))
assert isinstance(items, list) and items, "evaluation_items.json is not a non-empty array"
for key in ("input", "expected_output"):
    assert all(key in it for it in items), f"missing key: {key}"
print(f"ok: {len(items)} items")
PY
fi

cat >"${out_dir}/LICENSE.txt" <<EOF
evaluation_items.json: AJIMEE-Bench (https://github.com/azooKey/AJIMEE-Bench), commit ${AJIMEE_COMMIT}.
The data is based on the Japanese Wikipedia Typo Dataset v2 and is licensed under CC BY-SA 3.0.
Do not commit this directory to the ikari-ime repository.
EOF

echo "saved: ${out_file}"
