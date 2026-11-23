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
- [ ] 取得して使うモデル（zenz）の更新を確かめること。Hugging Face の API でコミット履歴とファイルの sha256 を見て、`scripts/lib.sh` に固定した sha256 と合っていること（`curl -s https://huggingface.co/api/models/Miwa-Keita/zenz-v3.2-small-gguf/commits/main`、`…/zenz-v3.2-small-gguf?blobs=true` の `lfs.sha256`）。ファイルが変わっていたら、ADR 0058 の手順（新しい sha256 を評価で確かめてから表を更新）に従う。README だけの更新は影響なし（2026-11-16 に確認: 最新の更新 2026-11-07 は README のみ。モデルのファイルは 2026-05-06 のまま、sha256 先頭 29c223d4c233…）。新しい版（zenz-v3.2 より新しいもの、xsmall など）の有無も、作者の一覧（`https://huggingface.co/api/models?author=Miwa-Keita`）で見る（#440）。
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

## 4. ビルド済みの配布を作るとき（今回はしない）

設計は [security-design.md](security-design.md)（[ADR 0069](adr/0069-distribution-authenticity.md)、[0070](adr/0070-update-and-vulnerability-response.md)）。タグを打つことと Release を公開することは、**作者だけが行う**（エージェントはしない）。

- [ ] #241（未署名のアプリを開く手順と、macOS の置き場所の実機確認）が済んでいること。Windows は [windows-real-device-check.md](windows-real-device-check.md) の該当の項目が済んでいること。
- [ ] 前の節（§3）の、Private vulnerability reporting が有効であること。
- [ ] Release を作れるアカウント（作者）に、2 要素認証（パスキーか物理キー）を設定したこと。SMS や認証アプリだけにしない（ADR 0076 §18.2。アカウントを取られると、偽の版を「新しい版」として勧めうるため）。
- [ ] タグを守る ruleset を入れたこと（Settings → Rules → Rulesets、対象はタグ）。対象は `v*`（アプリ）と `webwords-*`（データ）。作れる者を絞り、付け替え（force push）と削除を禁じる。
- [ ] Releases を immutable（公開したら資産とタグを変えられない）にしたこと。**要確認**: GitHub 側にこの機能があるか、どこで設定するかは、まだ確かめていない。無ければ、無いことをここに書く。
- [ ] 「最新の Release」の扱い（ADR 0076 §18.2）: アプリの Release は `gh release create … --latest` で明示して最新にする。データの Release（`webwords-*`）は `--latest=false` で作り、最新にしない。公開のあと、`gh release view --json tagName` の最新のタグが `v<数>.<数>.<数>` であることを確かめる。
- [ ] [security-design.md](security-design.md) §5.2 の「版を出す前」の検査が済んでいること（製品のビルドであること、インストーラの確認、送信の記録、他社の部品の版、3 つの証拠）。
- [ ] Windows のインストーラを、タグの前にリリースの workflow の試しのビルド（`workflow_dispatch`。証明と Release は作らない）で作り、VM で入れて確かめたこと（[windows-real-device-check.md](windows-real-device-check.md) の項目 47〜55）。
- [ ] 配る物は、GitHub のランナーで、タグから作ったものだけ（作者や利用者の機械で作ったものは配らない）。Release は下書きにし、作者が中身を見てから公開する。
- [ ] Release に 3 つの証拠が付いていること: `SHA256SUMS`、GitHub の成果物の証明（`gh attestation verify <ファイル> -R ikari-zen/ikari-ime` で確かめられる）、作者の鍵による `SHA256SUMS` の署名。
- [ ] 通常版とオフライン版の 2 OS × 2 版（4 成果物。[ADR 0080](adr/0080-bundle-model-and-dictionary.md) §7・§9。リリースの workflow への組み込み・dmg・成果物の確定名はレーン 4〜6 未実装のため**未定**）が揃っており、オフライン版に通信の部品（`ikari-data`、通信ライブラリ、`network.*` の entitlement、取得の印の文字列）が無いことを CI の検査とネットワークを切った VM で確かめたこと。
- [ ] Release のノートに、同梱した辞書・モデルの一覧（名前、元データの版、モデルの版、ライセンス、出典、前の版から元データを上げたか）を載せたこと（ADR 0080 §4 の 4・§8 の 3）。
- [ ] README の冒頭（「ビルド済みのアプリは、まだ配っていません」）と SECURITY.md の「配布」の節の同じ行を、配布に合わせて書き直す。README には、配布元が Releases だけであること、sha256 の比べ方、証明と署名の確かめ方、OS の警告の案内（実機で確かめた手順だけ。守りを切る手順は書かない）、Watch の案内を書く。
- [ ] 配布物と SECURITY.md の「約束すること」が合っていること（通信しない、秘密の欄、ログ）。
- [ ] ウイルス対策ソフトの検査: 配る物（未署名）を、Windows の VM の Microsoft Defender（SmartScreen を含む）で検査し、検出や警告が出ないことを確かめる。出たら、結果を記録し、配るか決める。作者の環境の ESET では検知されなかった（2026-11-17。製品と版が限られた 1 件の参考値で、保証ではない）。守りを切る手順は書かない。
- [x] 誤検出の報告の窓口: 公開後にウイルス対策ソフトの誤検出の報告が来たときの対応（各社への誤検出の報告）を、SECURITY.md か README に 1 行書く。（[SECURITY.md](../SECURITY.md) の「配布」に 1 行追記し、[antivirus-false-positive.md](antivirus-false-positive.md) に案内・確かめ方・Issue 窓口を下書き済み。公開鍵のありか等の未確定事項は未定）
- [ ] [CHANGELOG.md](../CHANGELOG.md) の 0.1.0 のリリース日を入れる。
- [ ] 配布物に含める依存と同梱データのライセンス表示を確かめる（`cargo xtask licenses --check`、[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)、および同梱データのそばの `NOTICE.txt`・`licenses/`。元ファイルは [dist-notices/NOTICE.txt](dist-notices/NOTICE.txt)、パッケージへの組み込み詳細はレーン 4〜6 未実装のため**未定**）。
- [ ] 作者が、下書きの Release を確かめて公開する。
