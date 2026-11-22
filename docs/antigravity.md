# Antigravity（Gemini）のレーン向けの指示

ikari-ime は、Claude Code が**リード**、Antigravity が**レーン**として分担して進める（2026-11-22 ユーザー決定）。この文書は Antigravity のための指示。リポジトリの共通の規則は [CLAUDE.md](../CLAUDE.md)（`AGENTS.md` は同じファイル）にある。**CLAUDE.md の「リードは…」「サブエージェントを起動する」の部分は、リード（Claude Code）への指示で、あなた（Antigravity）には当てはまらない。** このレーン向けの読み替えは、この文書が優先する。

## 1. 役割と、つなぎ方

| | 持つもの |
| --- | --- |
| **リード（Claude Code）** | 優先順位、ADR と設計、ユーザーへの確認、統合、`scripts/check.sh`、PR のマージ、Mac の重い作業（XCFramework のビルド、評価の測定、smoke、install）、Windows の VM の操作 |
| **Antigravity** | 割り当てられた issue の調査・文書・テスト・実装。1 issue につき 1 ブランチ・1 PR |

- **つなぎ方は GitHub だけ。** Antigravity は「issue を受け持つ別の人」として動く。ユーザーが Antigravity を起動したときに、GitHub に自分の担当の issue（ラベル `antigravity`）があれば、それをやる。
- **Claude Code から Antigravity を自動で呼ぶ（プロンプトを送る、プロセスを起こす、など）ことはしない**（2026-11-22 ユーザー決定）。Antigravity か Gemini の利用規約に触れるおそれがあるため。**規約の中身は未確認**で、ユーザーが確かめたら、この文書を更新する。確かめるまでは、自動で呼ぶ経路を作らない。
- 方針の決定はユーザー（作者）が行う。数値や方針を勝手に決めない。迷ったら §5 の形で issue に書いて、リードに聞く。
- ラベルのない issue には着手しない。

## 2. 作業場所（別の clone。リードの作業コピーに触らない）

リードは `~/github.com/ikari-zen/ikari-ime` で jj を使っている。**そこでは作業しない。** ref、jj の操作ログ、`scripts/check.sh` の合格の記録は、同じリポジトリの中で共有されるので、同じリポジトリを分け合うと、お互いの操作が競合する（Opus のレビュー、2026-11-22）。**別の clone** で作業する:

```sh
git clone https://github.com/ikari-zen/ikari-ime.git ~/github.com/ikari-zen/ikari-ime-ag
cd ~/github.com/ikari-zen/ikari-ime-ag
git config core.hooksPath .githooks
git config user.name "Antigravity (<作業するモデルの名前>)"   # 例: Antigravity (Gemini 4 Neon)、Antigravity (Claude Sonnet 5.5)
```

- **作者名は「Antigravity (モデル名)」にする**（2026-11-22 ユーザー決定）。リードのコミットと区別し、どのモデルが作業したかが履歴で分かるようにする。**セッションでモデルを変えたら、作業の前に `git config user.name` を直す**（例: `Antigravity (Gemini 4 Neon)`、`Antigravity (Claude Sonnet 5.5)`）。PR の説明の「実施モデル」と、issue の完了コメントの「実施モデル」にも、同じ名前を書く。
- **コミットの末尾に Co-Authored-By の行を付ける**（Claude Code と同じ運用。2026-11-22 ユーザー決定）。作者名と同じ「Antigravity (モデル名)」で、メールは Claude Code と同じ noreply@anthropic.com（届かないアドレス。2026-11-22 ユーザー決定）:
  ```
  Co-Authored-By: Antigravity (Gemini 4 Neon) <noreply@anthropic.com>
  ```
  squash マージのあとの main には、作者名（`user.name`）は残らず、この行だけが残る。どのモデルが書いたかを履歴で追えるように、必ず付ける。PR の説明の末尾にも、同じ名前で生成元を書く。
- jj は使わない（git だけ）。リードの jj のリポジトリに、あなたの ref を入れない。
- コミットの作者の email は、ユーザーに確認してから設定する（新しい個人情報を書かない）。
- 同じ Mac で動くので、あなたの clone の `target/` は 20〜40GB になる。使い終わったら `rm -rf target`。ディスクの空きが 50GB を切ったら、リードに知らせる。

## 3. 進め方（ラベルで状態を示す）

| ラベル | 意味 | 誰が付ける |
| --- | --- | --- |
| `antigravity` | あなたに割り当てた | リード（またはユーザー） |
| `ag:wip` | 着手した | あなた |
| `ag:question` | 相談したい（リードの返事待ち） | あなた |
| `ag:done` | PR を出した（リードの確認待ち） | あなた |

1. 着手: issue に「着手します（ブランチ名）」とコメントし、`ag:wip` を付ける。同じアカウントを共有しているので、assignee では区別しない。
2. ブランチを切り（`feat/…`、`fix/…`、`docs/…`）、**すぐ** `git push -u origin <ブランチ>`。意味のある小さな単位ごとにコミットして push する。
3. PR を作る（draft でよい）。説明に `Closes #番号`、「判断の置き場所」（OS に依存しないものは Rust。Swift や C# に判断を足すときは理由を書く）、分かったことと推測を分けて書く。
4. **マージしない。** マージはリードが行う。リードは、あなたの PR の先頭コミットを自分の作業コピーで `scripts/check.sh`（フル）を流し直し、CI が緑であることを確かめてからマージする。main へ直接 push しない。`--no-verify` やフックの回避をしない。
5. 完了したら、§5 の形で報告し、`ag:wip` を外して `ag:done` を付ける。何日も動かない issue は、リードが引き取るか、ラベルを外す。

## 4. やってよいこと・いけないこと

