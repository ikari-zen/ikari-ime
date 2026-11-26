# 公開の手順書（ソースだけを公開する）

このリポジトリを、非公開から公開に切り替えるときの手順です。チェックボックスを埋めながら進めます（保守者向け）。

**リポジトリを公開にする操作そのものは、AI エージェントは行いません。** 作者（リポジトリの管理者）が、GitHub の画面で行います。エージェントが手伝うのは、検査と文書の準備までです。

方針（2026-11-10 に作者が決定。Issue #99）: 公開は準備が済んだらすぐ行い、まず**ソースだけ**を公開します。ビルド済みのアプリの配布（GitHub Releases）は、#241（未署名のアプリを開く手順の実機確認）のあとです。

用語: `#番号` は、このリポジトリの GitHub の Issue 番号か Pull Request の番号です。

## 1. 公開の前

### 1.1 個人情報と秘密の走査

公開すると、履歴、Issue、PR のすべてが見えます。リポジトリ全体と、GitHub 上の文章を、もう一度走査します。個人名、住所、メールアドレス、パスワード、鍵、ホームディレクトリのパス、旧リポジトリのアカウントや名前は、書かれていないことを確かめます。走査する語は、`.gitleaks.toml` の規則と、公開しない `.gitleaks.local.toml`（作者の手元にだけある）の規則に従います。

- [x] 追跡しているファイル全体を `git grep` で探す。語は、`.gitleaks.local.toml` の規則にある語と、次の例の形に合わせる。

  ```sh
  git grep -nE '/Users/[A-Za-z0-9._-]+' -- . ':!THIRD_PARTY_LICENSES_RUST.md' ':!THIRD_PARTY_LICENSES_RUST_WINDOWS.md'   # ホームのパス（ダミーの someone と u は問題なし）
  git grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}' | grep -v noreply   # メールアドレスの形
  ```

- [x] 履歴の全部のコミットを走査する（作者欄のメールが noreply だけであることも `scripts/check.sh` が確かめる）。

  ```sh
  gitleaks git . --config .gitleaks.toml --redact --no-banner
  gitleaks git . --config .gitleaks.local.toml --redact --no-banner   # 作者の手元だけ
  git log --all --format='%an <%ae>%n%cn <%ce>' | sort -u                # 作者とコミッターの一覧
  ```

- [x] コミットメッセージを探す（上の `git grep` と同じ語を、履歴に対して）。

  ```sh
  git log --all --format='%H%n%B' | grep -nE '/Users/|@[A-Za-z0-9-]+\.[a-z]{2,}' | grep -v noreply
  ```

- [x] Issue を**全件**取得して探す（本文、コメント、タイトル。閉じたものも）。

  ```sh
  gh issue list --state all --limit 2000 --json number,title,body,comments > /tmp/ikari-issues.json   # 場所は作業用のフォルダでよい。終わったら消す
  grep -nE '/Users/|[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}' /tmp/ikari-issues.json | grep -v noreply
  ```

  `.gitleaks.local.toml` の語も同じ形で探します。件数が `limit` に達していないことを、`gh issue list --state all --limit 1 --json number` の最大の番号と見比べて確かめます。
- [x] Pull Request も同様に、全件を取得して探す（本文、コメント、レビューのコメント）。

  ```sh
  gh pr list --state all --limit 2000 --json number,title,body,comments,reviews > /tmp/ikari-prs.json
  ```

- [x] 見つかったものは、書き直すか、Issue と PR の本文を編集する。コミットメッセージに残るものは、公開をいったん止めて、作者に相談する（履歴の書き換えは作者が決める）。
- [x] 走査に使った一時ファイルを消す。

### 1.2 検査の全部を通す

- [x] `scripts/check.sh` を、**フルで**流して、`CHECK_EXIT=0` になること（fmt、clippy、`cargo test`、`swift test`、文書のリンク、gitleaks を含む）。`macos/Frameworks` が古いと止まるので、そのときは `macos/scripts/build.sh --signing team --release` を先に流す。
- [x] `cargo xtask licenses --check` が通ること（Rust の依存のライセンス一覧 [THIRD_PARTY_LICENSES_RUST.md](../THIRD_PARTY_LICENSES_RUST.md)（macOS）と [THIRD_PARTY_LICENSES_RUST_WINDOWS.md](../THIRD_PARTY_LICENSES_RUST_WINDOWS.md)（Windows）が最新）。
- [x] 新しいクローン（別のフォルダ）で、README の「ビルドと検証だけ」の手順を、そのまま流して通ること。README に書いた時間と容量と、手順の名前が合っていること。

### 1.3 ライセンスと表示

