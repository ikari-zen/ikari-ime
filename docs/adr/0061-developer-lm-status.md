# ADR 0061: 開発者向けに、その変換で LM が間に合ったか・かかった時間・GPU か CPU かを、ログと候補窓に出す

- 状態: 提案（2026-11-14）
- 日付: 2026-11-14
- 対象: `lm/src/ranker.rs`・`lm/src/scorer.rs`・`lm/src/llama.rs`（計測と装置）、`engine/src/workers.rs`・`cache.rs`・`convert.rs`・`diag.rs`（1 回分のまとめ、返事、ログ）、`proto/ikari/engine/v1/engine.proto`・`proto/ikari/shell/v1/shell.proto`、`shell-core/src/settings/`・`machine/`・`types.rs`・`scenario.rs`、`shell-core/scenarios/`、`shell-ffi/src/convert.rs`、`macos/Sources/IkariIME/CandidatePanel.swift`、`macos/Sources/IkariSettings/`
- 前提: main = d996a8c。ADR 0004 §3.4（変換 1 回の締め切り）、ADR 0021（先行計算と L0 の `lm_applied`）、ADR 0034（診断ログは常時オン、スイッチを作らない）、ADR 0040 §2.5 と #321・#326（設定画面の一覧は Rust）、ADR 0042（判断は Rust）、ADR 0058 §1.2（アイドル解放と読み直し）、ADR 0059（Windows の GPU。提案、PR #345）
- 判断の置き場所: Rust。計測は `lm`、1 回分のまとめは `engine`、表示するかどうかと表示の文は `shell-core`。Swift は、shell-core が作った 1 行を候補窓に描くことと、設定画面の 1 つのスイッチだけ（UI であるため）
- この ADR ではコードを変えない。「分かったこと」はコードで確かめたもの、「推測」は確かめていないもの

---

## 0. ユーザーの要望と回答（2026-11-14）

- 要望: Mac も Windows も、モデル（LM）が間に合ったかどうかを、変換のときに表示する機能を足す。開発者モードとして、既定はオフ。
- 表示の場所: ログと、候補窓の両方。ログは両 OS にすぐ。候補窓は Mac から（Windows の候補窓は W3 で作る。できてから）。
- 出す内容: (1) 間に合った／時間切れ、(2) LM の採点にかかった時間（ms。締め切りも）、(3) GPU か CPU か。
- 設定: 設定画面に「開発者」の欄を作る。既定オフ。説明に「開発者向け」と書く。

## 1. 現状（分かったこと）

### 1.1 LM が使われる経路
- Space（`Convert`）の時点で LM を待つことはない。LM を使うのは**先行計算**（`engine/src/workers.rs` の `run_job`。スレッド `ikari-prefetch` が 1 本）で、締め切りは `started + lm_budget.max`（= `engine.lm_deadline_ms`、既定 300ms）。`convert_until`、`page_until(0)`、`seg.prefill` の `rank` は同じ締め切りを使う。
- Space で先行計算に当たれば、その結果（と `lm_applied`）をそのまま返す。計算中なら `engine.convert_wait_ms`（既定 30ms）まで待つ。当たらない・間に合わないときは `convert_until(…, Some(Instant::now()))` で、**LM を使わずに**その場で変換する（`engine/src/convert.rs` の `convert`）。
- 2 文節以上の文節 1 以降の 1 ページ目は、返事のあとにスレッド `ikari-lookahead` が `lm_budget.max` で作る（`engine/src/segments.rs`）。ページ送り（`page`）と文節のページは、その場で `lm_budget.for_reading` まで待つ。
- → **先行計算のスレッドと先読みのスレッドが、同時に `rank` を呼ぶことがある。**

### 1.2 間に合ったかの今の判定
- `lm/src/ranker.rs` の `ZenzRanker` は、全体の累計の `LmStats`（`calls`、`timeouts`、`late_hits` など。`AtomicU64`）を持つ。時間は数えていない。
- エンジンは前後の `LmStats` の差で判定する: `lm_applied_since` = `calls` が増え、`timeouts` が増えていない（`engine/src/lm.rs`）。遅れて届いた結果を使った（`late_hits`）ときも「間に合った」になる。
- この `lm_applied` は、`ConvertReply.lm_applied` と `SegmentPageReply.lm_applied` で殻へ届き、**`confident` の閾値を選ぶ**（`CONFIDENT_GAP_WITH_LM` / `WITHOUT_LM`。#69）。表示のための値ではなく、振る舞いに効いている。
- 差で見るので、別のスレッドの `rank` が同じ時間に走ると混ざる（§1.1）。`lm_applied` の判定は今もこの性質を持つ。

