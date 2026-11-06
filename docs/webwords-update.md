# 外来語の一覧の更新手順

Wikipedia 日本語版の新しいダンプから外来語の一覧（[ADR 0055](adr/0055-web-words-classify-and-build.md)）を作り直し、Release に置き、固定を書き換える手順。約 3〜4 か月ごと、または Sudachi を更新したとき（ADR 0055 §6）に行う。

誰がいつ流しても同じになるように、機械的な部分は `cargo xtask webwords` にまとめてある。人が判断する点は「4. 人が読む点」にまとめた。

## 1. 前提

- macOS（評価と辞書のビルドを流す）。`xz`、`python3`、`gh`（`gh auth status` が通ること）、Rust が要る。
- `scripts/fetch-data.sh` で `data/raw/`（Sudachi など）が取得済みであること。`webwords` の一覧は要らない（`measure` が基準を作る）。
- 空きディスク: 取得したダンプが数十 GB、`measure` の scratch が辞書 1 点あたり約 1.2 GB（基準は残り、候補は評価のあと消える）。作業用のディレクトリは、実機の `data/` や `~/Library/Application Support/ikari-ime/` の外に置く（`measure` は重なる場所を拒否する）。
- 時間: 取得は 1 ファイルずつ間を空けるので長い。`build`（本文の集計）、`measure`（辞書のビルドと全セットの評価）も長く、`--lm` を足すとさらに長い（実測の値は、次の更新で分かった分をここに足す）。ほかの作業の裏で流してよい。
- LM ありを測るときは zenz の gguf のパスを用意する（`--lm`）。
- 値（df と基準コスト）は ADR 0055 §4.5 の承認済みのもの（dict-builder の既定）を使う。`measure` は値を引数で変えられない。値を替えたいときは、この手順ではなく ADR で掃引をやり直す。

以下、`DUMP=<YYYYMMDD>`（取るダンプの日付）、`WORK=<空のディレクトリ>`、`OUT=<出力先>` とする。

## 2. 取得（fetch）

```sh
cargo xtask webwords fetch --dump "$DUMP" --work "$WORK"
```

dumps.wikimedia.org から 1 ファイルずつ取り、sha1 を照合し、sha256 を `$WORK/manifest.tsv` に記録する。途中で止まったら、`$WORK` を空にして最初からやり直す。

## 3. 測る（measure）。Release の手前まで

```sh
cargo xtask webwords measure --dump "$DUMP" --work "$WORK" --out "$OUT" --lm <zenz の gguf>
```

- `$OUT` に一覧がなければ `build` を流す（collect → classify → xz）。あれば再利用する（**同じ `$OUT` を渡したときだけ**。別の出力先では作り直す）。
- `$WORK/manifest.tsv`（`fetch` が書く）が**必ず要る**。一覧が既にあっても、`fetch` を流さずに `measure` だけ流す経路はない。manifest を手で作らない（sha256 の記録になるため）。
- 評価の辞書を作るために、リポジトリの `data/raw/` が要る。新しい worktree には `data/` がないので、`--data <本体のリポジトリの data>`（読むだけ）を渡すか、`scripts/fetch-data.sh` で worktree に取る。実機の辞書とユーザーデータは読まない・書かない。
- 評価用の scratch（`$OUT/scratch`）に、基準（外来語なし）と、承認済みの既定の値の辞書を作り、全セットと AJIMEE を流して比べる。`--lm` を渡すと LM あり（時間切れの上限 2000ms）も測る。
- 出力: `$OUT/measure-$DUMP.md`（比較の表、辞書の増加量、追加した行数）、`$OUT/sample.tsv`（抜き取り 30 語。common で df が 5〜6。seed は既定で固定なので、同じ一覧からは同じ語が出る）。
- **悪化が 1 件でもあれば終了コードが非 0 になり、最後に大きく表示する。** その場合は Release に進まない。

実機の辞書（`data/build`）とユーザーデータには触れない。

## 4. 人が読む点（止まる場所）

次を読み、異常があれば止まって原因を調べる。

1. **一覧のレポート**（`$OUT/webwords-$DUMP-report.md`）の理由ごとの件数。前回の Release の資産のレポート（`gh release download webwords-<前回の日付> --pattern '*-report.md'`）と比べ、急に増減した理由がないか。
2. **抜き取り**（`$OUT/sample.tsv`）。30 語を見て、辞書に足してはいけない語（人名・地名・商品名・誤った切れ方など）が目立たないか。「普通名詞として足してよいか」を人が判断する。目立つなら止まって、ユーザーに報告する。
3. **悪化 0**（`measure-$DUMP.md` の結果の行）。LM なし・LM あり（渡したとき）とも 0 であること。
4. **辞書の増加量と追加した行数**を、ユーザーに伝える（前回との差も添える）。

この 4 点を確かめたら、**Release の前に、ユーザーに聞く**。ユーザーの了解がなければ `publish` に `--yes` を付けない。

## 5. Release と固定（publish）

まず dry-run（`--yes` なし）で、やる予定を確かめる。何も変えない。

```sh
cargo xtask webwords publish --dump "$DUMP" --out "$OUT"
```

表示されるのは、タグ（`webwords-<日付>`）、資産（xz とレポート）、xz の sha256 と行数、書き換える行、Release のノート（CC BY-SA 4.0 の表示つき）。ユーザーの了解を得たら:

```sh
cargo xtask webwords publish --dump "$DUMP" --out "$OUT" --yes
```

- `gh release create` で Release を作る。
- `scripts/lib.sh` の `WEBWORDS_LIST_URL` と `WEBWORDS_LIST_SHA256`、`THIRD_PARTY_NOTICES.md` のダンプの日付を書き換える。
- 書き換えは、Release を作る前に内容を作り、失敗するなら先に止まる。Release の作成が失敗したら、ファイルは変わらない。

## 6. 取れることの確認と PR

```sh
scripts/fetch-data.sh --only webwords
```

`data/raw/webwords/` に置かれ、sha256 の照合が通ることを確かめる。そのあと、書き換えた 2 ファイルを `docs/` の更新とともにブランチ（`feat/webwords-pin-<日付>` など）でコミットし、PR にする。

## 7. 後始末

作業用のディレクトリ（`$WORK`）と出力先（`$OUT`、特に `scratch/` の辞書）を消す。本文と SQL ダンプは配らない。sha256 は一覧のヘッダとレポートに残る。

## 関連

- 設計: [ADR 0055](adr/0055-web-words-classify-and-build.md)（§5.1 がこの手順の元）
- 実装: `xtask/src/webwords.rs`、`xtask/src/webwords/`（`measure`、`publish`）、`eval/scripts/webwords_sweep.sh`
- 薄いスキル: `.claude/skills/webwords-update/SKILL.md`（流す順と止まる場所だけ。詳細はここ）
