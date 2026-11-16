# 現在の構成（地図）

ikari-ime の今の構成を 1 か所にまとめた地図。判断の理由は書かない（ADR へのリンクにする）。
構成を変えたコミットで一緒に直すこと。要件は [spec](ikari-ime_spec.md)、日付つきの判断の記録は [ADR の索引](adr/README.md)。

状態（2026-11-14 に直した）: macOS 版は、入力メニュー、設定アプリ「ikari 設定」、ユーザー辞書と学習の管理（[ADR 0005](adr/0005-macos-settings-and-indicator.md)）。独自のモード表示は持たない（ADR 0005 §15.2）。英語のモード（打鍵を素通しにする）は B44 で足した（[ADR 0026](adr/0026-english-mode-and-key-stats.md)。下の「入力モードと入力ソース」）。殻の判断は Rust の `shell-core` に移した（[ADR 0041](adr/0041-shell-state-machine-in-rust.md)）。Windows は `pipe`・`windows/engine-host`・`windows/tip`（TIP の DLL）まで作ってあり、実機（VM）で確かめている途中（下の 2.1。候補窓と設定画面はまだ）。

## 1. crate（Rust、workspace）

| crate | 責務 | 主な決定 |
| --- | --- | --- |
| `dict-format` | バイナリ辞書の形式（`grammar.bin`、`system.dict`、`vectors.bin`）の定義、検証、読みの正規化。IO を持たない | [0001 §3](adr/0001-phase0-architecture.md)、[0002 §3](adr/0002-phase1-architecture.md) |
| `resource` | リソース（辞書・表・モデル）の束の契約（`ikari-resource`）。manifest の型と読み書き、束の検証（大きさ、sha256、形式の版、既知のモデル、古い版、パスの脱出）、既知のモデルの sha256 の表、入れ替えの journal の型。IO は束の読み取りだけ。エンジン（`model_check`、`lib.rs`）が使う。xtask からはまだ使っていない | [0058 §4〜§7](adr/0058-offline-resource-install.md) |
| `dict-builder` | SudachiDict の CSV と chiVe から辞書を作る。ポリシー（`policy/*.tsv`）でコストを調整する。異字同訓の表（`policy/kun.tsv`）で動詞・形容詞の表記を並べる。`town` サブコマンドが ABR の町字マスターから地名の語彙（`town_lex.csv`）と、住所の連なり（都道府県 + 郡 + 市区 + 町名。`chain_lex.csv`。同じ行の Core にも勝つ。[0030 §12](adr/0030-abr-town-vocabulary.md)）を作る | [0001 §8](adr/0001-phase0-architecture.md)、[0002 §4](adr/0002-phase1-architecture.md)、[0009](adr/0009-verb-notation.md)、[0030](adr/0030-abr-town-vocabulary.md) |
| `input` | ローマ字、JIS かな、字種変換。辞書に依存しない純粋な処理（殻がリンクする） | [0004 §3.3](adr/0004-phase2-macos.md) |
| `core` | 辞書引き、ラティスと Viterbi、Ranker（cost、左文脈（連接と複合語。[0007](adr/0007-left-context-compound.md)）、文書内キャッシュ、かなの表記、学習、プロファイル、話題）と Guard、`Converter`、文節の編集と再変換（`Segmented`、`Converter::segment` / `resize` / `reconvert`。区切りの規則は `pos::{ClauseWord, clause_starts, joins_following_noun}`、解析は `ContextAnalyzer::analyze_text`、読みの逆引きは `Dictionary::warm_reading_index`、辞書のページの先読みは `Dictionary::prefault`。[契約 §6](phase2-contract.md)）、LM が規則 S の守った1位を覆したら戻す段（`CompositeRanker` の最後の段。判定は `lattice::has_noun_suru_edge`。[0024](adr/0024-lm-keeps-rule-s.md)）、英字が1位のときのカタカナの繰り上げ（`ranker/english_first.rs` が置き換えた候補を作り、`CompositeRanker` の最後の段が1位にする。置き換えた候補の追加は `Conversion::build_first_pool`。[0020](adr/0020-english-spelling-first-candidate.md)）、読みそのままの平仮名・片仮名の救済（最初のプールを rank したあと、30 位より後ろなら 30 位へ。`conversion::rescue_reading_kana`、文節のモードは `pin_first` のあとにも。`convert.kana_rescue_rank`、0 で無効。[0052](adr/0052-reading-kana-rescue.md)）、ユーザーデータ（`user/`。同じ読みで直前に選んだ表記は `LearningStore` のメモリ `RecentChoices` に持ち、数分だけ加点する。保存しない。[0045](adr/0045-recent-choice-bonus.md)） | [0001 §4](adr/0001-phase0-architecture.md)、[0002](adr/0002-phase1-architecture.md)、[0003](adr/0003-topic-ranker-decision.md)、[0020](adr/0020-english-spelling-first-candidate.md)、[0024](adr/0024-lm-keeps-rule-s.md) |
| `lm` | zenz（GGUF、llama.cpp）による話題スコア。feature `llama` / `metal` / `vulkan` のときだけ llama.cpp をビルドする（`vulkan` は Windows の GPU。バックエンドは別の DLL）。推論を別プロセスに頼む口（`host`。取り決め `host::proto`、親の側 `host_loader`、子の側 `serve`）と、その子の bin `ikari-lm-host`（feature `llama` が要る）を持つ。`ikari-lm-host-fake` はテスト用の偽の子（配らない） | [0002 §13.2](adr/0002-phase1-architecture.md)、[0003](adr/0003-topic-ranker-decision.md)、[0059](adr/0059-windows-gpu-inference.md)、[0060 Q4](adr/0060-user-selectable-lm-model.md) |
| `stack` | Converter、Ranker、LM の共有の組み立て（`build_stack`）。エンジン、CLI、eval が使う | [0004 §3.2](adr/0004-phase2-macos.md) |
| `import` | 取り込みファイル（JSONL）の検証、強制アライメント、初期学習 | [0002 §11](adr/0002-phase1-architecture.md) |
| `proto` | エンジンと殻の間のメッセージ（`proto/ikari/engine/v1/engine.proto`、prost）。設定アプリと殻の間の `proto/ikari/bridge/v1/bridge.proto` も生成する（`shell-core` の bridge が使う。Swift は `gen-proto.sh`） | [0004 §5](adr/0004-phase2-macos.md)、[0005 §6.3](adr/0005-macos-settings-and-indicator.md) |
| `engine` | 常駐エンジン。`Envelope` を受けて返す。`lib.rs`（`Engine`、`Inner`、上限）、`dispatch`（入口）、`session`、`convert`、`lm`、`data`（データの監視）、`workers`（裏のスレッド）、`segments`（文節）、`admin`（管理）、`cache` | [0004 §7](adr/0004-phase2-macos.md) |
| `engine-ffi` | エンジンの C ABI（staticlib。ヘッダ `include/ikari_engine.h`） | [0004 §4.2](adr/0004-phase2-macos.md) |
| `shell-core` | 殻の OS に依存しない処理の全部（入力の状態機械、キー割り当て、合成器の呼び出し、英語のモードの文脈（`english`）、郵便番号と Unicode の表記（`postal`）、ショートコード、候補ウィンドウの配置（`panel`）、メニュー、エンジンとの通信と再接続（`engine`）、設定アプリとのブリッジの判断（`bridge`）、設定の検証と整形（`settings`）、診断ログ）。入口は「OS のイベント → `Shell::dispatch` → 動作の列」の 1 本。共通のシナリオ（`shell-core/scenarios/`）で振る舞いを守る | [0041](adr/0041-shell-state-machine-in-rust.md) |
| `shell-ffi` | 殻と設定アプリが使う `shell-core` の C ABI（staticlib。ヘッダ `include/ikari_shell.h`。`ikari_shell_*`、`ikari_settings_*`、`ikari_panel_*`。入出力は protobuf の `ikari.shell.v1`・`ikari.settings.v1`） | [0041 §4.4](adr/0041-shell-state-machine-in-rust.md) |
| `pipe` | TIP と常駐のエンジンをつなぐ、枠つきの双方向の流れ（枠の符号化、要求と返事の多重化、時間切れ、クライアントとサーバー）。標準ライブラリだけ。名前付きパイプの OS 層は `cfg(windows)`（W1-b）。`shell-core` に依存しない | [0040 §2.2、§5](adr/0040-windows-feasibility.md) |
| `windows/engine-host`（`ikari-engine-host`） | Windows の常駐のエンジン（exe は `ikari-engine-host.exe`）。`pipe::Server` で受けた枠を、接続ごとの `Connection::handle`（ADR 0064）に渡す。受け付けの繰り返しは OS に依存しない | [0040 §2.2](adr/0040-windows-feasibility.md) |
| `windows/tip`（`ikari-tip`） | Windows の TIP（各アプリに入る DLL。cdylib）。キー（VK → `ShellEvent`）、動作の列 → TSF の操作、secure の判定、モードの写し、`pipe::Client` を `EngineTransport` に合わせる輸送は OS に依存せず Mac でテストする。TSF の COM の層は `src/com`（`cfg(windows)`、`unsafe` はここだけ） | [0040 §2.2、§2.8](adr/0040-windows-feasibility.md) |
| `windows/candidate`（`ikari-candidate`） | Windows の候補窓の核（契約の骨組み。中身はスタブ。ADR 0062）。TIP → 候補窓の命令（`proto/ikari/candidate/v1/candidate.proto`）の型、行の高さ（固定の関数）、横の位置、画面（C#）が呼ぶ C の ABI（`include/ikari_candidate.h`）。TIP の送り口は `windows/tip/src/candidate_send.rs` | [0062](adr/0062-windows-candidate-window.md) |
| `runmark`（`ikari-runmark`） | 起動中の印と、前回の終わり方の判定（`exit=clean|term|crash`）。`engine` と `shell-core`、`shell-ffi` が使う | [0051](adr/0051-crash-exit-record.md) |
| `textfile`（`ikari-textfile`） | テキストファイルの形式の共通部分（UTF-8・LF で書き、LF・CRLF・BOM つきを読む）。標準ライブラリだけ | [0040 §2.9](adr/0040-windows-feasibility.md) |
| `testtime`（`ikari-testtime`） | 時間に依存するテストの倍率（`IKARI_TEST_TIME_SCALE`）。テストでだけ使う | — |
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
engine, pipe, shell-core, core ← windows/engine-host（shell-core は設定のパイプのため。core はデータの置き場所の決め方）
shell-core, pipe ← windows/tip
shell-core, pipe, proto ← windows/candidate（windows/tip も使う）
textfile ← core, dict-builder, engine, import, shell-core, xtask
runmark ← engine, shell-core, shell-ffi
resource ← engine
testtime（dev のみ）← pipe, engine, lm, windows/tip, windows/engine-host
```

- `lm` と `engine-ffi` は feature なしなら llama.cpp をビルドしないので、`cargo build` / `cargo test` は workspace 全体を対象にする。
- LM を有効にする feature は `lm`（GPU なし）、`lm-metal`（`cli`、`eval`、`stack`、`engine`、`engine-ffi`）、`lm-vulkan`（Windows。`eval`、`stack`、`engine`、`windows/engine-host`。ADR 0059）。`lm` crate の中は `llama` / `metal` / `vulkan`。Vulkan はバックエンドを別の DLL にして exe の隣の `backends\` から読み、GPU が使えなければ CPU に落ちる（デバイスの選択は `lm/src/llama.rs`、設定は `engine.lm_gpu`）。
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
  └─ IkariEngineService（Swift は FFI を呼ぶだけ）→ engine-ffi（Rust のエンジン）
        │ 標準入出力のパイプ（枠 + 採点の要求と NLL。lm の host::proto）
ikari-lm-host（IkariEngine.xpc/Contents/MacOS/ に同梱。LM の推論だけ。llama.cpp、Metal。エンジンが起動し、Sandbox を引き継ぐ。ADR 0060 Q4）
```