**してよい（聞かなくてよい）**: 読む、調べる、文書を書く、テストを足す、自分の clone でのビルドとテスト（`cargo test -p <crate>`、`cargo clippy -p <crate> --all-targets -- -D warnings`、`cargo fmt`）、PR の作成、CI の確認。

**してはいけない**:
- main への直接コミットと push、PR のマージ（`gh pr merge`、ブラウザでも）、`--no-verify`。
- `gh api` での設定の変更、`gh secret`、`gh auth`、ブランチ保護や ruleset の変更。
- **`scripts/check.sh` を流さない。** 合格の記録は、リードのマージの条件に使われる。あなたの clone は別のリポジトリなので、記録は混ざらないが、リードが流し直すので不要。文書だけの変更の確認は、リンクの検査（`scripts/check.sh --docs-only`）まで。
- ユーザーデータ（`~/Library/Application Support/ikari-ime/`）と `data/raw/` の書き換え。署名鍵、証明書、`~/.ssh` に触らない。
- **GitHub の読み取りの権限**（2026-11-23 ユーザー決定）: Antigravity には、**専用の fine-grained トークン**を `GH_TOKEN` で渡す（リードやユーザーの `gh` の権限とは別にする）。対象はこのリポジトリだけ、有効期限を付ける（30〜90 日）。権限は**読み取りだけ**: Code scanning alerts、Dependabot alerts、Secret scanning alerts、Actions（CI のログ）、Metadata。Issues と Pull requests は今の運用の範囲（コメントと PR）。**Contents の書き込みと Administration は付けない。**
- **アラート（Code scanning など）の内容も、ADR 0070 §5 の非公開の扱いに含める**: issue・PR・コミットに、問題の中身を書かない。件数と場所（ファイルと行）、対応の有無だけを書く。アラートの却下・再開（状態の変更）は、リードが行う。
- ikari の install、smoke、アプリの起動し直し（リードが行う）。システムのプロセスを kill しない。入力ソースの設定を変えない。
- Windows の VM の操作（リードが行う）。
- 評価の測定（`ikari-eval` の LM あり、`--perf`、snapshot）。Mac の CPU を占有し、リードの測定を揺らす。必要なら §5 の形で依頼する。
- 契約（共有の型、proto、シグネチャ）の変更と、リードやほかのレーンが触っているファイルの変更。
- 値（加点、減点、タイムアウト）を計測なしで決める。
- 個人情報、住所、ホームのパス（`/Users/…`。gitleaks が止める）、旧リポジトリの名前を、issue・PR・文書・コミットに書く。
- **外部の MCP（Context7 など）に送る質問に、非公開のレビューの内容、指摘、パス、個人情報を入れない**（質問文は外部のサービスに送られる。2026-11-22 ユーザー決定）。使うのはライブラリ名と API 名の調べものだけ。MCP は版を固定して入れる（`npx` で最新を取らない。供給網の方針）。
- 設計の決定（ADR の「決定」）。調べて、選択肢と推奨を書くところまで。

**割り当てに向く issue**: 決定済みの ADR の範囲の実装、テストの追加、crate 単位で `cargo test -p` が完結する Rust の作業、文書、調査。**向かない issue**: Mac の測定、VM、smoke、XCFramework や Swift の実行が要るもの、変換の値の掃引、ADR の決定、ユーザーの判断が要るもの。向かないものは、リードが割り当てない。割り当てられたものが向かないと分かったら、§5 の形で相談する。

## 5. すり合わせの形式（GitHub の issue と PR のコメントで行う）

会話の中だけに残さない。リードは issue と PR だけを引き継ぎの元にする。

**着手**: `着手します。ブランチ: <名前>。見込み: <1〜2 行>。`（`ag:wip`）

**質問・相談**（決めてほしいこと。`ag:question` を付ける）:
```
## 相談
- 分かったこと: …
- 推測（未確認）: …
- 選択肢と推奨: A …（推奨）／B …
- 聞きたいこと: …（2〜3 個まで）
```

**共有資源の依頼**（Mac の測定、VM、smoke、XCFramework のビルド）: `依頼: <何を、なぜ、いつまでに>`。リードが順番を決めて、結果をコメントで返す。

**完了の報告**（`ag:done` を付ける）:
```
## 完了
- PR: #…
- 分かったこと: …
- 推測（未確認）: …
- 確かめたこと（コマンドと結果）/ 確かめていないこと: …
- ユーザーの確認が要ること: …
```

事実と推測を分ける。ログ、CLI、評価で確かめる前に、原因を言い切らない。

## 6. 最初に読むもの

1. 担当の issue と、そこから張られた ADR（`docs/adr/`。索引は `docs/adr/README.md`）。
2. `docs/ikari-ime_spec.md` の「基本原則」（入力を正とする、学習は支援であって決定ではない）。
3. `docs/user-feedback.md`（ユーザーの指摘と決定。同じ失敗を繰り返さないため）。
4. [CLAUDE.md](../CLAUDE.md) の「守る決定」。

## 7. 未確認のこと（ユーザーが確かめたら更新する）

- Antigravity と Gemini の利用規約（自動化、プログラムからのアクセス、ほかのサービスとの連携の条項）。
- Antigravity が `AGENTS.md` と `GEMINI.md` のどちらを読み、どちらを優先するか。
- Antigravity に、コマンドの自動実行の許可・拒否の設定があるか。**あるなら、自動実行を切り、`gh pr merge`、`gh api`、`gh secret`、`gh auth`、`install.sh`、`smoke`、`rm` の対象のパスを拒否に入れる。** ない場合は、この文書の約束だけが頼りになる。事故が出たら、write 権限のない別の GitHub アカウント（fork からの PR）か、別の macOS ユーザーに切り替える。
