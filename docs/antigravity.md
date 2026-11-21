# Antigravity（Gemini）のレーン向けの指示

ikari-ime は、Claude Code が**リード**、Antigravity が**レーン**として分担して進める（2026-11-22 ユーザー決定）。この文書は Antigravity のための指示。リポジトリの共通の規則は [CLAUDE.md](../CLAUDE.md)（`AGENTS.md` は同じファイル）にある。**CLAUDE.md の「リードは…」「サブエージェントを起動する」の部分は、リード（Claude Code）への指示で、あなた（Antigravity）には当てはまらない。** このレーン向けの読み替えは、この文書が優先する。

## 1. 役割

| | 持つもの |
| --- | --- |
| **リード（Claude Code）** | 優先順位、ADR と設計、ユーザーへの確認、統合、`scripts/check.sh`、PR のマージ、Mac の重い作業（XCFramework のビルド、評価の測定、smoke、install）、Windows の VM の操作 |
| **Antigravity** | 割り当てられた issue の調査・文書・テスト・実装。1 issue につき 1 ブランチ・1 PR |

- 方針の決定はユーザー（作者）が行う。数値や方針を勝手に決めない。迷ったら §5 の形で issue に書いて、リードに聞く。
- 割り当ては `antigravity` ラベルのついた issue。ラベルのない issue には着手しない。

## 2. 作業場所（リードの作業コピーに触らない）

- リードは `~/github.com/ikari-zen/ikari-ime` で jj を使っている。**そこでは作業しない**（同じ作業コピーでの並行は、変更が二重になる）。
- 作業は、専用の作業場所（worktree）で行う。issue ごとに作り、終わったら消す。**あなたは git だけを使う**（jj は使わなくてよい）。次のどちらで作ってもよい:
  ```sh
  # (a) git の worktree
  git -C ~/github.com/ikari-zen/ikari-ime fetch origin
  git -C ~/github.com/ikari-zen/ikari-ime worktree add ~/github.com/ikari-zen/ikari-ime-ag/<ブランチ名> -b <ブランチ名> origin/main
  # (b) jj の workspace（リードが作る場合）。中身は同じく git の worktree になり、git がそのまま使える
  jj workspace add ~/github.com/ikari-zen/ikari-ime-ag/<ブランチ名>
  ```
  終わったら、(a) は `git worktree remove`、(b) は `jj workspace forget <名前>` と、フォルダの削除。
- 確かめたこと（2026-11-22。jj 0.47.1、共存のリポジトリを使い捨てで作って試した）: (b) で作った workspace にも `.git`（`.git/worktrees/…` を指すファイル）が付き、`git switch`、`git commit` が通る。git で作ったコミットとブランチは、リード側の `jj log` にも出る。フック（`.githooks/`）は未確認。
- 注意: リードの jj の操作（`jj abandon`、`jj rebase` など）は、同じリポジトリの全コミットに及ぶ。あなたのブランチは、push するまではローカルにしかないので、こまめに push する（§3）。

## 3. 進め方

1. issue の担当を宣言する: issue に「着手します（ブランチ名）」とコメントし、自分を assignee にする。
2. 上の worktree でブランチを切り、**すぐ** `git push -u origin <ブランチ>`。意味のある小さな単位ごとにコミットして push する。ブランチ名は `feat/…`、`fix/…`、`docs/…`。
3. PR を作る（draft でよい）。説明に `Closes #番号`、「判断の置き場所」（OS に依存しないものは Rust。Swift や C# に判断を足すときは理由を書く）、分かったことと推測を分けて書く。
4. **マージしない。** マージはリードが行う（`scripts/check.sh` の全体が通り、CI が緑になってから）。main へ直接 push しない。`--no-verify` や、フックの回避をしない。
5. 完了したら、issue に §5 の形で報告する。

## 4. やってよいこと・いけないこと

**してよい（聞かなくてよい）**: 読む、調べる、文書を書く、テストを足す、自分の worktree でのビルドとテスト（`cargo test -p <crate>`、`cargo clippy -p <crate> --all-targets -- -D warnings`、`cargo fmt`）、PR の作成、CI の確認。

**してはいけない**:
- main への直接コミットと push、PR のマージ、`--no-verify`。
- ユーザーデータ（`~/Library/Application Support/ikari-ime/`）と `data/raw/` の書き換え。
- ikari の install、smoke、アプリの起動し直し（リードが行う）。システムのプロセスを kill しない。入力ソースの設定を変えない。
- Windows の VM の操作（リードが行う）。
- 評価の測定（`ikari-eval` の LM あり、`--perf`、snapshot）。Mac の CPU を占有し、リードの測定を揺らす。必要なら §5 の形で依頼する。
- 値（加点、減点、タイムアウト）を計測なしで決める。決める前に、掃引で測る。
- 個人情報、住所、ホームのパス（`/Users/…`。gitleaks が止める）、旧リポジトリの名前を、issue・PR・文書・コミットに書く。
- 設計の決定（ADR の「決定」）。調べて、選択肢と推奨を書くところまで。

## 5. すり合わせの形式（GitHub の issue と PR のコメントで行う）

会話の中だけに残さない。リードは issue と PR だけを引き継ぎの元にする。

**着手**: `着手します。ブランチ: <名前>。見込み: <1〜2 行>。`

**質問・相談**（決めてほしいこと）:
```
## 相談
- 分かったこと: …
- 推測（未確認）: …
- 選択肢と推奨: A …（推奨）／B …
- 聞きたいこと: …（2〜3 個まで）
```

**共有資源の依頼**（Mac の測定、VM、smoke、XCFramework のビルド）: `依頼: <何を、なぜ、いつまでに>`。リードが順番を決めて、結果をコメントで返す。

**完了の報告**:
```
## 完了
- PR: #…
- 分かったこと: …
- 推測（未確認）: …
- 確かめたこと（コマンドと結果）/ 確かめていないこと: …
- ユーザーの確認が要ること: …
```

事実と推測を分ける。ログ、CLI、評価で確かめる前に、原因を言い切らない。

## 6. 共有している資源

Mac の CPU、ディスク、Windows の VM、ユーザーの ikari の入力（smoke のロック）は、リードと取り合いになる。

- 重いビルドを同時に走らせない。**自分の worktree の `target/` は 20〜40GB になる**。使い終わった worktree の `target/` は消す（`rm -rf target`）。ディスクの空きが 50GB を切ったら、リードに知らせる。
- フルの `scripts/check.sh`（XCFramework のビルドを含む）はリードが流す。手元では上の crate 単位のコマンドか、文書だけなら `scripts/check.sh --docs-only` まで。
- 文書だけの変更でも、リンクの検査（`--docs-only`）は流す。

## 7. 最初に読むもの

1. 担当の issue と、そこから張られた ADR（`docs/adr/`。索引は `docs/adr/README.md`）。
2. `docs/ikari-ime_spec.md` の「基本原則」（入力を正とする、学習は支援であって決定ではない）。
3. `docs/user-feedback.md`（ユーザーの指摘と決定。同じ失敗を繰り返さないため）。
4. [CLAUDE.md](../CLAUDE.md) の「守る決定」。