- LM の推論は `ikari-lm-host` が持つ。エンジンは、自分の実行ファイルの隣にそれがあれば子として起動し、採点を頼む（`engine` の `LmHostChoice::Beside`）。子が落ちても固まっても、エンジンは LM なしの順位で返し続け、次の要求で起動し直し、続けて失敗したら諦める（エンジンを起動し直すまで）。隣に無ければ、同じプロセスで動かす（llama.cpp を入れたビルドだけ。`--no-lm` のビルドは LM なし）。

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

## 2.1 Windows のプロセスの構成（[ADR 0040 §2.2](adr/0040-windows-feasibility.md)。今あるのは `pipe`、`windows/engine-host`、`windows/tip`、候補窓の契約（`windows/candidate` のスタブ。[0062](adr/0062-windows-candidate-window.md)）。候補窓の画面と設定画面はまだない）

```
アプリのプロセス（各アプリ）
  └─ ikari_tip.dll（windows/tip。Rust、cdylib。TSF の COM は cfg(windows) の薄い層。判断は shell-core）
       │ 名前付きパイプ（枠 + Envelope。pipe::Client）
ikari-engine-host.exe（windows/engine-host。ユーザーのセッションで常駐。pipe::Server → engine。ADR などの `ikari-engine.exe` はこれ）
  └─ ikari-lm-host.exe（lm の bin。エンジンの exe の隣。LM の推論だけ。エンジンが起動し、Job Object に入れる。標準入出力のパイプ。ADR 0060 Q4）
候補窓（windows/candidate。WinUI 3 の別プロセス `ikari-candidate.exe`。engine-host が起動して見張る。TIP から片道のパイプで送る。ADR 0062。契約と TIP の送り口は入っていて、画面と起動はこれから）
設定画面（方式は ADR 0040 §2.5 のスパイクで決める）
```