- [x] [LICENSE](../LICENSE)（Apache License 2.0）と [NOTICE](../NOTICE) が、リポジトリの直下にあること。
- [x] [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) の、辞書、モデル、地名のデータの条件が、取得元の今の表示と合っていること（zenz は Hugging Face の表示を、公開の直前にもう一度見る）。（2026-11-11 に確認。zenz: Hugging Face の表示は CC BY-SA 4.0。SudachiDict と chiVe: GitHub の表示は Apache-2.0。ABR: デジタル庁の利用規約（最終更新 2026-05-29）は PDL1.0 で、出典と加工の明記が要る。いずれも記載と一致。日本郵便の郵便番号データの再配布の可否は、取得元に記載がなく、これまでどおり「確かめていない」。ikari-ime は表を配らない）
- [x] 取得して使うモデル（zenz）の更新を確かめること。Hugging Face の API でコミット履歴とファイルの sha256 を見て、固定した sha256（`resource/src/models.rs` の `ZENZ_SMALL_SHA256` と `xtask/src/data/fetch.rs` の `SHA_ZENZ_SMALL`）と合っていること（`curl -s https://huggingface.co/api/models/Miwa-Keita/zenz-v3.2-small-gguf/commits/main`、`…/zenz-v3.2-small-gguf?blobs=true` の `lfs.sha256`）。ファイルが変わっていたら、ADR 0058 の手順（新しい sha256 を評価で確かめてから表を更新）に従う。README だけの更新は影響なし（2026-11-16 に確認: 最新の更新 2026-11-07 は README のみ。モデルのファイルは 2026-05-06 のまま、sha256 先頭 29c223d4c233…）。新しい版（zenz-v3.2 より新しいもの、xsmall など）の有無も、作者の一覧（`https://huggingface.co/api/models?author=Miwa-Keita`）で見る（#440）。（2026-11-26 に確認: 最新のコミットは 2026-11-07 の README のみ。モデルのファイルは 2026-05-06 のまま、sha256 先頭 29c223d4c233。v3.2 より新しい版はない。xsmall はある）
- [x] 辞書の元データ、モデルのファイル、ビルド済みの辞書を、リポジトリに**含めていない**こと（`git ls-files data` が空。大きなファイルがないことを `git ls-files -z | xargs -0 du -ch | tail -1` で見る）。

### 1.4 公開する形

- [x] 公開するのは、**ソースだけ**。Releases は作らない（ビルド済みの配布は #241 のあと）。
- [x] [README](../README.md) の冒頭が「ビルド済みのアプリは、まだ配っていません」になっていること。[SECURITY.md](../SECURITY.md) も同じ内容であること。
- [x] [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md)、[CONTRIBUTING.md](../CONTRIBUTING.md)、[CHANGELOG.md](../CHANGELOG.md) があること。

## 2. 公開の操作（作者が行う。エージェントはしない）

- [x] 作者が、GitHub のリポジトリの Settings → General → Danger Zone の **Change visibility** から、Public に切り替える。
- [x] 切り替えたら、ログインしていないブラウザで、リポジトリの URL が開けることを確かめる。

## 3. 公開のあと、すぐ

（2026-11-23 に、GitHub の設定を `gh` で確かめて、済んだものにチェックを付けた。未確認のものは未チェックのまま）

- [x] **外来語の一覧の固定**（PR #235、ブランチ `feat/webwords-pin-20261101`）: マージの前に、main の最新を取り込み（`gh pr update-branch 235`）、フルの `scripts/check.sh` を流して `CHECK_EXIT=0` を確かめる。マージは、作者の方針に沿って行う。
- [ ] 認証なしで取得できること。公開前は認証なしの curl が 404 になるので、この確認は公開のあとにしかできない。

  ```sh
  scripts/fetch-data.sh --only webwords
  ```

  失敗したら、固定した URL と sha256（`scripts/lib.sh`）を確かめる。
- [ ] Settings → Code security（Advanced Security）で、次を有効にする。
  - [x] Dependency graph と **Dependabot alerts**（設定ファイルは `.github/dependabot.yml`。週 1 回、更新の PR を出す）
  - [x] **Secret scanning** と **Push protection**
  - [x] 依存の定期検査（`.github/workflows/audit.yml`。週 1 回の `cargo audit`）を、Actions の画面から手動で 1 回動かして、通ることを確かめる。失敗の通知が届く設定（Settings → Notifications）も見る。
  - [x] Code scanning（CodeQL。`.github/workflows/codeql.yml` が main への push と PR、週 1 回で動く）の最初の結果を確かめる