### 1.3 ログ
- `convert` の行の注記: `cache=hit|wait_hit|wait_miss|miss lm_applied=0|1 miss=… gap= confident= rlen= qlen=`（`cache.rs` の `log_note`、`convert.rs`）。`prefetch_done` の行: 所要（先行計算全体の us）と `lm_applied= lm_calls=`。`lm_load` の行: 読み込みの所要と成否（`diag.rs`）。
- GPU か CPU かは、どこにも出ていない。`lm/src/llama.rs` の `load_model` は `n_gpu_layers = if cfg!(feature = "metal") { u32::MAX } else { 0 }`。Windows は ADR 0059（提案）で、Vulkan の GPU を選び、失敗したら CPU で読み直す。
- 診断ログは常時オンで、スイッチを作らない（ADR 0034 §結果、ユーザー決定 2026-10-25）。中身は固定の語と数だけ（NFR-19）。

### 1.4 殻
- `shell-core/src/machine/state.rs` の `ConversionPage` が `lm_applied`・`confident`・`engine_micros` を写している。候補窓の中身は `CandidatePanelModel`（`types.rs`。`candidates`、`selected`、`annotation`、`reserved_rows`）で、`shell.proto` の同名のメッセージで Swift に渡る（`shell-ffi/src/convert.rs`）。
- 似た前例: `candidate.hide_when_confident`。エンジンの返事の 1 つの値と、殻の設定の 1 つのスイッチで、shell-core が表示を決める。共通シナリオ `shell-core/scenarios/confident.json` で確かめている。
- 設定画面の一覧は `shell-core/src/settings/specs.rs`（#321）。画面は `Input` と `Conversion`、節は `LanguageModel` などで、LM の締め切り（`engine.lm_deadline_ms`）と待ち時間（`engine.convert_wait_ms`）は、すでに変換の画面の「言語モデル」の節にある。一覧は `macos/Resources/settings-specs.tsv` に書き出し、テストがずれを止める。

## 2. 目標と、目標にしないもの

- 目標: 変換 1 回ごとに、(1) 判定、(2) LM を待った時間と締め切り、(3) 装置（GPU / CPU）を、ログに（両 OS）、候補窓に（Mac。Windows は W3 のあと）出す。
- 守ること: **変換の結果、順位、タイミング、`lm_applied`、`confident` を変えない**（表示のための計測だけ）。オフのときの追加の費用がほぼ無いこと。
- 目標にしないもの: ページ送り・文節のページ・先読みの LM の表示（§3 の D7）。時間切れの要求が実際に何 ms で終わったか（§9 Q3）。モデルの名前（ADR 0060 が入ったら、要るか考える）。ikari-eval のレポートの変更。

## 3. 決定の要約（提案）