- データの置き場所: `%LOCALAPPDATA%\ikari-ime`（開発用の口: `dev-knobs` のビルドと CLI・評価だけ、`IKARI_USER_DIR` があればそちら。製品のビルドでは読まない。`core/src/user/dir.rs`。2026-11-12 決定、ADR 0066 Q2・0067）。ログ、ロック、辞書・モデル、学習、設定は同じ下に置き、同期しない。
- 依存の向きは `windows/tip → pipe`、`windows/tip → shell-core`、`windows/engine-host → pipe`、`windows/engine-host → engine`、`windows/engine-host → shell-core`（設定のパイプ）、`windows/engine-host → core`（データの置き場所）の一方向。
- 検証: Mac で `cargo check --workspace --tests --target x86_64-pc-windows-msvc`（`scripts/check.sh` に入っている。LM なし。外す crate は今ない）。CI の job `windows`（`windows-latest`。`cargo test --workspace`、LM なし。必須ではない）。キー入力と画面は実機（VM）。

## 2.2 プロセス間の取り決め（共通の規則。#395。2026-11-15 の Fable の案）

「今の状態」の表で、決定ではない。境界を足す・変える PR は、この表を一緒に直す（理由は ADR にリンクで書き、ここには書かない）。「未決」は決まっていないもの。S1〜S5 は、セキュリティの指摘の番号（issue で追う）。セキュリティの規則（P1〜P6、R1〜R6）は [security-design.md](security-design.md)、口ごとの決定は [ADR 0068](adr/0068-ipc-endpoints.md)。決定済みでも、実装が入るまでは「実装はこれから」と書く。