- [ ] **ブランチ保護**（Settings → Rules → Rulesets、または Branches）。今は、main への直接のコミットと push を、手元の `.githooks/`（`git config core.hooksPath .githooks` で有効）と、Claude Code のフックで止めている。これらは手元でしか効かないので、GitHub の設定でも止める。
  - [x] main への直接の push を禁止する（PR を必須にする）。
  - [x] 必要なチェックに `check`（と `rust`。`.github/workflows/ci.yml`。macOS ランナーで `scripts/check.sh --swift-only`。XCFramework の鮮度、swift test、entitlement の検査。文書・Rust の検査は Ubuntu の `rust` job が `--rust-only` で流す）を入れた（2026-11-11 作者決定。#290）。最新の main を取り込んだものだけ通す（strict）。Ruleset「protect main」に例外（バイパス）は置かない（作者も、`check` が通っていないものはマージできない）。手元の `CHECK_EXIT=0` の確認は、引き続き行う。
  - [x] マージの方法は squash だけにする（Settings → General → Pull Requests）。マージ後にブランチを自動で消す。
  - [x] force push と、main の削除を禁止する。
- [x] **CI を足した**（#290、PR #292。#295 で分割）。`rust`（Ubuntu。`--rust-only`）と `check`（macOS。`--swift-only`）の 2 つに分けて流す。Ruleset「protect main」の必須チェックは `check` と `rust`（strict）。`deny`、`windows`、`windows-artifact` は必須外（2026-11-23 時点の Ruleset の設定）。テストの時間は `IKARI_TEST_TIME_SCALE=5`（CONTRIBUTING.md）。
- [ ] リポジトリの説明（About）とトピックを設定する。例: 説明は「文脈を使って同音異義語を選び分ける日本語入力メソッド（macOS）」、トピックは `ime`、`japanese`、`macos`、`input-method`、`rust`、`swift`。
- [ ] Issue テンプレート（`.github/ISSUE_TEMPLATE/`。不具合の報告、作業の項目）が、ログインしていない人にも見える形で出ること。`blank_issues_enabled` は今 `true`（テンプレートなしの Issue も可）。このままにする（2026-11-10 作者決定）。
- [x] **Discussions は使わない**（Settings → General → Features で Discussions がオフのまま）。質問と提案は Issue で受ける。
- [x] Security タブの **Private vulnerability reporting** を有効にする（[SECURITY.md](../SECURITY.md) と [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) の連絡先がこれなので、有効でないと届かない）。
- [ ] 最初の数日は、新しい Issue と PR を見て、個人的な文章が書かれていたら、本文を編集してもらう。

## 4. ビルド済みの配布（0.1.0 タグ打ちから公開までの手順書・下書き）

設計は [security-design.md](security-design.md)（[ADR 0069](adr/0069-distribution-authenticity.md)、[ADR 0070](adr/0070-update-and-vulnerability-response.md)、[ADR 0076](adr/0076-ikari-data-contract.md) §18.2、[ADR 0080](adr/0080-bundle-model-and-dictionary.md) §9）。0.1.x は macOS 版（Apple Silicon、通常版・オフライン版の 2 種、未署名）だけを配り、Windows 版の同梱は 0.2.0 からの予定（#99、[CHANGELOG.md](../CHANGELOG.md)）。1.0.0 は独自の AI モデルを組み込めた時点。Windows 版の公開前に追加で要る項目は §4.9 に分ける。

以下の workflow の動きは、`.github/workflows/release.yml` を読んで合わせたもの（食い違ったら workflow が正しい。この節を直す）。

この節は 0.1.0 の初回公開に向けた**下書き**であり、タグからの本番 workflow の実走や実機確認が済んでいない項目には**未確認**と記す。

### 役割分担（AI エージェントがしてよい操作／作者だけが行う操作）

| 区分 | 操作の例 |
| --- | --- |
| **作者のみ（AI エージェントは実行しない）** | タグの作成・push（`git tag` / `git push`）、作者の minisign 秘密鍵の作成・保管・`SHA256SUMS` への署名、下書き Release の削除、下書き Release への署名添付と公開（`--draft=false --latest` / Web UI）、GitHub のアカウント（2FA）・リポジトリ設定・Ruleset の変更、`.gitleaks.local.toml` を使った走査、公開可否の最終判断 |
| **AI エージェント可（準備・確認の補助）** | 公開規則での事前走査・`scripts/check.sh` などの検査準備、試しのビルド（`workflow_dispatch`）の起動（リードが起動してよい。§4.2）、ドキュメント（README・SECURITY.md・CHANGELOG.md・リリースノート案）の下書き、workflow の実行ログ・下書き資産一覧の読み取り確認、公開後の読み取り確認（`gh release view` 等。利用可能な権限の範囲に限る） |