- **D1. 計測は `lm` が持つ。スレッドごとの記録にする。** `ZenzRanker::rank` は、呼び出したスレッドの記録（thread-local）に、呼び出し・時間切れ・遅れた結果の使用の回数と、返事を待った時間（`recv_timeout` の前後）と、ワーカーが報告した推論の時間を足す。エンジンは先行計算の前に記録を空にし、あとで取り出す（`lm::trace::begin()` / `lm::trace::take() -> LmTrace`）。先読みのスレッドの `rank` は混ざらない。全体の累計の `LmStats` は今のまま残す（eval とテストが使う）。
- **D2. 判定は今の `lm_applied` から作る。** 表示の「間に合った」は、殻の `confident` を決めた値と同じでなければならない。スレッドごとの回数は判定には使わず、ログに並べて書く（混ざりの検出に使える）。`lm_applied` の求め方は変えない。
- **D3. 装置は、モデルを読んだときに決まる。** `LmScorer` に `fn device(&self) -> LmDevice`（既定の実装は `Cpu`）を足し、`LlamaScorer` は GPU に層を載せて読めたら `Gpu` を返す。ワーカーは読んだあとに `Shared` へ書き（`AtomicU8`）、`LmStats.device` と `LmTrace.device` で見える。アイドル解放のあとの読み直しで変わりうるので、変換ごとに読む。
- **D4. エンジンは `ConvertReply` に `LmReport` を常に入れる。** スイッチを見ない（数個の数で、費用はほぼ無い）。先行計算の結果は `CacheEntry` の `Ready` に `lm_applied` と並べて `LmTrace` を持たせる。
- **D5. ログは常時（§9 Q1。推奨）。** `convert` の行と `prefetch_done` の行に `lm_us= lm_infer_us= lm_deadline_us= lm_dev=` を、`lm_load` の行に `dev=` を足す。固定の語と数だけ。
- **D6. 候補窓に出すかと、出す文は shell-core が決める。** 設定 `developer.lm_status`（Bool、既定 `off`、すぐに効く）。オンなら、shell-core が `LmReport` から 1 行の文を作り、`CandidatePanelModel.developer_note` に入れる。オフなら空で、文を作らない。Swift は空でなければ、注釈の下に小さな 1 行で描く。Windows の候補窓（W3）は同じ欄を描くだけでよい。
- **D7. 範囲は Space の最初の返事だけ。** 同じ変換のあいだ（ページ送り、文節の移動）は同じ行を出し続ける。ページや文節のページで新たに LM を待った分は、今回は出さない（`SegmentPageReply.lm_applied` は今のまま）。
- **D8. 設定画面に「開発者」の節を足す**（§9 Q2。推奨は変換の画面のいちばん下の節）。

## 4. 計測（`lm`）

### 4.1 `LmTrace`（新しい。`lm` の公開の型）

| 欄 | 型 | 意味 |
|----|----|------|
| `calls` | `u32` | このスレッドで、採点の対象があり締め切りのある `rank` の回数（`LmStats.calls` と同じ数え方） |
| `timeouts` | `u32` | そのうち間に合わなかった回数（`expired`、重ねて送らなかった、送れなかった、待ちの時間切れを含む） |
| `late_hits` | `u32` | 遅れて届いた結果を使った回数 |
| `wait_micros` | `u64` | 返事を待った時間の合計（`recv_timeout` の前後。送っていない呼び出しは 0） |
| `infer_micros` | `u64` | 受け取った返事の、ワーカーでの推論（`nll`）の時間の合計。時間切れの要求の分は入らない |
| `device` | `LmDevice` | 取り出したときの装置（`Unknown` / `Cpu` / `Gpu`）。モデルが読まれていなければ `Unknown` |

- 推論の時間は、ワーカーが `nll` の前後を測り、返事（今の `Scored`）と一緒に送る。返事の型を `(Scored, Duration)` にする（`lm` の中だけの変更）。遅れて届いた結果の 1 件の置き場所（`late`）にも時間を残し、`late_hits` のときは `infer_micros` に足す。
- `begin()` はこのスレッドの記録を 0 にする。`take()` は取り出して 0 にする。`begin` を呼ばないスレッドでも足すだけで、だれも読まない（費用は `Instant::now()` 2 回と `Cell` の書き込み数回。LM の待ちは ms の単位なので無視できる）。
- `LmStats` に `device: LmDevice` を足す。時間の累計は足さない（使う人がいない）。

### 4.2 装置
- `LmDevice` は `lm` に置く（3 つで足りる）。
- macOS（`metal`）: `n_gpu_layers = u32::MAX` で読めたら `Gpu`。llama.cpp が Metal の初期化に失敗して黙って CPU で動く場合があるかは**推測**（未確認）。実装のときに、読んだあとの装置の一覧（`list_llama_ggml_backend_devices`）かモデルの情報で確かめられるなら、それを使う。確かめられなければ「GPU を要求して読めた」を `Gpu` とし、ADR の結果に注記する。
- Windows: ADR 0059 §2.4 の選択の結果（GPU で読めた / CPU のデバイスだけで読んだ / GPU で失敗して CPU で読み直した）をそのまま返す。ADR 0059 が入る前は常に `Cpu`。

## 5. 1 回分のまとめと返事（`engine`）

### 5.1 判定（`LmVerdict`）