| 境界（名前、起動する者） | 締め切り | 固まったとき | 落ちたとき | 版違い | 認証（相手の確認） | プライバシー（出してよいもの） |
| --- | --- | --- | --- | --- | --- | --- |
| **エンジン**: TIP → engine-host。`ikari-ime-engine-v1-<SID>-s<セッション>`。起動は W5（未決）。macOS は XPC、launchd | 返事: 変換 `min(300, max(30, 6×字数)) + 40`ms、管理 3 秒、転送 30 秒、郵便 1 秒（両 OS 共通）。書き: 200ms（暫定。#392）。接続: ふさがりで 100ms × 3、失敗後は 100ms〜1 秒控える（暫定） | 返事の時間切れ → かな候補。書きの締め切り → 切断 → 落ちたと同じ。時間切れ・つなぎ直しの失敗を休止に数えるかは未決（#404。案は PR #407。値は未計測） | 次の要求でつなぎ直し、`Hello` とセッションを送り直す。60 秒に 3 回で 300 秒休止。その間はかな候補（両 OS 共通）。エンジンを開けない（辞書がない、など）ときは、engine-host は終わらずに受け口を立て、`ENGINE_NOT_READY`（殻はかな候補）を返し、前の試みから 2 秒以上あけた次の要求で開き直す（両 OS 共通。Windows は #414。`windows/engine-host/src/slot.rs`） | 版の文字列: macOS は止めて起こし直す、Windows は共存（ADR 0064）。`schema_version` 違い → かな候補。壊す変更はパイプ名の `-v1-` | 受け口: 自分のユーザーと SYSTEM だけの DACL、リモート拒否、名前の先取り防止。TIP は、何かを送る前に相手の実行ファイルのパスを、DLL の場所から決まるインストール先のパスと照らし、違えば閉じる（S1。`windows/tip/src/peer.rs`。インストール先が管理者だけ書ける場所であることが前提）。**engine-host はつないできた相手を確かめない（決定。「同じ利用者のプログラムなら変換を頼める」口。ADR 0068 §3）**。高い権限の相手は断る（実装済み。`pipe/src/elevation.rs`）。macOS は署名要件で相互に確認 | 読み・左文脈・確定を運ぶ。ログは番号と固定の語だけ（NFR-19）。secure は記録しない |
| **設定**: 設定画面 → engine-host。`ikari-ime-settings-v1-…`。画面の方式は未決（W4-s3）。macOS は殻の Unix ソケット | 管理 3 秒、転送 30 秒。書き 200ms | 未決（画面の側の扱い） | 設定の接続が切れてもエンジンのパイプは動く。画面の側は未決 | 最初の枠で照合、違えば返事を送って閉じる（shell-core `bridge`。両 OS 共通） | DACL は上と同じ。**相手の確認なし（S3。決定: 双方が相手のパスを照らす。ADR 0068 §4。実装はこれから）**。macOS は同じユーザー + 署名 + cdhash + パス | 設定、ユーザー辞書、学習（書き出しを含む）。中継は許可リストだけ |
| **候補窓**: TIP → `ikari-candidate.exe`（片道）。`ikari-ime-candidate-v1-…`。起動と見張りは engine-host で、エンジンを開く前に起動する（[ADR 0062 §5.4](adr/0062-windows-candidate-window.md)。入口は `windows/engine-host/src/candidate_window.rs`。今は何もせず、起動と Job Object は W3 の L4）。macOS は同じプロセス | 返事なし。書き 200ms | 送れなければ捨てて切断、再送しない（ADR 0062 §3 の 4）。画面が固まった窓の検知は未決 | 窓なしで続行（§5.4）。エンジンと同じ間隔でつなぎ直す。持ち主の接続が切れたら核が隠す。起動し直しの上限は未決（値） | 知らない欄と命令は無視、`Hello.schema_version` はログだけ。世代は接続ごと（#405） | DACL は上と同じ。**TIP は相手のパスを照合していない（S4。決定: TIP が送る前に照らす。ADR 0068 §5。実装はこれから）。窓は送り手を確かめない（どのアプリの TIP も正当なので確かめられない）** | 候補と注釈の文字（表示に必要）。ログと既定の PNG には出さない |
| **LM の子**: engine → `ikari-lm-host`。標準入出力（名前なし）。起動はエンジン（[ADR 0060](adr/0060-user-selectable-lm-model.md) Q4。`lm/src/host/`）。両 OS 共通 | 準備（起動からモデルの読み込みの完了まで）60 秒、採点 1 回（書きと読みを合わせて）30 秒（どちらも仮置き。計測で決める）。変換の待ち（`engine.lm_deadline_ms`。300ms まで）は別で、今までどおり | 締め切りで kill して wait → その回は LM なしの順位。変換は止めない | 次の要求で起動し直す。連続 3 回の失敗（起動できない、準備が間に合わない、落ちた、固まった、壊れた返事、版違い、読み込みの失敗）で諦め、以後は起動しない（仮置き）。GPU を使う設定では、諦める前に CPU だけの子（`--gpu cpu`）に切り替えて、もう 3 回まで試す。採点が 1 回成功すれば数は 0 に戻る。エンジンの再起動で忘れる。子が生きていて返した採点 1 件の失敗は数えない。アイドルの解放（`engine.idle_unload_secs`）では子を終わらせ、次の要求で起動する。エンジンが死ぬと、子は stdin が閉じたことで終わる（Windows は Job Object でも） | `Ready` の枠に版（`PROTOCOL_VERSION`）。違えば使わない（失敗に数える）。子はエンジンと同じ場所に置き、一緒に入れ替える前提 | 名前がないので横取りはない。exe はエンジンの実行ファイルの隣の絶対パス（PATH から探さない）。macOS はエンジンの署名に封印。**Windows で子に渡るハンドルは標準入出力の 3 つだけ（S5。ADR 0068 §6.1。実装済み）**。檻は段階で入れる（[ADR 0072](adr/0072-lm-cage-levels.md)。段 1: Job の制限と低い整合性は実装済み。段 2 は計測と実機の確認のあと）。macOS はエンジンの App Sandbox を引き継ぐ | 左文脈と読みと候補（採点に必要）。子から受け取るのは数値と、装置の固定の語だけ（ログに書く前に文字を検査する）。子の失敗の文面はログに書かない |