---

### 4.1 事前の前提（タグを打つ前に済ませるもの）

#### アカウントとリポジトリの保護（設定変更は作者のみ）

- [x] 前の節（§3）の、Private vulnerability reporting が有効であること。
- [ ] Release を作れるアカウント（作者）に、2 要素認証（パスキーか物理キー）を設定したこと。SMS や認証アプリだけにしない（ADR 0076 §18.2。アカウントを取られると、偽の版を「新しい版」として勧めうるため）。**未確認**。
- [x] タグを守る ruleset を入れたこと（Settings → Rules → Rulesets、対象はタグ）。対象は `v*`（アプリ）と `webwords-*`（データ）。作れる者を絞り、付け替え（force push）と削除を禁じる。（2026-11-23 確認: Ruleset「protect tags」が active。禁じるのは creation、update、non_fast_forward、deletion。バイパスは Admin のロールだけ。「protect main」と違ってバイパスがあるのは、作成を絞るとき作者自身が作れなくなるのを避けるため。なお `abr-mirror-*` が対象に含まれているかは**未確認**。）
- [x] Releases を immutable（公開したら資産とタグを変えられない）にしたこと。Settings → General で有効にした（2026-11-23 確認: `GET /immutable-releases` が `enabled: true`）。公開した Release から効く。すでに公開したものには遡らない。
- [ ] 作者のリリースの鍵（minisign。Ed25519）を手元で作成・保管し、公開鍵を [README.md](../README.md)・[SECURITY.md](../SECURITY.md)・リポジトリ内（および [antivirus-false-positive.md](antivirus-false-positive.md)）へ記載したこと（ADR 0069 §5。秘密鍵は CI に置かない）。鍵の作成と公開鍵の配置先は**未確認**（未作成）。

#### 実機確認が前提の項目（確認は作者／リード。すべて未確認）

- [ ] **#241（実機確認が前提）**: macOS の未署名（ad-hoc）配布について、別アカウントまたは別 Mac へ隔離属性が付く経路で渡した状態で、(a) dmg の開き方、(b) `~/Library/Input Methods` での入力ソース追加・エンジン・設定アプリ・LM の動作、(c) 隔離属性が付いた場合の安全な許可手順、(d) dmg 内の `~/Library/Input Methods` へのショートカットと案内の形（ADR 0080 §5.1・§5.2）を実機で確かめ、確かめた手順だけを [README.md](../README.md) と [SECURITY.md](../SECURITY.md) に書いたこと（OS やウイルス対策の守りを切る手順は書かない）。**未確認**。
- [ ] **#459 の項目 6（実機確認が前提）**: ADR 0077 §8 の項目 6（#583 の確認項目へ移管済み。macOS の配布パッケージ構成での LM の檻の動作確認。あわせて #459 の ADR 0077 §8 項目 2 のアクティビティモニタ記録も残っている）が済んでいること。**未確認**。
- [ ] **#502（実機確認が前提）**: macOS の App Sandbox コンテナについて、同じ識別子で署名方式を切り替えたとき（ad-hoc ⇔ team、および ad-hoc 配布の更新時）に確認ダイアログが出る条件・文面・engine と lm-cage での再現有無を確かめ、必要な案内を [README.md](../README.md) と [SECURITY.md](../SECURITY.md) に書いたこと。**未確認**。
- [ ] **オフライン版の実機確認（ADR 0080 §10 C5・C6）**: macOS のオフライン版 dmg をネットワークを切った環境で動かし、同梱辞書と LM で変換でき、設定画面に通信機能（URL 取得・版の確認）が出ないこと、通常版でもオフライン状態で変換できることを確かめたこと。**未確認**。

#### workflow・文書・ライセンスの整備（下書きは AI エージェント可、確定・反映はリード／作者）