| 値 | 条件 | 表示で言いたいこと |
|----|------|--------------------|
| `LM_APPLIED` | 先行計算に当たり、その `lm_applied` が真 | 間に合った（遅れて届いた結果を使った場合を含む。`late_hits` で区別できる） |
| `LM_TIMED_OUT` | 先行計算に当たり、`lm_applied` が偽で `calls > 0` | LM の締め切りに間に合わなかった |
| `LM_NOT_SCORED` | 先行計算に当たり、`calls == 0` | 採点する候補がなかった（かなだけなど） |
| `LM_PREFETCH_LATE` | 計算中の先行計算が `convert_wait` までに終わらなかった（`wait_miss`） | Space の待ちに間に合わず、LM なしで変換した |
| `LM_NO_PREFETCH` | 先行計算がなかった（`miss`） | LM なしで変換した |
| `LM_SECURE` | 秘密の入力 | LM を使わない（NFR-18） |
| `LM_UNAVAILABLE` | `lm_state` が ready でない（無効、読み込み中、失敗） | LM がない。状態は既存の `LmState` で言う |

- 判定の順は表の下から（`LM_UNAVAILABLE` と `LM_SECURE` を先に見る）。読み込み中に先行計算が走った場合は、今の実装では時間切れとして数えられる（`ranker.rs` の冒頭の注記、NFR-15）。このときは、変換の時点の `lm_state` が loading なら `LM_UNAVAILABLE` とする（読み込みの遅れと締め切りの不足を分けるため）。
- `LM_TIMED_OUT` の条件は「`lm_applied` が偽」で決め、スレッドごとの `timeouts` は見ない（D2）。

### 5.2 先行計算と Space
- `run_job` は、`convert_until` の前に `lm::trace::begin()`、`seg.prefill` のあとに `take()` を呼び、`entry.set_ready(conv, applied, trace)` で残す。`lm_applied_since` の呼び方は変えない。
- `convert` は、当たったら `CacheEntry` の `LmTrace` から、当たらなかったら（`miss` / `wait_miss` / secure）締め切りの値だけで `LmReport` を作る。`lm_deadline_micros` は `lm_budget.max`、`convert_wait_micros` は `self.convert_wait`。

### 5.3 proto（契約。engine.proto）

```proto
enum LmVerdict { LM_VERDICT_UNSPECIFIED = 0; LM_APPLIED = 1; LM_TIMED_OUT = 2; LM_NOT_SCORED = 3;
                 LM_PREFETCH_LATE = 4; LM_NO_PREFETCH = 5; LM_SECURE = 6; LM_UNAVAILABLE = 7; }
enum LmDevice { LM_DEVICE_UNSPECIFIED = 0; LM_DEVICE_CPU = 1; LM_DEVICE_GPU = 2; }
// 開発者向けの表示とログのための、その変換の LM の様子（ADR 0061）。振る舞いには使わない
message LmReport { LmVerdict verdict = 1; uint32 wait_micros = 2; uint32 infer_micros = 3;
                   uint32 lm_deadline_micros = 4; uint32 convert_wait_micros = 5; LmDevice device = 6;
                   uint32 calls = 7; uint32 timeouts = 8; uint32 late_hits = 9; LmState lm_state = 10; }
// ConvertReply に足す
LmReport lm_report = 9;   // 新しいエンジンは Convert の返事に常に入れる。ない（古いエンジン）なら殻は表示しない
```

- `Page` の返事（`PageReply` も `ConvertReply`）には入れない（D7）。
- 時間は us（`engine_micros` と同じ単位）。表示で ms に丸めるのは shell-core。
- `lm_state` は `LM_UNAVAILABLE` の中身（無効・読み込み中・失敗）を言うため。既存の `LmState` を使う。

### 5.4 ログ（D5）
- `convert`: 当たったとき `lm_us=<wait> lm_infer_us=<infer> lm_deadline_us=<lm_budget.max> lm_dev=gpu|cpu|none lm_calls=<n> lm_to=<timeouts>`。当たらなかったときは `lm_dev=` だけ（判定は既存の `cache=` と `lm=` で分かる）。secure の行には足さない。
- `prefetch_done`: 同じ欄を足す（既存の `lm_applied= lm_calls=` はそのまま）。
- `lm_load`: `dev=gpu|cpu`（失敗のときは付けない）。
- `scripts/diag-report.py` は知らないキーを無視するので、壊れない（ADR 0034 §5）。集計に足すかは別のタスク。

## 6. 表示（`shell-core` と殻）

