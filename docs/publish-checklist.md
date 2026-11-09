# 公開の手順書（ソースだけを公開する）

このリポジトリを、非公開から公開に切り替えるときの手順です。チェックボックスを埋めながら進めます（保守者向け）。

**リポジトリを公開にする操作そのものは、AI エージェントは行いません。** 作者（リポジトリの管理者）が、GitHub の画面で行います。エージェントが手伝うのは、検査と文書の準備までです。

方針（2026-11-10 に作者が決定。Issue #99）: 公開は準備が済んだらすぐ行い、まず**ソースだけ**を公開します。ビルド済みのアプリの配布（GitHub Releases）は、#241（未署名のアプリを開く手順の実機確認）のあとです。

用語: `#番号` は、このリポジトリの GitHub の Issue 番号か Pull Request の番号です。

## 1. 公開の前

### 1.1 個人情報と秘密の走査

公開すると、履歴、Issue、PR のすべてが見えます。リポジトリ全体と、GitHub 上の文章を、もう一度走査します。個人名、住所、メールアドレス、パスワード、鍵、ホームディレクトリのパス、旧リポジトリのアカウントや名前は、書かれていないことを確かめます。走査する語は、`.gitleaks.toml` の規則と、公開しない `.gitleaks.local.toml`（作者の手元にだけある）の規則に従います。

- [ ] 追跡しているファイル全体を `git grep` で探す。語は、`.gitleaks.local.toml` の規則にある語と、次の例の形に合わせる。

  ```sh
  git grep -nE '/Users/[A-Za-z0-9._-]+' -- . ':!THIRD_PARTY_LICENSES_RUST.md'   # ホームのパス（ダミーの someone と u は問題なし）
  git grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}' | grep -v noreply   # メールアドレスの形
  ```

- [ ] 履歴の全部のコミットを走査する（作者欄のメールが noreply だけであることも `scripts/check.sh` が確かめる）。

  ```sh
  gitleaks git . --config .gitleaks.toml --redact --no-banner
  gitleaks git . --config .gitleaks.local.toml --redact --no-banner   # 作者の手元だけ
  git log --all --format='%an <%ae>%n%cn <%ce>' | sort -u                # 作者とコミッターの一覧
  ```

- [ ] コミットメッセージを探す（上の `git grep` と同じ語を、履歴に対して）。

  ```sh
  git log --all --format='%H%n%B' | grep -nE '/Users/|@[A-Za-z0-9-]+\.[a-z]{2,}' | grep -v noreply
  ```

- [ ] Issue を**全件**取得して探す（本文、コメント、タイトル。閉じたものも）。

  ```sh
  gh issue list --state all --limit 2000 --json number,title,body,comments > /tmp/ikari-issues.json   # 場所は作業用のフォルダでよい。終わったら消す
  grep -nE '/Users/|[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}' /tmp/ikari-issues.json | grep -v noreply
  ```

  `.gitleaks.local.toml` の語も同じ形で探します。件数が `limit` に達していないことを、`gh issue list --state all --limit 1 --json number` の最大の番号と見比べて確かめます。
- [ ] Pull Request も同様に、全件を取得して探す（本文、コメント、レビューのコメント）。

  ```sh
  gh pr list --state all --limit 2000 --json number,title,body,comments,reviews > /tmp/ikari-prs.json
  ```

- [ ] 見つかったものは、書き直すか、Issue と PR の本文を編集する。コミットメッセージに残るものは、公開をいったん止めて、作者に相談する（履歴の書き換えは作者が決める）。
- [ ] 走査に使った一時ファイルを消す。

### 1.2 検査の全部を通す

- [ ] `scripts/check.sh` を、**フルで**流して、`CHECK_EXIT=0` になること（fmt、clippy、`cargo test`、`swift test`、文書のリンク、gitleaks を含む）。`macos/Frameworks` が古いと止まるので、そのときは `macos/scripts/build.sh --signing team --release` を先に流す。
- [ ] `cargo xtask licenses --check` が通ること（Rust の依存のライセンス一覧 [THIRD_PARTY_LICENSES_RUST.md](../THIRD_PARTY_LICENSES_RUST.md) が最新）。
- [ ] 新しいクローン（別のフォルダ）で、README の「ビルドと検証だけ」の手順を、そのまま流して通ること。README に書いた時間と容量と、手順の名前が合っていること。

### 1.3 ライセンスと表示