- [ ] `.github/workflows/release.yml` が 0.1.0 の配布範囲（macOS 通常版・オフライン版の 2 版）と整合し、タグからの公開経路のレビュー（#657）で挙がった修正が `main` に反映されていること。（#661 と #664 で、対象を macOS の 2 版にする分岐、成果物の明示した取り込み、`SHA256SUMS` の照合、下書きの重複防止、Windows を含める版を 0.2.0 以降にする変更は入った。本番のタグでの動きと、残りの指摘の扱いは**未確認**。）
- [ ] [security-design.md](security-design.md) §5.2 の「版を出す前」の検査のうち macOS 0.1.0 に該当するものが済んでいること（製品のビルドであること、Smoke 無し、App Sandbox と `network.*` entitlement の検査、オフライン版に通信の部品と取得の印が無いこと、他社の部品の版、3 つの証拠）。**未確認**。
- [ ] 配布物に含める依存と同梱データのライセンス表示を確かめたこと（`cargo xtask licenses --check`、[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)、および同梱データのそばの `NOTICE.txt`・`licenses/`。元ファイルは [dist-notices/NOTICE.txt](dist-notices/NOTICE.txt)）。
- [ ] [README.md](../README.md) の冒頭（「ビルド済みのアプリは、まだ配っていません」）と [SECURITY.md](../SECURITY.md) の「配布」の節を配布開始に合わせて書き直したこと。README には、配布元が Releases だけであること、sha256 の比べ方、証明と署名の確かめ方、実機で確かめた OS の警告の案内（守りを切る手順は書かない）、Watch の案内を書く。
- [ ] 配布物と [SECURITY.md](../SECURITY.md) の「約束すること」（通信しない、秘密の欄、ログ）および「約束しないこと」（`~/Library/Input Methods` の限界を含む。ADR 0080 §5.4）が合っていること。
- [x] 誤検出の報告の窓口: 公開後にウイルス対策ソフトの誤検出の報告が来たときの対応を、[SECURITY.md](../SECURITY.md) の「配布」に 1 行追記し、[antivirus-false-positive.md](antivirus-false-positive.md) に案内・確かめ方・Issue 窓口を下書き済み（公開鍵のありか・確定ファイル名・未署名警告の越え方は公開前に埋める）。
- [ ] [CHANGELOG.md](../CHANGELOG.md) の `0.1.0` のリリース日を入れ、未定の箇所（最低対応 macOS バージョン、配布物の確定名、公開鍵のありか等）を確定したこと。

---

### 4.2 公開直前の走査と検査（AI エージェント可／`.gitleaks.local.toml` と最終判断は作者）

タグを打つ直前の `main` 先頭コミットに対して行う。

- [ ] `Cargo.toml` の `[workspace.package] version` が `0.1.0`（タグの `v` を除いた値）と一致していること（`verify-tag` が一致を検査するのは**タグの push のときだけ**。試しのビルドでは検査しないので、ここで見る）。
- [ ] §1.1 と同じ観点で、直前の差分・履歴・新規 Issue / PR に個人情報、ホームのパス、秘密情報、非公開レビューの詳細（ADR 0070 §5）が混ざっていないことを走査したこと（`.gitleaks.local.toml` の走査は作者の手元のみ）。（注: 2026-11-26 の事前の走査では、公開規則の範囲で該当なし。`.gitleaks.local.toml` の走査と、タグの直前の最終の走査は未実施）
- [ ] リード（または作者）の手元で、対象コミットに対してフルの `scripts/check.sh` が `CHECK_EXIT=0` で通っていること。
- [ ] `cargo xtask licenses --check` が通ること。
- [ ] 同梱する他社部品・モデル・辞書の元データの版とライセンスを確認し、更新の有無をリリースノートに書く準備ができたこと（ADR 0070 §6、ADR 0080 §8 の 3）。
- [ ] タグを打つ前に、対象コミット（`main` の先頭）で `.github/workflows/release.yml` の試しのビルド（`workflow_dispatch`）を流し、`verify-tag`・`source-checks`・`build-dict`・`build-macos (normal)`・`build-macos (offline, -offline)` がすべて緑であることを確かめたこと。
  - **起動する人**: リードが起動してよい（作者が起動してもよい）。タグも Release も作らない。

    ```sh
    gh workflow run release.yml -R ikari-zen/ikari-ime --ref main
    ```

  - 試しのビルドでは `provenance` と `draft-release` はスキップされる。`verify-tag` のタグの検査（形、`main` に入っていること、`Cargo.toml` の版との一致）も流れない。
  - `build-windows` も流れる（版は `0.0.0`）。0.1.x のタグでは流れず配らないので、上の条件には入れていない。
  - 成果物は artifact `release-macos-normal`・`release-macos-offline`（と `release-windows`）に 7 日残る。
  - 記録: #661 の後の `main`（`8d6568a`）の試しのビルド（run `45304091723`）は全 job が成功した。#664 の後の `main` では**未確認**。

---

### 4.3 タグを打って push する（作者のみ。AI エージェントは実行しない）

配る物は、GitHub のランナーでタグから作ったものだけとする（作者や利用者の機械で作ったものは配らない。ADR 0069 §4.1）。