### 6.1 設定（契約）
- キー: `developer.lm_status`。値は `on` / `off`（既存のスイッチと同じ書き方）。既定 `off`。`Reflect::Immediate`。殻だけが読む（エンジンの設定ではない。`engine_settings_differ` に入れない）。
- `SettingsSnapshot` に `developer_lm_status: bool`（`shell.proto` の `SettingsSnapshot` に `bool developer_lm_status = 21;`）。
- `specs.rs`: `SettingsSection::Developer`（id `developer`）を足し、置く画面は §9 Q2 の答えで決める（推奨は `Conversion` のいちばん下）。文言の鍵は今の決まりで `settings.developer.lm_status.label` / `.description`、`settings.section.developer.title`。`settings-specs.tsv` を作り直す。
- 文言（案）: 題「開発者」。項目「変換のときに LM の様子を表示する」。説明「開発者向け。候補窓の下に、言語モデルが間に合ったか、待った時間と締め切り、GPU か CPU かを出します。ふだんはオフのままにしてください。」

### 6.2 候補窓の 1 行（契約）
- `CandidatePanelModel`（`types.rs`、`shell.proto`）に `developer_note: String`（`string developer_note = 5;`。空なら行を出さない。serde は空なら省く）。
- shell-core は `ConversionPage` に `lm_report: Option<LmReportInfo>` を写し、変換のあいだ `ConvertingState` に持つ。`panel_model` で、`settings.developer_lm_status` が真のときだけ文を作る。
- 文（案。1 行。数は ms に丸める）:
  - `LM_APPLIED`: `LM 間に合った 42ms / 締め切り 300ms（推論 38ms）GPU`（遅れて届いた結果なら「間に合った（遅れて届いた結果）」）
  - `LM_TIMED_OUT`: `LM 時間切れ 300ms / 締め切り 300ms GPU`
  - `LM_NOT_SCORED`: `LM 採点なし GPU`
  - `LM_PREFETCH_LATE`: `LM なし: 先行計算が Space から 30ms に間に合わず`
  - `LM_NO_PREFETCH`: `LM なし: 先行計算なし`
  - `LM_SECURE`: `LM なし: 秘密の入力`
  - `LM_UNAVAILABLE`: `LM なし: 読み込み中` / `無効` / `失敗`（`LmReport.lm_state` から）
- 確信が高く 1 位だけを出す場合（`candidate.hide_when_confident`）は、候補窓を出さないので、行も出ない。窓を出す・出さないの判断は変えない（ログで見られる）。
- `reserved_rows`（位置を決める高さ）には数えない。Swift が行の分だけ窓を高くする。**推測**: 窓の下端が画面の端に近いとき、1 行分はみ出しうる。開発者向けなので、今回は受け入れる。

### 6.3 Swift（入口と UI だけ）
- `CandidatePanel.swift`: `developerNote` が空でなければ、注釈の下に小さい文字で 1 行描く。中身は解釈しない。
- `IkariSettings`: 一覧に従って「開発者」の節とスイッチを出す。説明の文に「開発者向け」。
- Windows: ログは engine-host が同じエンジンを使うので、コードの変更なしで出る。設定画面は一覧（`specs.rs`）に従うので、節が増える（**推測**: Windows の設定画面が一覧から自動で節を作るかは、実装のときに確かめる）。候補窓は W3 で `developer_note` を描く。

### 6.4 作者以外の利用者にも見える（注意）
- 「開発者」の節は、配る版（#99）でもすべての利用者に見える。既定オフで、説明に「開発者向け」と書き、オンにしても入力の結果は変わらない（表示だけ）ことを守る。
- 出すのは判定・時間・装置だけで、読みや候補や文脈は出さない（ログと同じく NFR-19 の範囲）。画面の共有や録画で他人に見えても困らない内容にする。
- 利用者がオンのまま不具合を報告してきたら、行の中身はそのまま手がかりになる（利点）。

## 7. 共通シナリオで確かめる範囲

新しいファイル `shell-core/scenarios/developer-lm-status.json`。`ScriptPage` に `lm_report`（省略可。`serde(default)`）を足す（契約）。

