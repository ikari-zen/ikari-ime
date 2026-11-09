# 現在の構成（地図）

ikari-ime の今の構成を 1 か所にまとめた地図。判断の理由は書かない（ADR へのリンクにする）。
構成を変えたコミットで一緒に直すこと。要件は [spec](ikari-ime_spec.md)、日付つきの判断の記録は [ADR の索引](adr/README.md)。

状態: フェーズ3a まで完了（2026-10-12）。macOS の入力メニュー、設定アプリ「ikari 設定」、ユーザー辞書と学習の管理（[ADR 0005](adr/0005-macos-settings-and-indicator.md)）。独自のモード表示は持たない（ADR 0005 §15.2）。英語のモード（打鍵を素通しにする）は B44 で足した（[ADR 0026](adr/0026-english-mode-and-key-stats.md)。下の「入力モードと入力ソース」）。

## 1. crate（Rust、workspace）

| crate | 責務 | 主な決定 |
| --- | --- | --- |
| `dict-format` | バイナリ辞書の形式（`grammar.bin`、`system.dict`、`vectors.bin`）の定義、検証、読みの正規化。IO を持たない | [0001 §3](adr/0001-phase0-architecture.md)、[0002 §3](adr/0002-phase1-architecture.md) |
| `resource` | リソース（辞書・表・モデル）の束の契約（`ikari-resource`）。manifest の型と読み書き、束の検証（大きさ、sha256、形式の版、既知のモデル、古い版、パスの脱出）、既知のモデルの sha256 の表、入れ替えの journal の型。IO は束の読み取りだけ。まだエンジン・xtask からは使っていない（#246-2 で配線） | [0058 §4〜§7](adr/0058-offline-resource-install.md) |
| `dict-builder` | SudachiDict の CSV と chiVe から辞書を作る。ポリシー（`policy/*.tsv`）でコストを調整する。異字同訓の表（`policy/kun.tsv`）で動詞・形容詞の表記を並べる。`town` サブコマンドが ABR の町字マスターから地名の語彙（`town_lex.csv`）と、住所の連なり（都道府県 + 郡 + 市区 + 町名。`chain_lex.csv`。同じ行の Core にも勝つ。[0030 §12](adr/0030-abr-town-vocabulary.md)）を作る | [0001 §8](adr/0001-phase0-architecture.md)、[0002 §4](adr/0002-phase1-architecture.md)、[0009](adr/0009-verb-notation.md)、[0030](adr/0030-abr-town-vocabulary.md) |
| `input` | ローマ字、JIS かな、字種変換。辞書に依存しない純粋な処理（殻がリンクする） | [0004 §3.3](adr/0004-phase2-macos.md) |
| `core` | 辞書引き、ラティスと Viterbi、Ranker（cost、左文脈（連接と複合語。[0007](adr/0007-left-context-compound.md)）、文書内キャッシュ、かなの表記、学習、プロファイル、話題）と Guard、`Converter`、文節の編集と再変換（`Segmented`、`Converter::segment` / `resize` / `reconvert`。区切りの規則は `pos::{ClauseWord, clause_starts, joins_following_noun}`、解析は `ContextAnalyzer::analyze_text`、読みの逆引きは `Dictionary::warm_reading_index`、辞書のページの先読みは `Dictionary::prefault`。[契約 §6](phase2-contract.md)）、LM が規則 S の守った1位を覆したら戻す段（`CompositeRanker` の最後の段。判定は `lattice::has_noun_suru_edge`。[0024](adr/0024-lm-keeps-rule-s.md)）、英字が1位のときのカタカナの繰り上げ（`ranker/english_first.rs` が置き換えた候補を作り、`CompositeRanker` の最後の段が1位にする。置き換えた候補の追加は `Conversion::build_first_pool`。[0020](adr/0020-english-spelling-first-candidate.md)）、読みそのままの平仮名・片仮名の救済（最初のプールを rank したあと、30 位より後ろなら 30 位へ。`conversion::rescue_reading_kana`、文節のモードは `pin_first` のあとにも。`convert.kana_rescue_rank`、0 で無効。[0052](adr/0052-reading-kana-rescue.md)）、ユーザーデータ（`user/`） | [0001 §4](adr/0001-phase0-architecture.md)、[0002](adr/0002-phase1-architecture.md)、[0003](adr/0003-topic-ranker-decision.md)、[0020](adr/0020-english-spelling-first-candidate.md)、[0024](adr/0024-lm-keeps-rule-s.md) |
| `lm` | zenz（GGUF、llama.cpp）による話題スコア。feature `llama` / `metal` のときだけ llama.cpp をビルドする | [0002 §13.2](adr/0002-phase1-architecture.md)、[0003](adr/0003-topic-ranker-decision.md) |
| `stack` | Converter、Ranker、LM の共有の組み立て（`build_stack`）。エンジン、CLI、eval が使う | [0004 §3.2](adr/0004-phase2-macos.md) |
| `import` | 取り込みファイル（JSONL）の検証、強制アライメント、初期学習 | [0002 §11](adr/0002-phase1-architecture.md) |
| `proto` | エンジンと殻の間のメッセージ（`proto/ikari/engine/v1/engine.proto`、prost）。設定アプリと殻の間の `proto/ikari/bridge/v1/bridge.proto` も生成する（`shell-core` の bridge が使う。Swift は `gen-proto.sh`） | [0004 §5](adr/0004-phase2-macos.md)、[0005 §6.3](adr/0005-macos-settings-and-indicator.md) |
| `engine` | 常駐エンジン。`Envelope` を受けて返す。`lib.rs`（`Engine`、`Inner`、上限）、`dispatch`（入口）、`session`、`convert`、`lm`、`data`（データの監視）、`workers`（裏のスレッド）、`segments`（文節）、`admin`（管理）、`cache` | [0004 §7](adr/0004-phase2-macos.md) |
| `engine-ffi` | エンジンの C ABI（staticlib。ヘッダ `include/ikari_engine.h`） | [0004 §4.2](adr/0004-phase2-macos.md) |
| `shell-core` | 殻の OS に依存しない処理の全部（入力の状態機械、キー割り当て、合成器の呼び出し、英語のモードの文脈（`english`）、郵便番号と Unicode の表記（`postal`）、ショートコード、候補ウィンドウの配置（`panel`）、メニュー、エンジンとの通信と再接続（`engine`）、設定アプリとのブリッジの判断（`bridge`）、設定の検証と整形（`settings`）、診断ログ）。入口は「OS のイベント → `Shell::dispatch` → 動作の列」の 1 本。共通のシナリオ（`shell-core/scenarios/`）で振る舞いを守る | [0041](adr/0041-shell-state-machine-in-rust.md) |
| `shell-ffi` | 殻と設定アプリが使う `shell-core` の C ABI（staticlib。ヘッダ `include/ikari_shell.h`。`ikari_shell_*`、`ikari_settings_*`、`ikari_panel_*`。入出力は protobuf の `ikari.shell.v1`・`ikari.settings.v1`） | [0041 §4.4](adr/0041-shell-state-machine-in-rust.md) |
| `cli` | コマンド `ikari`（変換、ユーザー辞書、取り込み）。バイナリ名は `ikari` | [0001 §5](adr/0001-phase0-architecture.md)、[0002 §14](adr/0002-phase1-architecture.md) |
| `eval` | 評価セットの実行、レポート、シナリオ、AJIMEE。`ikari-eval` | [0001 §6](adr/0001-phase0-architecture.md)、[0002 §15](adr/0002-phase1-architecture.md) |

