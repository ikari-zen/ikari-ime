#!/bin/bash
# 同梱するデータ（辞書、モデル、NOTICE）を束の中に置く・封印を検査する関数（source して使う。ADR 0080 §3.1、§3.4、§11 のレーン 4）。
# build.sh と test-package.sh が使う。lib.sh を先に source しておくこと。
#
# 置き場所: <ikari.app>/Contents/XPCServices/IkariEngine.xpc/Contents/Resources/data/
#   dict/{grammar.bin,system.dict,vectors.bin}（必須）、dict/{emoji.tsv,postal.tsv,BUILD_INFO}（あれば）、
#   models/<モデル>.gguf（必須）、NOTICE.txt、licenses/（docs/dist-notices/ にあれば）
# 元のデータの並びは `cargo xtask data install` の --src と同じ（<src>/build/dict/、<src>/raw/zenz/）。
#
# 判断はここに足さない。置く物の名前と sha256 の「正」は Rust の表（resource/src/models.rs）、
# 辞書の 3 ファイルの中身の検査はエンジン（起動時の magic、形式の版、dict_id の照合）が持つ。

# 束の中の、データの置き場所（ikari.app からの相対）。engine/src/data_location.rs の bundled_data_dir_for_exe と同じ
readonly BUNDLED_DATA_REL="Contents/XPCServices/IkariEngine.xpc/Contents/Resources/data"
# XPC の束（ikari.app からの相対）。データはこの束の署名が封印する
readonly BUNDLED_XPC_REL="Contents/XPCServices/IkariEngine.xpc"
readonly BUNDLED_DICT_REQUIRED=(grammar.bin system.dict vectors.bin)
readonly BUNDLED_DICT_OPTIONAL=(emoji.tsv postal.tsv BUILD_INFO)
readonly BUNDLED_MODELS_RS="$IKARI_ROOT/resource/src/models.rs"
readonly BUNDLED_NOTICE_DIR="$IKARI_ROOT/docs/dist-notices"

sha256_file() { shasum -a 256 "$1" | cut -d' ' -f1; }

# resource/src/models.rs の表から、プリセットのモデルのファイル名と sha256 を読む（唯一の出どころ。別の表を持たない）
pinned_model_file() {
  sed -n 's/^pub const ZENZ_SMALL_FILE_NAME: &str = "\(.*\)";$/\1/p' "$BUNDLED_MODELS_RS" | head -1
}
pinned_model_sha256() {
  sed -n '/^pub const ZENZ_SMALL_SHA256: &str =/{n;s/^[[:space:]]*"\([0-9a-f]\{64\}\)";$/\1/p;}' "$BUNDLED_MODELS_RS" | head -1
}

# データの元（--data-dir）に、必須のファイルが揃っているか。揃っていなければ die
check_data_source() { # <data のディレクトリ>
  local src="$1" model file
  [[ -d "$src" ]] || die "--data-dir がディレクトリではありません: $src"
  model="$(pinned_model_file)"
  [[ -n "$model" ]] || die "$BUNDLED_MODELS_RS からモデルのファイル名を読めません"
  for file in "${BUNDLED_DICT_REQUIRED[@]}"; do
    [[ -s "$src/build/dict/$file" ]] || die "辞書がありません: ${src}/build/dict/${file}（scripts/build-dict.sh）"
  done
  [[ -s "$src/raw/zenz/$model" ]] || die "モデルがありません: ${src}/raw/zenz/${model}（cargo xtask data fetch）"
}