- [ ] 対象コミットが `origin/main` の最新で、§4.2 で検査し試しのビルドを流したコミットと同じであることを確認する。
- [ ] **作者が**手元で `v0.1.0` のタグを打ち、`origin` へ push する。`protect tags` は作成・付け替え・削除を禁じる（バイパスは Admin のロールだけ。§4.1）。**push したタグは、バイパスを使って付け替えたり削除したりせず、再利用もしない**（失敗したときは §4.4。同じタグの再実行はよい）。

  ```sh
  git fetch origin main
  git tag -a v0.1.0 -m "v0.1.0" origin/main
  git push origin v0.1.0
  ```

  （注: プレリリース形式 `v0.1.0-rc.1` 等は、付け方が決まるまで `verify-tag` が止める。）

---

### 4.4 Release workflow の実行と完了確認（閲覧は AI エージェント可／再実行・停止判断は作者）

`v0.1.0` の push で `.github/workflows/release.yml` が起動する。

| job | 0.1.x のタグでの動き |
| --- | --- |
| `verify-tag` | タグの形（プレリリースは止める）、`main` に入っていること、`Cargo.toml` の版との一致を検査。`include_windows` は `false`（0.2.0 以上で `true`） |
| `source-checks` | `cargo deny` と静的な検査 |
| `build-dict` | 同梱の辞書とモデル（artifact `bundle-data`） |
| `build-macos (normal)`・`build-macos (offline, -offline)` | dmg（artifact `release-macos-normal`・`release-macos-offline`） |
| `build-windows` | **スキップされる**（正常） |
| `provenance` | macOS の dmg 2 つだけを取り込み、`SHA256SUMS` を作り、2 つの dmg に証明を付ける（artifact `draft-release-assets`） |
| `draft-release` | `SHA256SUMS` と照合し、同じタグの Release（下書き・公開済み）があれば止まり、無ければ下書きを作る |

- [ ] GitHub Actions の `Release` workflow がタグ `v0.1.0` で起動し、`build-windows` を除く全 job が成功したことを確認する。
  - **未確認（本番のタグでしか確かめられない）**: `build-windows` がスキップされても `provenance` が動くこと、`actions/attest-build-provenance` が macOS の 2 つを載せた `SHA256SUMS` で通ること、`draft-release` の同じタグの Release の確認と `gh release create --draft --verify-tag` の動き、artifact の受け渡しと所要時間。
  - 注意: 公開リポジトリでは、`provenance` の証明の手順が通った時点で、下書きの公開前でも GitHub の公開の attestation の台帳に記録される。あとで失敗しても、記録は取り消せない前提で扱う。

#### 失敗したとき（復旧の手順は**すべて未実走**）

- [ ] 原因を特定し、下の表で対応を決める（再実行や版を上げる判断は作者）。
- **タグを変えるのは、中身（コードや workflow）を直すときだけ。** ランナーを確保できない、取得先の一時的な不調などは、同じタグの run の **「Re-run failed jobs」**（失敗した job の再実行。コミットもタグも変わらない）でやり直す（2026-11-26 ユーザー決定）。

| 失敗した場所 | 対応 |
| --- | --- |
| 検査、辞書、ビルドの job（`verify-tag`・`source-checks`・`build-dict`・`build-macos`）の一時的な失敗 | Re-run failed jobs（続きの job も流れる） |
| `provenance`（`draft-release-assets` を保存する前） | Re-run failed jobs |
| `draft-release` | 下書きが残っていれば**作者が**削除してから、Re-run failed jobs（下書きが残ったままだと「同じタグの Release がある」で止まる） |
| コードや workflow の直しが必要 | 次の版（`v0.1.1` など。番号と出し直すかは作者が決める）。失敗した版のタグは付け替えず、削除せず、再利用しない |

- **「Re-run all jobs」（全部の再実行）は使わない。** ビルドし直すと中身のバイト列が変わり、公開の証明の記録が増える。保存済みの artifact と名前が衝突するおそれもある。
- **再実行は 7 日以内にする。** 途中の artifact の保存期間が 7 日。
- `provenance` が `draft-release-assets` を保存したあとに `provenance` 自体を再実行すると、名前が衝突して失敗するおそれがある。その場合は次の版にする。
- 次の版にするときは、`Cargo.toml` の版と [CHANGELOG.md](../CHANGELOG.md) を PR で上げて `main` に入れ、§4.2 からやり直す（以降の `0.1.0` は読み替える）。`workflow_dispatch` の run は `provenance` と `draft-release` がスキップされるので、代わりにならない。
- どの場合も、公開前の下書きが残っていないかを Releases の画面で確かめる（下書きは作者が削除できる）。

---

### 4.5 下書き Release の確認（取得・照合の補助は AI エージェント可／採用判断は作者）

