# ドキュメントの入口

ikari-ime（日本語入力メソッド）の文書の索引です。文書は日本語です。初めて読む人は、上から順に「利用者向け」「開発者向け」のどちらかを選んでください。

用語: 「ADR」は、設計の判断と理由を 1 件ずつ残した記録です。`#番号` は、このリポジトリの GitHub の Issue 番号です。「ユーザー」は、機能を使う人（方針の決定は作者が行うので、作者の決定を「ユーザー決定」と書いた文書もあります）。

## 利用者向け

| 文書 | 内容 |
| --- | --- |
| [../README.md](../README.md) | ikari-ime とは。ソースからのビルド、入力ソースの追加、アンインストール、プライバシー |
| [features.md](features.md) | 機能の一覧（実装済み、一部、予定） |
| [roadmap.md](roadmap.md) | 今後の計画と、公開の方針 |
| [proxy-download.md](proxy-download.md) | プロキシの下で元データを取得する手順 |
| [../SECURITY.md](../SECURITY.md) | 脆弱性の報告の方法 |

## 開発者向け

| 文書 | 内容 |
| --- | --- |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | 貢献の方針（Issue、Pull Request、fork） |
| [../CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) | 行動規範 |
| [architecture.md](architecture.md) | 今の構成の地図（crate、プロセス、データの置き場所、スクリプト） |
| [ikari-ime_spec.md](ikari-ime_spec.md) | 要件定義書。最初に「基本原則」を読むと方針が分かります |
| [adr/README.md](adr/README.md) | 設計の判断の索引（ADR） |
| [user-feedback.md](user-feedback.md) | ユーザーの指摘と、それを受けた決定（具体例つき） |
| [history/README.md](history/README.md) | 検討の経緯（試したこと、やめた案とその理由） |
| [../CLAUDE.md](../CLAUDE.md) | AI エージェント向けの指示。作者の開発の運用も含みます |
| [../eval/reports/README.md](../eval/reports/README.md) | 評価の結果の索引 |
| [../macos/README.md](../macos/README.md) | macOS 版（Swift の殻、設定アプリ）の注意 |
| [webwords-update.md](webwords-update.md) | 外来語の一覧の更新手順（保守者向け） |
| [publish-checklist.md](publish-checklist.md) | リポジトリを公開するときの手順書（保守者向け） |

## 調査と作業の記録（読まなくても構いません）

特定の時点の調査や作業の記録です。今の状態とは違うことがあります。

- [accuracy-breakdown-2026-10-16.md](accuracy-breakdown-2026-10-16.md)、[homophone-analysis-2026-10-17.md](homophone-analysis-2026-10-17.md)、[lm-regression-2026-10-16.md](lm-regression-2026-10-16.md): 変換の精度の調査
- [phase2-contract.md](phase2-contract.md)、[phase3a-checklist.md](phase3a-checklist.md)、[b28-lanes.md](b28-lanes.md)、[refactor-plan-2026-10.md](refactor-plan-2026-10.md): 開発の途中の契約、確認、計画
- [research/](research/steno-survey-2026-10.md): 入力方式の調査