### 依存の向き（矢印は「依存する」。循環なし）

```
dict-format ← dict-builder
dict-format ← resource
dict-format, input ← core ← lm（optional: llama-cpp-2）
core, lm（optional）← stack ← engine ← engine-ffi（staticlib）
core, dict-format ← import
core, stack, import（+ lm optional）← cli, eval
input, proto ← shell-core ← shell-ffi（staticlib）
proto ← engine
```

- `lm` と `engine-ffi` は feature なしなら llama.cpp をビルドしないので、`cargo build` / `cargo test` は workspace 全体を対象にする。
- LM を有効にする feature は `lm`（Metal なし）と `lm-metal`（`cli`、`eval`、`stack`、`engine`、`engine-ffi`）。`lm` crate の中は `llama` / `metal`。
- LM のモデルの種類は設定キー `zenz.model_kind` で選ぶ（既定は zenz。kanakanji-edge は比較用に残してある。[0003 §7.1](adr/0003-topic-ranker-decision.md)）。

## 2. macOS のプロセスの構成

```
ikari.app（殻。Swift、InputMethodKit、Sandbox なし。~/Library/Input Methods/）
  ├─ IkariIME        IMKServer、IkariInputController、候補ウィンドウ、ブリッジの待ち受け。判断は持たず、イベントを
  │                  shell-core に渡して動作の列を実行する（ADR 0041）
  ├─ IkariShellABI   shell-ffi（`ikari_shell_*`、`ikari_panel_*`、`ikari_settings_*`）の薄い層と、キーコードの表（`KeyEvents`）
  ├─ IkariXPC        エンジンへの XPC の輸送（`XPCEngineConnections`。shell-core のホストの口）、識別子、署名要件。
  │                  smoke が使う型付きの口（`XPCEngineClient`）
  ├─ IkariBridge     設定アプリとのブリッジの輸送（枠の読み書き、BridgeTransport）。照合と中継の許可リストは shell-core
  └─ IkariProto      生成した protobuf（engine、bridge、shell、settings の .pb.swift。macos/scripts/gen-proto.sh）
        │ XPC（Envelope。署名要件で相互に相手を確かめる）
IkariEngine.xpc（同梱の XPC サービス。App Sandbox、ネットワークなし）
  └─ IkariEngineService（Swift は FFI を呼ぶだけ）→ engine-ffi（Rust のエンジン。LM は llama.cpp）
```