## 3. データの置き場所

| 場所 | 中身 | 作る人 |
| --- | --- | --- |
| `data/raw/`（gitignore） | SudachiDict、chiVe、zenz のモデル、AJIMEE、kanakanji-edge、emoji-data（`emoji-data/emoji.json`。コミット固定、[0027 §5](adr/0027-emoji-input.md)）、ABR の町字マスター（`abr/mt_town_all.csv`。任意。[0030](adr/0030-abr-town-vocabulary.md)） | `scripts/fetch-data.sh` |
| `data/build/dict/`（gitignore） | `grammar.bin`、`system.dict`、`vectors.bin`、`BUILD_INFO`、`emoji.tsv`（emoji.json があれば。1,975 名・1,914 絵文字。辞書の入力・BUILD_INFO には含めない）。旧版は `data/build/dict.prev/`。ABR があれば、地名の語彙の中間ファイル `data/build/town_lex.csv` も作る（[0030](adr/0030-abr-town-vocabulary.md)） | `scripts/build-dict.sh` |
| `~/Library/Application Support/ikari-ime/dict/`、`models/` | エンジンが読む辞書とモデル（上の 2 つのコピー） | `scripts/install-data.sh` |
| 同 `learning.tsv`、`profile.tsv`、`user_dict.tsv`、`ingest_*.tsv` | ユーザーデータ（学習、属性プロファイル、ユーザー辞書、取り込み）。開発用の口（`dev-knobs` のビルドと CLI・評価だけ）では `IKARI_USER_DIR` で場所を変えられる。製品のビルドは読まない（`--data-dir` も受けない。ADR 0067 §4.3、0068） | エンジンと `ikari` CLI（[0002 §14](adr/0002-phase1-architecture.md)） |
| `core/assets/packs/it.tsv` | IT の語彙集（読みのカタカナが1位、英字が2位）。`ikari-core` にコンパイルして埋め込む（配置は要らない）。`DictionarySet` の slot は システム辞書、語彙集、ユーザー辞書 の順。`settings.tsv` の `dict.pack_it`（既定 `on`）で切れる（[0012 §4](adr/0012-new-word-dictionary.md)） | 人（Claude の下書きをユーザーが確認） |
| `target/macos/<release\|debug>-<team\|adhoc>/ikari.app` | ビルドしたバンドル | `macos/scripts/build.sh` |
| `testdata/sudachi-mini/` | テスト用の小さな辞書の fixture（リポジトリに入れてある） | — |
| `eval/sets/`、`eval/reports/` | 評価セットと結果のレポート（[索引](../eval/reports/README.md)） | 人とエージェント |