| シナリオ | 確かめること |
|----------|--------------|
| off は行を出さない | 既定（設定なし）で、`lm_report` があっても `show_candidates` に `developer_note` がない |
| on・間に合った | `developer.lm_status` を on にして、`LM_APPLIED`、待ち 42000us、推論 38000us、締め切り 300000us、GPU の返事から、§6.2 の文 |
| on・時間切れ | `LM_TIMED_OUT` と CPU の文 |
| on・LM なし | `LM_PREFETCH_LATE`、`LM_NO_PREFETCH`、`LM_SECURE`、`LM_UNAVAILABLE` の文 |
| on・古いエンジン | `lm_report` なしなら行を出さない |
| on・ページ送り | 次のページでも同じ行のまま |
| on・確信が高い | `hide_when_confident` が既定のままなら窓を出さない（今の `confident.json` と同じ動作） |

シナリオで確かめられないもの: 判定と時間の中身（エンジンの Rust のテスト。モックの `LmScorer` で、間に合う・時間切れ・遅れた結果・装置を作る。`lm/src/ranker/tests.rs`、`engine/src/tests/convert.rs` の形）、先読みのスレッドと混ざらないこと（同じテストで、別のスレッドから `rank` を呼んで `take()` が変わらないこと）、Swift の描画（目で見る）。

## 8. 実装の進め方（レーンと順序）

1 タスク 1 ブランチ（issue は task-keeper が切る）。

| 順 | レーン | 中身 | 持つファイル | 前提 |
|----|--------|------|--------------|------|
| 0 | 契約（リード、または code-editor 1 本） | §5.3 の proto と生成物、§6.1・§6.2 の型と既定値（`developer_note` は空、`lm_report` は `None`、`developer_lm_status` は偽）、`lm` の `LmTrace`・`LmDevice`・`trace::begin/take` と `LmScorer::device` の宣言（中身は 0 を返すスタブ）、`ScriptPage.lm_report` | `proto/`、`macos/Sources/IkariProto/`、`lm/src/lib.rs`・`scorer.rs`（宣言だけ）、`shell-core/src/types.rs`・`scenario.rs`・`settings/defaults.rs` | なし |
| 1a | A: 計測とエンジン（Sonnet。並行処理なので、2 回つまずいたら Opus） | §4、§5.1・§5.2・§5.4。Rust のテスト | `lm/src/ranker.rs`・`ranker/tests.rs`・`llama.rs`・`scorer.rs`、`engine/src/workers.rs`・`cache.rs`・`convert.rs`・`diag.rs`・`tests/` | 0 |
| 1b | B: shell-core（Sonnet） | §6.1 の設定（parse、values、specs、TSV の作り直し）、§6.2 の写しと文、§7 のシナリオ、`shell-ffi` の写し | `shell-core/src/settings/`・`machine/`・`types.rs`、`shell-core/scenarios/developer-lm-status.json`、`shell-ffi/src/convert.rs`、`macos/Resources/settings-specs.tsv` | 0 |
| 2 | C: Swift（Sonnet） | §6.3。候補窓の 1 行と設定画面の節 | `macos/Sources/IkariIME/CandidatePanel.swift`、`macos/Sources/IkariSettings/`、`macos/Tests/IkariSettingsTests/` | 1b（TSV と ffi の写し） |
| 3 | 統合（リード） | 確認のスクリプト（フル）、実データの性能（変換の時間が変わらないこと）、評価（全セットで 1 位が変わらないこと）、実機で目で見る | なし | 1a、1b、2 |

- 1a と 1b は並行できる。境界は proto と `LmReport` の意味（§5.1）だけで、契約は変えさせない。
- Windows の候補窓（W3）の側は、W3 のタスクに「`developer_note` を描く」を 1 行足す（task-keeper）。
- ADR 0059 の実装（Vulkan）が先に入れば、1a の `LlamaScorer::device` はその選択の結果を返す。後なら、ADR 0059 の実装が返すように直す（どちらの順でも、契約は同じ）。

## 9. ユーザーの判断が要る点（推奨つき）

### 決定済み（ユーザー、2026-11-14）
すべて推奨どおり。
- **Q1: ログはスイッチに関係なく常に書く。** スイッチが切り替えるのは画面の表示だけ。
- **Q2: 「開発者」の節は、変換の画面のいちばん下に置く。**
- **Q3: 時間切れのとき、締め切りのあとに実際にかかった推論の時間は出さない。**
- 表示の文と設定の文言は、実機で見てから直してよい。