- [ ] [LICENSE](../LICENSE)（Apache License 2.0）と [NOTICE](../NOTICE) が、リポジトリの直下にあること。
- [ ] [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) の、辞書、モデル、地名のデータの条件が、取得元の今の表示と合っていること（zenz は Hugging Face の表示を、公開の直前にもう一度見る）。
- [ ] 辞書の元データ、モデルのファイル、ビルド済みの辞書を、リポジトリに**含めていない**こと（`git ls-files data` が空。大きなファイルがないことを `git ls-files -z | xargs -0 du -ch | tail -1` で見る）。

### 1.4 公開する形

- [ ] 公開するのは、**ソースだけ**。Releases は作らない（ビルド済みの配布は #241 のあと）。
- [ ] [README](../README.md) の冒頭が「ビルド済みのアプリは、まだ配っていません」になっていること。[SECURITY.md](../SECURITY.md) も同じ内容であること。
- [ ] [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md)、[CONTRIBUTING.md](../CONTRIBUTING.md)、[CHANGELOG.md](../CHANGELOG.md) があること。

## 2. 公開の操作（作者が行う。エージェントはしない）

- [ ] 作者が、GitHub のリポジトリの Settings → General → Danger Zone の **Change visibility** から、Public に切り替える。
- [ ] 切り替えたら、ログインしていないブラウザで、リポジトリの URL が開けることを確かめる。

## 3. 公開のあと、すぐ

- [ ] **外来語の一覧の固定**（PR #235、ブランチ `feat/webwords-pin-20261101`）: マージの前に、main の最新を取り込み（`gh pr update-branch 235`）、フルの `scripts/check.sh` を流して `CHECK_EXIT=0` を確かめる。マージは、作者の方針に沿って行う。
- [ ] 認証なしで取得できること。公開前は認証なしの curl が 404 になるので、この確認は公開のあとにしかできない。

  ```sh
  scripts/fetch-data.sh --only webwords
  ```

  失敗したら、固定した URL と sha256（`scripts/lib.sh`）を確かめる。
- [ ] Settings → Code security（Advanced Security）で、次を有効にする。
  - [ ] Dependency graph と **Dependabot alerts**（設定ファイルは `.github/dependabot.yml`。週 1 回、更新の PR を出す）
  - [ ] **Secret scanning** と **Push protection**
  - [ ] Code scanning（CodeQL。`.github/workflows/codeql.yml` が main への push と PR、週 1 回で動く）の最初の結果を確かめる
- [ ] **ブランチ保護**（Settings → Rules → Rulesets、または Branches）。今は、main への直接のコミットと push を、手元の `.githooks/`（`git config core.hooksPath .githooks` で有効）と、Claude Code のフックで止めている。これらは手元でしか効かないので、GitHub の設定でも止める。
  - [ ] main への直接の push を禁止する（PR を必須にする）。
  - [ ] 必要なチェックに、CodeQL を入れる。`scripts/check.sh`（fmt、clippy、テスト）は、手元で流す検査で、GitHub では動かない。必要なチェックには入れられないので、作者が PR ごとに `CHECK_EXIT=0` を確かめる（今の運用）。CI で流す案は、別の Issue で決める。
  - [ ] マージの方法は squash だけにする（Settings → General → Pull Requests）。マージ後にブランチを自動で消す。
  - [ ] force push と、main の削除を禁止する。
- [ ] リポジトリの説明（About）とトピックを設定する。例: 説明は「文脈を使って同音異義語を選び分ける日本語入力メソッド（macOS）」、トピックは `ime`、`japanese`、`macos`、`input-method`、`rust`、`swift`。
- [ ] Issue テンプレート（`.github/ISSUE_TEMPLATE/`。不具合の報告、作業の項目）が、ログインしていない人にも見える形で出ること。`blank_issues_enabled` は今 `true`（テンプレートなしの Issue も可）。このままでよいかを作者が決める。
- [ ] **Discussions は使わない**（Settings → General → Features で Discussions がオフのまま）。質問と提案は Issue で受ける。
- [ ] Security タブの **Private vulnerability reporting** を有効にする（[SECURITY.md](../SECURITY.md) と [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) の連絡先がこれなので、有効でないと届かない）。
- [ ] 最初の数日は、新しい Issue と PR を見て、個人的な文章が書かれていたら、本文を編集してもらう。

## 4. ビルド済みの配布を作るとき（今回はしない）

- [ ] #241（未署名のアプリを開く手順の実機確認）が済んでいること。
- [ ] README の冒頭（「ビルド済みのアプリは、まだ配っていません」）と SECURITY.md の同じ行を、配布に合わせて書き直す。
- [ ] [CHANGELOG.md](../CHANGELOG.md) の 0.1.0 のリリース日を入れる。
- [ ] 配布物に含める依存のライセンスの表示を確かめる（`cargo xtask licenses --check`、[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)）。
- [ ] GitHub Releases にバイナリを置く。