### エンジンのログ（`logs/engine.log`）と Space の待ち（[ADR 0021](adr/0021-lm-lookahead.md) L0・L1）
- 1 行は `時刻ms 種類 所要us lm=<状態> err=<種類> [注記]`。注記は固定の語の `key=value` だけ（読み・候補・文脈は書かない。NFR-19）。
- 書き込みは別スレッド `ikari-log`（有界のキュー 8192 行、64KB のバッファ、1 秒ごとに flush）。`Engine::handle` はキューに入れるだけでディスクに触らない。満杯なら捨てて、次に書けたときに `log_dropped n=N` の行を書く（[ADR 0034](adr/0034-diagnostic-log.md) §3.1）。
- 4MB を超えるか、ローカルの日付が変わったら回し、`logs/engine-YYYYMMDD-HHMMSS.log.gz` にする。180 日か合計 100MB のどちらか先で古い順に消す（同 §3.2）。読むときは `cd ~/Library/Application\ Support/ikari-ime/logs && gzip -dc engine-*.log.gz | cat - engine.log | less`（名前の順が時刻の順。macOS の `zcat` は `.gz` を読まないので `gzip -dc` か `gzcat`）。
- 変換の行を結ぶ番号（番号だけで文字ではない）: `convert` と `reconvert` に `session=<番号> conv=<変換の番号>`、`page` `segment_page` `resize_segment` `commit` `commit_segments` `cancel` に `conv=`。`sh_episode` も同じ `session=` を持つので、「どの変換で LM が効かなかったか」「LM なしの変換のまま確定したか」を、時刻の順と番号で結べる。
- 接続の番号（Windows の engine-host。#398）: 既定の接続（macOS）以外の `session=` の行（`begin_session` `end_session` `convert` `reconvert` `sh_episode` ほか）は `session=N conn=M` の順に書く。接続の開始と終了は `connect conn=M` と `disconnect conn=M`（番号だけ）。macOS の接続 0 の行は変えない。殻の版がエンジンと違う `hello` の行には（接続を問わず。macOS でも版が違えば付く） `peer_version=<殻の版> skew=1`（版は `[A-Za-z0-9.+-]` で 32 バイトまで、外れたら `invalid`）。TIP 側の版の違いは OutputDebugString に `ikari-tip: version skew tip=<版> engine=<版>`。
- `convert` の注記: `segments=N`、`cache=hit|wait_hit|wait_miss|miss`、`lm_applied=1|0`（返事の `lm_applied`。3 つ目の項目の `lm=<状態>` とは別のキー）、外れ（`cache=miss`）なら `miss=reading|context|none`（同じセッションのキャッシュに、読みだけが違うもの／文脈の番号だけが違うものがある／近いものがない）。秘密の入力は `cache=miss miss=none`。続けて `gap=<1 位と 2 位のスコアの差>`（候補が 1 件以下なら `na`）と `confident=1|0`（#69）。続けて `rlen=<読みの文字数>`（secure は書かない）と `qlen=<変換の時点で終わっていない先行計算の数（待ち + 処理中）>`（#159。数だけで文字は出さない）。
- `prefetch_done` の注記: `lm_applied=1|0`（LM が間に合って使われたか）、`lm_calls=N`（その先行計算での LM 呼び出し回数。2 文節以上は 2）。
- 先行計算の左文脈: 合成の最初の `contextRefreshKeystrokes` 打鍵は左文脈を読み直す（#1。変換中に次の文字を打つと、1 打鍵目の文脈は直前の確定がアプリに入る前に読まれるため）。文脈の番号（`contextRev`）が進んだら、その文脈で先行計算を送り直す。それより後の打鍵は読み直さず、Space で読み直す。
- Space（`Convert`）で先行計算が計算中なら、**Space から最大 30ms**（既定。`settings.tsv` の `engine.convert_wait_ms`、0〜200。`Engine::open` のときだけ読む。NFR-04）待つ。先行計算が LM を待てる締め切り（`LmBudget.max`）も既定 300ms で、`engine.lm_deadline_ms`（50〜2000）で変えられる。範囲は暫定。間に合わなければ LM なしでその場で変換する。以前の「Prefetch の受付＋`lm_budget`」は廃止。再変換・区切り直しは待たない（変更なし）。

## 4. スクリプト

| スクリプト | 役割 |
| --- | --- |
| `scripts/setup.sh` | 取得 → 辞書のビルド → データの配置 → アプリのビルドとインストール → smoke を通す |
| `scripts/fetch-data.sh`、`build-dict.sh`、`install-data.sh` | 上の各段。単独でも使える |
| `scripts/check.sh` | コミットの前に流す門（fmt、clippy、test、Windows ターゲットの `cargo check`、`swift test`、文書のリンク。`--lm` `--real` `--perf` `--eval`） |
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

- 上の表は初期の 6 本だけ。**すべての ADR の一覧と状態は [adr/README.md](adr/README.md)（索引）**。0006 以降の決定（Windows は 0040〜0042、0059〜0064 など）はそちらを見る。
- ユーザーの指摘と、それを受けた決定は [user-feedback.md](user-feedback.md) にまとめてある（具体例つき）。
- 今のタスク（ユーザーが要るものと、Claude が進められるもの）は GitHub の issue と Project。