- `IkariSettings`（設定アプリ「ikari 設定」。`ikari.app/Contents/Helpers/` に同梱）は、殻の Unix ソケット（`IkariBridge`）だけにつなぐ。検証・整形・変更の組み立ては shell-core（`ikari_settings_app`）に頼み、Swift は画面と小さな画面の型（`ScreenTypes.swift`）だけを持つ。
- 移行の前の Swift の状態機械（`IkariShellCore`）とその測り手（`IkariShellBench`）、旧い FFI（`ikari_composer_*`、`ikari_convert_script`）は、移行の片付け（M5、#138）で消した。
- `ikari-smoke`（`IkariSmoke`）: バンドルの中から XPC 経由で E2E を流す。`macos/scripts/smoke.sh`。
- 識別子と署名要件は [phase2-contract.md](phase2-contract.md)。Bundle ID `io.github.ikari-zen.inputmethod.ikari`、チームID（`lib.sh` の `TEAM_ID`）。
- 設計は [0004](adr/0004-phase2-macos.md)。設定アプリ（IkariSettings）と管理のメッセージは [0005](adr/0005-macos-settings-and-indicator.md)（実装済み。ブリッジと上限は [phase2-contract.md §5](phase2-contract.md)）。
- ビルドとインストールの手順は [README](../README.md#5-インストールソースからのビルド)、バンドルの内部は [macos/README.md](../macos/README.md)。

### 入力モードと入力ソース（ADR 0026 §5、B44）
- ikari は入力ソースを 2 つ持つ（`ikari.Info.plist` の `ComponentInputModeDict`）。**日本語**（辞書のキーと `TISInputSourceID` が `io.github.ikari-zen.inputmethod.ikari.Japanese`。`tsInputModeJISKeyboardShortcutKey = 1` = かなキー）と、**英語**（辞書のキーが macOS の標準の `com.apple.inputmethod.Roman`、`TISInputSourceID` が `io.github.ikari-zen.inputmethod.ikari.English`、`JISKeyboardShortcutKey = 3` = 英数キー、`tsInputModeDefaultStateKey = true`）。英数キーが ikari の英語に行くのは、この標準のキーによる（S-A の T1）。ABC は残す。
- 表示名（`Resources/{ja,en}.lproj/InfoPlist.strings`）は、`TISInputSourceID` ではなく**モードの辞書のキー**で引かれる。英語は `ikari (English)`。アイコンは `ikari-japanese.pdf`（塗りの「あ」）と `ikari-english.pdf`（輪郭の「A」）。
- `IkariInputController` は `setValue(_:forTag:client:)`（`kTextServiceInputModePropertyTag`）で今のモード（`InputMode`）を知る。コントローラ、セッション ID は変えず、`BeginSession` も送り直さない。日本語 → 英語のときは合成中の文字列を今の表示のまま確定し、候補ウィンドウを閉じる。
- **英語のモードは打鍵をすべてアプリに素通しにする**（`handle` が false）。英数・かなキー（keyCode 102・104）はどのモードでも受け止める（素通しにするとアプリに余計な文字が入る）。secure なら、日本語でも英語でも素通しで、何も記録しない（日本語は合成中なら今の表示のまま確定してから）。キー割り当ての `mode.toggle` / `mode.english` / `mode.japanese`（既定は割り当てなし）は、どちらのモードでも変換より先に見る。
- 英語の文脈は、殻が `EnglishContextPool`（`shell-core/src/english/`）にためる。印字できる文字を足し、Backspace は末尾を消し、区切り（Return、Tab、矢印、Escape、F キー、Ctrl・Option・Command 付き、モードの切り替え、`deactivateServer`、2000 スカラー）で 1 件の `CommitText`（`reading` は空）として片道でエンジンに送る。エンジンは確定履歴に入れるだけで、学習しない。`reading` が空の項目は、履歴の読みを使う再変換（`core/src/converter/reconvert.rs`）が飛ばす。
- ショートコード（ADR 0027）の英語のモードへの配線は B48、打鍵と確定文字列の統計は B45。
- 英語のモードの郵便番号（ADR 0032 §9.1、設定 `postal.english`、既定 off）: 語頭の `te` + 数字 7 桁を殻の `PostalMachine`（`shell-core/src/postal/`）がためて、7 桁目で候補ウィンドウを自動で出す（下線なし。選ぶのは ↑/↓ だけ）。住所は、殻が主スレッドの外で `PostalLookupRequest` をエンジンへ 1 回送り（shell-core の `engine`。待ちの上限は暫定で 1 秒）、エンジンが `postal.tsv` の表（`stack().postal`）を引いて返す。返事が来たとき状態が同じ（同じコードと同じ候補の通し番号）なら候補に足し、違えば捨てる。設定アプリからは中継しない（shell-core の `bridge/relay_policy.rs`）。同じ状態機械が、`u+` + 16 進 4〜6 桁（設定 `unicode.notation`、既定 off。issue #28）も扱う。4 桁目で候補（そのまま、その文字）が出て、桁が増えるたびに更新する。問い合わせはなく、10FFFF を超える並び・制御文字・未割り当ては出さない。`u` の次は `+` が要る（`use`、`unit` などは素通し）。
- 確信度が高い変換（#69、設定 `candidate.hide_when_confident`、既定 on。2026-10-28 ユーザー決定）: エンジンが、読み全体の候補の 1 位と 2 位の最終スコアの差を見て、閾値以上なら `ConvertReply.confident = true` を返す（閾値は LM なし 5000、LM あり 10000。暫定。`convert` の行の `gap=` `confident=` で実利用の分布を見て見直す）。殻は、設定がオンで confident、1 文節のとき、最初の Space で 1 位だけを下線つきで出し、候補ウィンドウは出さない。もう一度 Space、↓、↑、数字キーで一覧を出す（選択は 0 番のまま）。Enter、Esc、BackSpace、文字入力は今までどおり。

## 3. データの置き場所

| 場所 | 中身 | 作る人 |
| --- | --- | --- |
| `data/raw/`（gitignore） | SudachiDict、chiVe、zenz のモデル、AJIMEE、kanakanji-edge、emoji-data（`emoji-data/emoji.json`。コミット固定、[0027 §5](adr/0027-emoji-input.md)）、ABR の町字マスター（`abr/mt_town_all.csv`。任意。[0030](adr/0030-abr-town-vocabulary.md)） | `scripts/fetch-data.sh` |
| `data/build/dict/`（gitignore） | `grammar.bin`、`system.dict`、`vectors.bin`、`BUILD_INFO`、`emoji.tsv`（emoji.json があれば。1,975 名・1,914 絵文字。辞書の入力・BUILD_INFO には含めない）。旧版は `data/build/dict.prev/`。ABR があれば、地名の語彙の中間ファイル `data/build/town_lex.csv` も作る（[0030](adr/0030-abr-town-vocabulary.md)） | `scripts/build-dict.sh` |
| `~/Library/Application Support/ikari-ime/dict/`、`models/` | エンジンが読む辞書とモデル（上の 2 つのコピー） | `scripts/install-data.sh` |
| 同 `learning.tsv`、`profile.tsv`、`user_dict.tsv`、`ingest_*.tsv` | ユーザーデータ（学習、属性プロファイル、ユーザー辞書、取り込み）。`IKARI_USER_DIR` で場所を変えられる | エンジンと `ikari` CLI（[0002 §14](adr/0002-phase1-architecture.md)） |
| `core/assets/packs/it.tsv` | IT の語彙集（読みのカタカナが1位、英字が2位）。`ikari-core` にコンパイルして埋め込む（配置は要らない）。`DictionarySet` の slot は システム辞書、語彙集、ユーザー辞書 の順。`settings.tsv` の `dict.pack_it`（既定 `on`）で切れる（[0012 §4](adr/0012-new-word-dictionary.md)） | 人（Claude の下書きをユーザーが確認） |
| `target/macos/<release\|debug>-<team\|adhoc>/ikari.app` | ビルドしたバンドル | `macos/scripts/build.sh` |
| `testdata/sudachi-mini/` | テスト用の小さな辞書の fixture（リポジトリに入れてある） | — |
| `eval/sets/`、`eval/reports/` | 評価セットと結果のレポート（[索引](../eval/reports/README.md)） | 人とエージェント |

### エンジンのログ（`logs/engine.log`）と Space の待ち（[ADR 0021](adr/0021-lm-lookahead.md) L0・L1）
- 1 行は `時刻ms 種類 所要us lm=<状態> err=<種類> [注記]`。注記は固定の語の `key=value` だけ（読み・候補・文脈は書かない。NFR-19）。
- 書き込みは別スレッド `ikari-log`（有界のキュー 8192 行、64KB のバッファ、1 秒ごとに flush）。`Engine::handle` はキューに入れるだけでディスクに触らない。満杯なら捨てて、次に書けたときに `log_dropped n=N` の行を書く（[ADR 0034](adr/0034-diagnostic-log.md) §3.1）。
- 4MB を超えるか、ローカルの日付が変わったら回し、`logs/engine-YYYYMMDD-HHMMSS.log.gz` にする。180 日か合計 100MB のどちらか先で古い順に消す（同 §3.2）。読むときは `cd ~/Library/Application\ Support/ikari-ime/logs && gzip -dc engine-*.log.gz | cat - engine.log | less`（名前の順が時刻の順。macOS の `zcat` は `.gz` を読まないので `gzip -dc` か `gzcat`）。
- 変換の行を結ぶ番号（番号だけで文字ではない）: `convert` と `reconvert` に `session=<番号> conv=<変換の番号>`、`page` `segment_page` `resize_segment` `commit` `commit_segments` `cancel` に `conv=`。`sh_episode` も同じ `session=` を持つので、「どの変換で LM が効かなかったか」「LM なしの変換のまま確定したか」を、時刻の順と番号で結べる。
- `convert` の注記: `segments=N`、`cache=hit|wait_hit|wait_miss|miss`、`lm_applied=1|0`（返事の `lm_applied`。3 つ目の項目の `lm=<状態>` とは別のキー）、外れ（`cache=miss`）なら `miss=reading|context|none`（同じセッションのキャッシュに、読みだけが違うもの／文脈の番号だけが違うものがある／近いものがない）。秘密の入力は `cache=miss miss=none`。続けて `gap=<1 位と 2 位のスコアの差>`（候補が 1 件以下なら `na`）と `confident=1|0`（#69）。続けて `rlen=<読みの文字数>`（secure は書かない）と `qlen=<変換の時点で終わっていない先行計算の数（待ち + 処理中）>`（#159。数だけで文字は出さない）。
- `prefetch_done` の注記: `lm_applied=1|0`（LM が間に合って使われたか）、`lm_calls=N`（その先行計算での LM 呼び出し回数。2 文節以上は 2）。
- 先行計算の左文脈: 合成の最初の `contextRefreshKeystrokes` 打鍵は左文脈を読み直す（#1。変換中に次の文字を打つと、1 打鍵目の文脈は直前の確定がアプリに入る前に読まれるため）。文脈の番号（`contextRev`）が進んだら、その文脈で先行計算を送り直す。それより後の打鍵は読み直さず、Space で読み直す。
- Space（`Convert`）で先行計算が計算中なら、**Space から最大 30ms**（既定。`settings.tsv` の `engine.convert_wait_ms`、0〜200。`Engine::open` のときだけ読む。NFR-04）待つ。先行計算が LM を待てる締め切り（`LmBudget.max`）も既定 300ms で、`engine.lm_deadline_ms`（50〜2000）で変えられる。範囲は暫定。間に合わなければ LM なしでその場で変換する。以前の「Prefetch の受付＋`lm_budget`」は廃止。再変換・区切り直しは待たない（変更なし）。

## 4. スクリプト

| スクリプト | 役割 |
| --- | --- |
| `scripts/setup.sh` | 取得 → 辞書のビルド → データの配置 → アプリのビルドとインストール → smoke を通す |
| `scripts/fetch-data.sh`、`build-dict.sh`、`install-data.sh` | 上の各段。単独でも使える |
| `scripts/check.sh` | コミットの前に流す門（fmt、clippy、test、`swift test`、文書のリンク。`--lm` `--real` `--perf` `--eval`） |
| `macos/scripts/{build,install,smoke}.sh` | バンドルのビルド、署名、配置、E2E。macOS 固有 |
| `eval/scripts/snapshot.sh` | 振る舞いが変わっていないことの確認（リファクタの前後の比較） |
| `cargo xtask webwords fetch\|build\|measure\|publish` | 外来語の一覧の作り直し（約 3〜4 か月ごと。手順は [webwords-update.md](webwords-update.md)） |

## 5. どの ADR が何を決めたか

| ADR | 決めたこと |
| --- | --- |
| [0001](adr/0001-phase0-architecture.md) | crate の分け方、辞書形式 v1、変換の流れ、CLI と eval の契約 |
| [0002](adr/0002-phase1-architecture.md) | 辞書形式 v2、文書内キャッシュ、文脈つき学習、ユーザー辞書、Rewriter、取り込み、話題スコアの実験 |
| [0003](adr/0003-topic-ranker-decision.md) | 話題スコアと LM の既定構成（zenz small を 1 つ）、GrammarGuard、長文の扱い、基本原則との照合 |
| [0004](adr/0004-phase2-macos.md) | macOS の殻と常駐エンジン、XPC、署名、先行計算、配布 |
| [0005](adr/0005-macos-settings-and-indicator.md) | ON/OFF 表示、入力メニュー、設定アプリ、ユーザー辞書と学習の管理、管理のメッセージ |
| [0026](adr/0026-english-mode-and-key-stats.md) | 英語のモード（英数キーで入る、打鍵は素通し）、モードの切り替え、英語の文脈、入力の統計（B45）、直接入力の Unicode |

- ユーザーの指摘と、それを受けた決定は [user-feedback.md](user-feedback.md) にまとめてある（具体例つき）。
- 今のタスク（ユーザーが要るものと、Claude が進められるもの）は GitHub の issue と Project。