`draft-release` が作るのは**下書き（draft）**であり、まだ公開されていない。

- [ ] GitHub の Releases の画面で `v0.1.0`（題名もタグ名）が **Draft** の状態で、1 つだけあることを確認する。
- [ ] 下書きに添付された資産が、次の 3 つだけであることを確認する（Windows のインストーラは無い。GitHub が自動で付ける Source code の zip と tar.gz は資産に数えない）。
  - `ikari-0.1.0-macos-arm64-adhoc.dmg`（通常版）
  - `ikari-0.1.0-macos-arm64-adhoc-offline.dmg`（オフライン版）
  - `SHA256SUMS`（上の dmg 2 つの行だけ）
- [ ] 一時作業ディレクトリを作り、下書き Release の全資産をダウンロードして次を確かめる（作業後に一時ディレクトリは消す）。

  ```sh
  # 作業用の一時ディレクトリで実行（リポジトリの外なので -R を付ける）
  gh release download v0.1.0 -R ikari-zen/ikari-ime
  shasum -a 256 -c SHA256SUMS
  gh attestation verify ikari-0.1.0-macos-arm64-adhoc.dmg -R ikari-zen/ikari-ime
  gh attestation verify ikari-0.1.0-macos-arm64-adhoc-offline.dmg -R ikari-zen/ikari-ime
  ```

  （注: 証明の対象は dmg で、`SHA256SUMS` 自体ではない。下書きの段階での `gh release download` と `gh attestation verify` の実際の動きは**未確認**。）
- [ ] 下書きのリリースノートの本文（workflow が入れた仮の案内文）を、**本来のリリースノート**へ書き換える。次を必ず含める:
  - 変更の概要（[CHANGELOG.md](../CHANGELOG.md) と整合）
  - 同梱した辞書・モデルの一覧（名前、元データの版、モデルの版、ライセンス、出典、前の版から元データを上げたか。ADR 0080 §4 の 4・§8 の 3）
  - 3 つの証拠の確かめ方への案内（[README.md](../README.md)・[antivirus-false-positive.md](antivirus-false-positive.md)）
  - セキュリティの修正を含む版では、先頭にその旨とログアウト等の案内（ADR 0070 §4 の 1・4）

---

### 4.6 作者の鍵での署名（minisign）と下書きへの添付（作者のみ。AI エージェントは実行しない）

3 つ目の証拠（作者の手元の鍵による `SHA256SUMS` の署名。ADR 0069 §3・§5）を付ける。**秘密鍵は作者の手元だけに置き、AI エージェントや CI には渡さない。**

- [ ] §4.5 でダウンロードしてハッシュと証明を照合した `SHA256SUMS` に対して、**作者が**手元で minisign 署名を作り、`SHA256SUMS.minisig` を生成する（コマンド細部と鍵パスは鍵作成後に確定するため**未確認**）。

  ```sh
  minisign -Sm SHA256SUMS -s <作者の秘密鍵ファイル>
  ```

- [ ] 添付の前に、手元で公開鍵による検証が通ることを確かめる。

  ```sh
  minisign -Vm SHA256SUMS -p <公開鍵ファイル>
  ```

- [ ] **公開前の下書き Release に** `SHA256SUMS.minisig` を添付する（Web UI の下書き編集画面、または `gh release upload v0.1.0 SHA256SUMS.minisig -R ikari-zen/ikari-ime`）。
  - **重要（immutable releases）**: 一度公開すると資産を追加・変更・削除できない。必ず公開前に `SHA256SUMS.minisig` が下書きに付いていることを目視確認する。

---

### 4.7 下書き Release の公開（作者のみ。AI エージェントは実行しない）

- [ ] 公開前の最終確認（作者）:
  1. 下書きの資産が、dmg 2 つ（通常版・オフライン版）、`SHA256SUMS`、`SHA256SUMS.minisig` の 4 つであること。
  2. リリースノート本文が仮の案内文のまま残っておらず、同梱データ一覧を含む本来の文面になっていること。
  3. 公開後は immutable releases により資産とタグを変更・削除できないことを了解していること。
- [ ] **作者が**下書き Release を **`Latest` を明示して**公開する（ADR 0076 §18.2）。
  - GitHub Web UI: 下書き編集画面で **「Set as the latest release」** にチェックが入っていることを確かめて **「Publish release」** を押す。
  - CLI を使う場合:

    ```sh
    gh release edit v0.1.0 -R ikari-zen/ikari-ime --draft=false --latest
    ```

  - 注意: workflow の `gh release create` は `--latest` を指定していない。下書きの段階で `Latest` に設定できるかは**未確認**なので、公開の操作で必ず明示し、公開のあとに §4.8 で確かめる。
  - データ用の Release（`webwords-*`、`abr-mirror-*` など）は `Latest` にしない運用（ADR 0076 §18.2。作るときは `--latest=false`）。2026-11-26 の `gh release list` では `abr-mirror-20261025` が `Latest` になっている（公開済みのアプリの版がまだ無いため）。

