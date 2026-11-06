---
name: webwords-update
description: 外来語の一覧（ADR 0055）を新しい Wikipedia のダンプで作り直すとき（約 3〜4 か月ごと、Sudachi の更新のあと）に、流す順と止まる場所を示す。詳細は docs/webwords-update.md。
---

# 外来語の一覧の更新

手順の本体は [docs/webwords-update.md](../../../docs/webwords-update.md)。ここは流す順、止まる場所、ユーザーに聞くことだけ。値（df、基準コスト）は決めない。承認済みの既定（ADR 0055 §4.5）を使う。

## 流す順

1. `cargo xtask webwords fetch --dump <日付> --work <空のディレクトリ>`
2. `cargo xtask webwords measure --dump <日付> --work <同じ> --out <出力先> [--lm <gguf>]`（`build` を含む。Release の手前まで）
3. （人が読む。下の「止まる場所」）
4. `cargo xtask webwords publish --dump <日付> --out <出力先>`（dry-run。何も変えない）
5. **ユーザーに聞く**。了解が出たら `--yes` を付けて同じコマンド
6. `scripts/fetch-data.sh --only webwords` で取れることを確かめ、`scripts/lib.sh` と `THIRD_PARTY_NOTICES.md` の変更を PR にする
7. 作業用ディレクトリと出力先を消す

## 止まる場所

- `measure` が非 0 で終わった（悪化がある）: 進まない。`measure-<日付>.md` を読んで、リードがユーザーに報告する。
- 一覧のレポートの理由別件数が前回と大きく違う、抜き取り（`sample.tsv`）に不適切な語が目立つ: 進まない。リードがユーザーに報告する。
- **`publish --yes` の前: 必ずユーザーに聞く。** 伝えるのは、悪化 0 であること、辞書の増加量と追加した行数、抜き取りで見たこと、作るタグ。

## 役割

- コマンドを流す: Haiku でよい（fetch、measure、publish の dry-run、後始末）。
- レポートの異常の判断、抜き取りの確認: リードか Sonnet。Haiku に任せない。
- `--yes`: ユーザーの了解のあと、リードが流す。

## 触れないもの

実機の辞書（`data/build`）、`~/Library/Application Support/ikari-ime/`、`data/raw/`。smoke は流さない。