### Q1. ログはスイッチに関係なく、常に書くか
- **推奨: 常に書く。** 診断ログは常時オンで、スイッチを作らない決定（ADR 0034）と揃う。中身は数と固定の語だけで、1 行あたり 60 バイトほど増えるだけ（**推測**。ログの回し方は今のまま）。スイッチを見ないので、エンジンに殻の設定を読ませる配線が要らない。不具合の報告のときに、利用者がスイッチを入れていなくても手がかりが残る。
- 代案: `developer.lm_status` がオンのときだけ書く。エンジンも `developer.lm_status` を読む（設定の反映の扱いが増える）。

### Q2. 「開発者」の節をどこに置くか
- **推奨: 変換の画面のいちばん下に「開発者」の節を足す。** LM の締め切りと待ち時間がすでに変換の画面にあり、近くに置ける。タブを増やさないので、Swift の変更が小さい。
- 代案: 新しいタブ「開発者」。開発者向けの項目が今後増えるなら見通しがよいが、今は 1 つだけ（YAGNI）。増えたら移す。

### Q3. 時間切れのとき、実際の推論の時間（締め切りのあとに終わった時間）も出すか
- **推奨: 今回は出さない。** 時間切れの要求はワーカーで続き、終わるのは返事のあと。出すには、遅れて終わった時間を次の変換かログの別の行で知らせる仕組みが要る。締め切りの値を決める掃引は、ikari-eval（`--lm-deadline-ms`）ですでにできる。
- 代案: ワーカーが遅れて終えたときに時間を残し、エンジンの見回りが `lm_late infer_us=` の行を書く（ログだけ）。候補窓には出さない。

表示の文（§6.2）と設定の文言（§6.1）は案。実装のあと、実機で見てから直してよいか、あわせて確かめる。

## 10. 却下した案

| 案 | 却下の理由 |
|----|------------|
| 前後の `LmStats` の差で時間も数える（今の `lm_applied` と同じ形） | 先読みのスレッドの待ちが混ざり、開発者に見せる数が違う値になる（§1.1）。計測のための機能で、数が違うのは困る |
| `Ranker::rank` が結果を返す、または `RankInput` に記録の置き場所を足す | `core` の trait と全部の Ranker（テストのモックを含む）に触る。LM の記録は `lm` の中で閉じられるので、`core` に概念を足す利点がない |
| 判定も、スレッドごとの回数から作る（`lm_applied` を求め直す） | `confident` の閾値を選ぶ値が変わりうる（振る舞いが変わる）。表示と振る舞いの値がずれると、表示の意味がなくなる |
| エンジンがスイッチを見て、オンのときだけ `LmReport` を作る | 数個の数で、作る費用より配線の費用が大きい。殻の設定をエンジンに読ませる口が増える |
| 文を Swift で作る（数だけを渡す） | Windows の候補窓で同じ文を作り直すことになる。表示の判断は Rust（ADR 0042） |
| 候補窓ではなく、メニューや通知で出す | 「変換のときに」見たい、というユーザーの回答に合わない |
| ページ送りと文節のページでも毎回出す | 返事ごとに行が変わり、Space の最初の変換の様子が読みにくくなる。必要になったら `SegmentPageReply` に同じ `LmReport` を足せる（契約は足すだけ） |

## 11. 懸念と前提

- macOS で `Gpu` と出ても、実際に GPU で動いているかの保証は §4.2 の確かめ方に依る（**推測**）。
- `LmReport` は `ConvertReply` に常に入るので、XPC と名前付きパイプの 1 回の返事が 20〜30 バイトほど増える（**推測**）。変換の時間には効かない見込みで、統合で性能を見る。
- 候補窓の高さが 1 行分変わるので、オンのときだけ、窓の位置の決め方（ADR 0014 §2.2）とずれる（§6.2）。
- ADR 0059・0060 は提案のまま（PR #345・#347）。この ADR の番号は、それらのあとの 0061 にした。索引にはまだ 0059・0060 の行がない（それぞれの PR が足す）。

## 参考

- ADR 0004 §3.4、ADR 0014 §2.2、ADR 0021、ADR 0034、ADR 0040 §2.5、ADR 0042、ADR 0058 §1.2、ADR 0059（PR #345）、ADR 0060（PR #347）
- #69（confident）、#159（時間切れ）、#321・#326（設定画面の契約）、#99（配布）