---

### 4.8 公開のあとの確認（読み取り確認は AI エージェント可／対応判断は作者）

- [ ] ログインしていないブラウザで `https://github.com/ikari-zen/ikari-ime/releases/tag/v0.1.0` を開き、タイトル・本文・全資産（dmg、`SHA256SUMS`、`SHA256SUMS.minisig`）が正しく表示されることを確かめる。
- [ ] 「最新の Release」が `v0.1.0`（`v<数>.<数>.<数>`）であることを確かめる（ADR 0076 §18.2）。

  ```sh
  gh release view -R ikari-zen/ikari-ime --json tagName   # タグを指定しないと、Latest の Release が出る
  ```

- [ ] 公開済み Release から資産を別の一時ディレクトリへ取り直し、利用者の手順どおり 3 つの証拠すべてが通ることを確かめる（実走**未確認**）。

  ```sh
  gh release download v0.1.0 -R ikari-zen/ikari-ime
  shasum -a 256 -c SHA256SUMS
  gh attestation verify <ダウンロードした dmg> -R ikari-zen/ikari-ime
  minisign -Vm SHA256SUMS -p <公開鍵ファイル>
  ```

- 通常版の設定画面の「新しい版を確認」の確認は、0.1.0 では対象外（macOS の設定アプリに、このボタンがまだ無い。2026-11-26 ユーザー決定）。ボタンを足す版で確かめる。
- [ ] 署名・検証に使った一時作業ディレクトリを消す。

---

### 4.9 0.2.0（Windows 版を含む配布）の公開前に追加で要る項目

以下は Windows 版を配り始める 0.2.0 向けの項目（macOS だけの 0.1.x の公開前には不要）。§4.1〜§4.8 の資産の一覧とコマンドも、0.2.0 の前に Windows の分を足して直す。

- [ ] 0.2.0 のタグを打つ**前**に、Windows の成果物の残り（#585: オフライン版）が済んでいること（辞書とモデルの同梱は #663 で入った）。今の `release.yml` は、タグの版が 0.2.0 以上なら `include_windows` が `true` になり、`build-windows` の出力（`ikari-ime-<版>-windows-x64-setup.exe`。今は通常版だけで、辞書・モデルを含み、VC++ 再頒布パッケージは含まない。2026-11-26 の試しのビルド run `45752380961` でインストーラは約 499MB（498,592,550 バイト））を配る物に入れる。
- [ ] Windows のオフライン版を足すときは、`provenance` の取り込みと期待するファイルの一覧、`draft-release` の期待するファイルの一覧（`files`）の両方を揃えて直すこと（#585）。
- [ ] Windows の実機確認（[windows-real-device-check.md](windows-real-device-check.md) の該当項目）と、タグの前にリリースの workflow の試しのビルド（`workflow_dispatch`）で作った Windows インストーラの VM 確認（同 項目 47〜55）が済んでいること。
- [x] インストーラの失敗時の手動の復旧手順（[windows-user-guide.md](windows-user-guide.md) の「インストールに失敗したとき」）が書かれ、VM で確かめられていること（実機確認の項目 79〜81。2026-11-23 に確認。画面の表示などの未確認は項目に書いた）。
- [ ] インストーラの失敗時の自動の巻き戻しを作るか、ADR で決めたこと（#646）。
- [ ] 通常版とオフライン版の 2 OS × 2 版（4 成果物。[ADR 0080](adr/0080-bundle-model-and-dictionary.md) §6・§7・§9。Windows の VC++ 再頒布パッケージとオフライン版は未完了。実データを同梱したインストーラを VM に入れて変換する確認も**未確認**）が揃っており、オフライン版に通信の部品（`ikari-data`、通信ライブラリ、`network.*` の entitlement、取得の印の文字列）が無いことを CI の検査とネットワークを切った VM で確かめたこと。
- [ ] ウイルス対策ソフトの検査: 配る物（未署名）を、Windows の VM の Microsoft Defender（SmartScreen を含む）で検査し、検出や警告が出ないことを確かめる。出たら、結果を記録し、配るか決める。作者の環境の ESET では検知されなかった（2026-11-17。製品と版が限られた 1 件の参考値で、保証ではない）。守りを切る手順は書かない。
