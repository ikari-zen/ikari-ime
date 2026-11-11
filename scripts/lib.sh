#!/bin/bash
# scripts/*.sh の共通部分（source して使う）。
# macos/scripts/lib.sh とは共有しない（macos/ を単独で読めるようにするため。docs/refactor-plan-2026-10.md §5.3）。

IKARI_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IKARI_ROOT="$(cd "$IKARI_SCRIPTS_DIR/.." && pwd)"

# ---- データの定数（取得、辞書のビルド、配置の各スクリプトが同じ値を使う） ----
# モデルのファイル名は engine/src/lib.rs の MODEL_FILE_NAME と一致させる（scripts/check.sh が grep で確かめる）
readonly SUDACHI_VERSION="20260723"
readonly SUDACHI_DIR_NAME="sudachi-$SUDACHI_VERSION"
readonly CHIVE_NAME="chive-1.3-mc90"
readonly ZENZ_MODEL_FILE="zenz-v3.2-small-Q5_K_M.gguf"
readonly DICT_FILES=(grammar.bin system.dict vectors.bin)
# 絵文字の表（ADR 0027 §5）。辞書の本体とは別に作り、なくても辞書は使える
readonly EMOJI_TABLE_FILE="emoji.tsv"
readonly EMOJI_DATA_COMMIT="13ee711e222ea17fe537bfea953c687866f16411"
readonly EMOJI_DATA_JSON="emoji-data/emoji.json" # data/raw/ からの相対パス
# ABR（デジタル庁）の町字マスター（ADR 0030）。辞書に大字・町名を足す任意の入力。data/raw/ からの相対パス
readonly ABR_DIR_NAME="abr"
readonly ABR_TOWN_CSV="abr/mt_town_all.csv"
# ABR の町字・郵便番号変換表（ADR 0032 §7）。住所 <-> 郵便番号の表 postal.tsv の任意の入力。data/raw/ からの相対パス
readonly ABR_POSTAL_ITEM_ID="1098a4f61dab4f52997839d04245132a"
readonly ABR_POSTAL_CSV="abr/abr_post_code.csv"
# 住所と郵便番号の表（辞書の本体とは別に作り、なくても辞書は使える。emoji.tsv と同じ置き場所）
readonly POSTAL_TABLE_FILE="postal.tsv"
# ABR から作る地名の lex.csv（辞書のビルドの中間ファイル。data/build/ の下）
readonly ABR_TOWN_LEX_FILE="town_lex.csv"
readonly ABR_CHAIN_LEX_FILE="chain_lex.csv"
# Wikipedia 日本語版から集めた外来語の一覧（ADR 0055。CC BY-SA 4.0）。辞書に語を足す任意の入力。data/raw/ からの相対パス。
# URL（GitHub の Release の資産。xz）と sha256（xz の資産そのものの sha256）が空の間は、取得しない（一覧を Release に置いたら埋める）。
# 辞書のビルドは、data/raw/ の下に WEBWORDS_LIST_PATH のファイル（xz を展開したもの）があれば組み込む（なければ今と同じ辞書を作る）
readonly WEBWORDS_LIST_PATH="webwords/webwords.tsv"
readonly WEBWORDS_LIST_URL="https://github.com/ikari-zen/ikari-ime/releases/download/webwords-20261101/webwords-20261101.tsv.xz"
readonly WEBWORDS_LIST_SHA256="0e0eea7a5386fda1190f1ca9056918b60537d0fe14731a4e11b03230782d1ba6"
# 外来語の一覧から作る lex.csv（辞書のビルドの中間ファイル。data/build/ の下）
readonly WEBWORDS_LEX_FILE="webwords_lex.csv"
readonly DICT_POLICY="dict-builder/policy/chive.tsv"
readonly APP_SUPPORT_DIR="$HOME/Library/Application Support/ikari-ime"

log() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