# データの元を、束の中に置き、置いた物の一覧（sha256 と相対パス。元のファイルから数える）を <一覧> に書く。
# <dest> は束の中のデータの置き場所（`<ikari.app>/$BUNDLED_DATA_REL`）
stage_bundled_data() { # <data のディレクトリ> <dest> <一覧>
  local src="$1" dest="$2" list="$3" model file rel
  check_data_source "$src"
  model="$(pinned_model_file)"
  rm -rf "$dest"
  mkdir -p "$dest/dict" "$dest/models"
  for file in "${BUNDLED_DICT_REQUIRED[@]}" "${BUNDLED_DICT_OPTIONAL[@]}"; do
    [[ -f "$src/build/dict/$file" ]] || continue
    cp "$src/build/dict/$file" "$dest/dict/$file"
  done
  cp "$src/raw/zenz/$model" "$dest/models/$model"
  if [[ -f "$BUNDLED_NOTICE_DIR/NOTICE.txt" ]]; then cp "$BUNDLED_NOTICE_DIR/NOTICE.txt" "$dest/NOTICE.txt"; fi
  if [[ -d "$BUNDLED_NOTICE_DIR/licenses" ]]; then cp -R "$BUNDLED_NOTICE_DIR/licenses" "$dest/licenses"; fi

  # 一覧は、元のファイルから数える（コピーの壊れも見つかる）。置いた物は、元のファイルと同じ相対パスで並べる
  : >"$list"
  for file in "${BUNDLED_DICT_REQUIRED[@]}" "${BUNDLED_DICT_OPTIONAL[@]}"; do
    [[ -f "$src/build/dict/$file" ]] || continue
    printf '%s  dict/%s\n' "$(sha256_file "$src/build/dict/$file")" "$file" >>"$list"
  done
  printf '%s  models/%s\n' "$(sha256_file "$src/raw/zenz/$model")" "$model" >>"$list"
  while IFS= read -r -d '' file; do
    rel="${file#"$dest"/}"
    case "$rel" in dict/* | models/*) continue ;; esac
    printf '%s  %s\n' "$(sha256_file "$file")" "$rel" >>"$list"
  done < <(find "$dest" -type f -print0 | sort -z)
}

# 署名した束の、データの封印を検査する。落ちたら理由を標準エラーに書いて 1 を返す（die しない。テストのため）
#   1. 束の署名の検証（codesign --verify --deep --strict）。署名のあとに 1 バイトでも変わると落ちる
#   2. データの全ファイルが、XPC の束の CodeResources に載っている（封印から漏れていない）
#   3. 束の中のデータが、一覧（置いたときの sha256）と過不足なく一致する
#   4. モデルが、アプリに埋めた表の sha256（resource/src/models.rs）と一致する
bundled_data_seal_error() { # <ikari.app> <一覧> [モデルの sha256（既定は表の値）]
  local app="$1" list="$2" want_model_sha="${3:-$(pinned_model_sha256)}"
  local data="$app/$BUNDLED_DATA_REL" xpc="$app/$BUNDLED_XPC_REL" resources sum rel got model_line
  [[ -d "$data" ]] || { echo "同梱のデータの場所がありません: $data"; return 1; }
  [[ -f "$list" ]] || { echo "置いた物の一覧がありません: $list"; return 1; }
  [[ -n "$want_model_sha" ]] || { echo "表からモデルの sha256 を読めません"; return 1; }

  codesign --verify --deep --strict "$app" >/dev/null 2>&1 || { echo "束の署名を検証できません（署名のあとに中身が変わった）"; return 1; }
  codesign --verify --deep --strict "$xpc" >/dev/null 2>&1 || { echo "エンジンの署名を検証できません（署名のあとに中身が変わった）"; return 1; }

  resources="$xpc/Contents/_CodeSignature/CodeResources"
  [[ -f "$resources" ]] || { echo "$resources がありません"; return 1; }
  while read -r sum rel; do
    grep -qF "<key>Resources/data/$rel</key>" "$resources" || { echo "エンジンの署名が data/$rel を封印していません"; return 1; }
  done <"$list"

  # 過不足なし（一覧にある物がすべてあり、一覧にない物がない）
  local listed present
  listed="$(awk '{ $1 = ""; sub(/^ +/, ""); print }' "$list" | sort)"
  present="$(cd "$data" && find . -type f | sed 's|^\./||' | sort)"
  [[ "$listed" == "$present" ]] || { echo "束の中のデータのファイルの一覧が、置いたときと違います"; return 1; }
  while read -r sum rel; do
    got="$(sha256_file "$data/$rel")"
    [[ "$got" == "$sum" ]] || { echo "data/$rel の sha256 が、置いたときと違います"; return 1; }
  done <"$list"

  model_line="$(grep -E '  models/[^/]+$' "$list" || true)"
  [[ "$(printf '%s\n' "$model_line" | grep -c .)" -eq 1 ]] || { echo "モデルがちょうど 1 つではありません"; return 1; }
  [[ "${model_line%% *}" == "$want_model_sha" ]] || { echo "モデルの sha256 が、アプリに埋めた表の値と違います"; return 1; }
  return 0
}
