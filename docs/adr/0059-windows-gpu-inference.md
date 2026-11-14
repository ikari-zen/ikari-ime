# ADR 0059: Windows の LM を GPU（Vulkan）で推論する

- 状態: 提案
- 日付: 2026-11-12
- 対象: `lm/`（バックエンドの読み込み、デバイスの選択）、`stack/`・`engine/`・`eval/`・`windows/engine-host/` の feature、`engine/src/settings.rs`（設定のキー）、`windows/dev/build.ps1`、`.github/workflows/ci.yml`、`docs/windows-real-device-check.md`（項目 38 の追記）
- 前提: main = 60a62be。ADR 0040 §2.6（Windows の LM は任意で既定オフ）・§6 の S4・§7 の W4、ADR 0042（判断は Rust）、ADR 0058（モデルと辞書の置き場所）、#343（`ikari-engine-host` の `lm` feature）。llama-cpp-2 / llama-cpp-sys-2 は 0.1.161（`Cargo.lock`）
- **前提（重要）: 本 ADR に書いた Windows 上の挙動は、誰も Windows で実行して確かめていない。** 事実として書いたものは、llama-cpp-sys-2 / llama-cpp-2 0.1.161 のソースと、このリポジトリのファイルで確かめたものだけ。それ以外は「推測」と書く
- タスク: S4 の続き（issue は task-keeper が切る）
- 判断の置き場所: Rust（`lm` がバックエンドの読み込みとデバイスの選択、`engine` が設定の読み取り）。Swift と TIP（OS の入口）には足さない。GPU の有無は llama.cpp の API で取れ、OS の API や UI である理由がないため

---

## 0. ユーザーの決定（2026-11-12）

1. Windows でも GPU で推論できるようにする。これは機能ではなく**非機能要件（遅延）**。数字は先に決めず、実測のあとに決める。
2. Vulkan は**動的なバックエンド**（`GGML_BACKEND_DL`。llama-cpp-2 の `dynamic-backends`）にする。GPU が無い、または `vulkan-1.dll` が無い機械でも、エンジンが起動して CPU で動くこと。
3. 実装の前に、CPU と Vulkan の遅延を実測する（スパイク）。作者の miniPC（N150。内蔵 GPU の VM へのパススルーは未検証）は「動けば良い」程度。
4. 1.0.0 では LM は任意で既定オフ（ADR 0040）。変えない。GPU の既定も実測のあとに決める。

## 1. 現状（分かったこと）

### 1.1 リポジトリ
- `lm/src/llama.rs` の `load_model` は `n_gpu_layers = if cfg!(feature = "metal") { u32::MAX } else { 0 }`。GPU を使う条件は `metal` だけ。`LlamaBackend::init` は `backend()` の `OnceLock` で 1 回だけ呼ぶ。
- feature の系列は `lm`（CPU）と `lm-metal` が、`lm` → `stack` → `engine` / `eval` に並ぶ。`windows/engine-host` は `lm = ["ikari-engine/lm"]` だけ（#343）。
- `Engine::open`（`engine/src/lib.rs`）は、`settings.tsv` の `engine.lm`（なければ `EngineConfig.enable_lm`）とモデルの照合（`model_check::select_lm_model`）で LM を使うかを決め、LM のないビルドでは使わない。設定のキーは `engine.lm`、`engine.idle_unload_secs`、`engine.lm_deadline_ms`（既定 300、範囲 50〜2000）、`engine.convert_wait_ms`（既定 30）。
- 先行計算の予算は `LmBudget`（`core/src/converter/options.rs`。下限 30ms、1 文字 6ms、上限 300ms）。LM のワーカーはアイドルで解放し、次の要求で読み直す（ADR 0058 §1.2）。→ **読み込みの重さ（GPU なら初期化も）は、起動のときだけでなく、アイドルのあとにも毎回かかる。**
- CI の `windows` job は、最後に `cargo build -p ikari-engine-host --features lm`（実行はしない。モデルがない）。`timeout-minutes: 45`。`windows-artifact` は LM なしの release ビルド。どちらも必須のチェックではない。
- `ikari-eval` のレポートが出す LM の数は、呼び出し回数、時間切れの回数と割合、遅れて届いた結果の利用、変換の warm / cold の時間（`eval/src/report.rs`）。**LM の 1 回の推論の時間そのものは、今の出力には見当たらない**（項目 38 の「1 回の平均と p99」は、今の出力から直接は取れない可能性がある）。

### 1.2 llama-cpp-2 / llama-cpp-sys-2 0.1.161（researcher。ソースで確認）
- `vulkan` feature は、Windows で `VULKAN_SDK` が必須（無いと build script が panic）で、`rustc-link-lib=vulkan-1` を無条件に出す。→ `dynamic-backends` なしでは exe が `vulkan-1.dll` に依存する。無い機械で起動しない可能性が高い（推測。未実行）。
- `dynamic-backends` は `dynamic-link` を含み、`GGML_BACKEND_DL=ON`、`GGML_CPU_ALL_VARIANTS=ON`、`GGML_BACKEND_DIR=$OUT_DIR/backends` を設定する。バックエンド（CPU の各種と ggml-vulkan）は別の DLL として、ビルド機の `$OUT_DIR/backends` に出る。ggml 本体は共有ライブラリになる。DLL を target の下に hard link する処理はあるが、`backends/` の中身を含むかは未確認。
- `load_backends_from_path(dir)`（`dynamic-backends` のときだけ）は、DLL が無い・読めなくても黙って無視する（戻り値なし）。`LlamaBackend::init` の前に呼ぶ。`load_backends()` はコンパイル時の `BACKENDS_DIR`（ビルド機のパス）を読むので、配布では使えない。
- `list_llama_ggml_backend_devices()` で、登録されたデバイスの backend 名と `device_type`（Gpu / IntegratedGpu / Cpu）が見られる。`LlamaModelParams::with_devices` でデバイスを指定できる。
- GPU の初期化に失敗したとき、`load_from_file` が自動で CPU に落ちるか Err になるかは、ソースだけでは確定していない（推測）。前例（myime）は、Vulkan のバックエンドが登録済みだと `n_gpu_layers = 0` でもデバイスが列挙されるので、CPU のときは CPU のデバイスだけを渡している。

### 1.3 速さの見込み（推測）
- zenz small（約 70MB）の短い入力で、Intel の内蔵 GPU が CPU より速いという公開の数字は見つかっていない。前例（myime、azooKey-Windows）は既定を CPU にし、GPU は選ぶ形。
- azooKey 本体の zenz はかな漢字変換そのものを生成する。ikari は辞書の候補を並べ替えるだけなので、1 回の重さは同じではない。ikari の 1 回の重さは未計測。**GPU で速くならない可能性は十分ある**（特に内蔵 GPU。転送と起動の固定費が、短い入力の計算より大きいかもしれない）。

## 2. 決定（案）

### 2.1 非機能要件の枠（数字はスパイクのあと）
枠だけを置く。X・Y・Z・W はスパイクの結果を見てユーザーが決め、本 ADR に追記する。

| # | 要件（枠） | 測り方 | 判定の相手 |
| --- | --- | --- | --- |
| NF-1 | 温まった状態の LM の 1 回が、p99 で X ms 以内 | 締め切りを十分大きくして（時間切れで打ち切らずに）、1 回の推論の時間を測る。CPU と GPU を同じ機械・同じセットで | `LmBudget` の上限（今 300ms）と、読み 1 文字あたりの予算 |
| NF-2 | 既定の予算（`LmBudget` の既定）のもとで、時間切れの割合が Y% 以下 | `ikari-eval` の既定の締め切りでの「時間切れ」の割合と「遅れて届いた結果」 | CPU の同じ値（GPU が CPU より悪くないこと） |
| NF-3 | 冷えた状態（モデルの読み込み、GPU の初期化、シェーダーの準備）の最初の 1 回が Z ms 以内 | エンジンの起動直後と、アイドルの解放（`engine.idle_unload_secs`）のあとの最初の 1 回 | 起動の体感と、アイドル明けの最初の変換で LM が効かない時間 |
| NF-4 | Space から 1 ページ目までの時間（NFR-02、p99 16ms 以下）を悪化させない | `ikari-eval` の warm / cold の変換の時間 | NFR-02。LM は先行計算なので、GPU で悪化しないことの確認 |
| NF-5 | 1 位が悪化しない | 全セットの 1 位（`eval/scripts/snapshot.sh` の手順）。GPU と CPU で数値の誤差により順位が変わりうる | CPU の同じ機械の結果 |
| NF-6 | メモリが W 以内（RSS と、取れるならビデオメモリ） | `ikari-eval` の最大 RSS。ビデオメモリはタスクマネージャーの GPU の欄（手で） | CPU の値 |

- GPU を既定にしてよい条件（形だけ）: NF-1〜NF-2 で CPU より目に見えて良く、NF-3 と NF-6 が許容内で、NF-4・NF-5 が悪化しないこと。どれかが満たせなければ、GPU は「選べるが既定ではない」にとどめる。
- 「LM の 1 回」の時間が `ikari-eval` の今の出力に無ければ（§1.1）、スパイクで測る口を足す。`ikari-eval` に残すかは、スパイクのあとに決める（評価の数字として役に立つなら残す）。

### 2.2 feature: `lm-vulkan` を別に足す（`lm` に混ぜない）
- `lm` crate に `vulkan = ["llama", "llama-cpp-2/vulkan", "llama-cpp-2/dynamic-backends"]`。`stack`・`engine`・`eval` に `lm-vulkan = ["lm", "…/lm-vulkan"]`、`windows/engine-host` に `lm-vulkan = ["ikari-engine/lm-vulkan"]`。既存の `lm-metal` と同じ並べ方にする。
- `metal` と `vulkan` を同時に付けたら `compile_error!` で止める。
- `lm`（CPU だけ）は今のまま静的リンクで、`dynamic-backends` を付けない。理由: `lm` のビルドに Vulkan SDK を要らないままにする（開発、CI、#343 の手順書の項目 37 を変えない）。CPU だけの LM を動的にする利点（CPU の種類ごとの最適化を選べる `GGML_CPU_ALL_VARIANTS`）は、スパイクで CPU の数字が足りないと分かったら検討する。
- `windows/dev/build.ps1` に `-LmVulkan` を足す（`-Lm` とは排他）。`VULKAN_SDK` が無ければビルドの前に止めて、入れ方を出す。

### 2.3 バックエンドの置き場所と読み込み
- **配置**: エンジンの exe の隣の `backends\` に、バックエンドの DLL（CPU の各種、`ggml-vulkan`）を置く。ggml と llama の本体の共有ライブラリは exe と同じフォルダに置く（Windows が exe のフォルダから DLL を探すため）。TIP の DLL の側には置かない（LM はエンジンのプロセスだけが使う）。
- **ビルドからの配置**: build script が `backends\` を target の下に置くかは未確認（§1.2）。置かないなら、`build.ps1`（と、W5 のインストーラ作り）が `$OUT_DIR/backends` から exe の隣の `backends\` に写す。`cargo run -p ikari-eval` で測るときも、同じ形（`target\release\backends\`）にする。
- **読み込み**: `lm` の `backend()` の中で、`LlamaBackend::init` の前に `load_backends_from_path(<exe のフォルダ>\backends)` を 1 回呼ぶ。exe のフォルダは `std::env::current_exe()` から取る。**`load_backends()`（ビルド機のパス）は使わない。** 環境変数などで場所を替える口は作らない（使う場面がない。必要になったら足す）。
- 読み込みは黙って失敗しうるので、読み込みのあとに `list_llama_ggml_backend_devices()` を見て、結果をエンジンのログに 1 行出す（backend 名と種類。例 `lm devices=cpu,vulkan:IntegratedGpu`）。**CPU のデバイスが 1 つも無ければ、その起動は LM なし**（`LM_FAILED`。理由をログへ）。エンジンは落とさない。

### 2.4 デバイスの選択と、CPU への確実な落とし方
- GPU を使わないとき（設定がオフ、または GPU のデバイスが無い）: `with_devices` に **CPU のデバイスだけ**を渡し、`n_gpu_layers = 0`。Vulkan が登録されているだけで GPU に載る経路を断つ（前例の知見）。
- GPU を使うとき: GPU のデバイスを 1 つ選ぶ。種類が Gpu のものを IntegratedGpu より先に、同じ種類なら列挙の最初。`with_devices([その 1 つ])`、`n_gpu_layers = u32::MAX`。複数の GPU から選ぶ設定は作らない（YAGNI）。
- GPU での `load_from_file` が Err なら、その場で CPU のデバイスだけで読み直す。理由をログに出す（`model_check` の照合で落ちたときと同じ扱い）。その起動の間は、以後も CPU で読む（アイドル明けの読み直しで、毎回 GPU を試して失敗しない）。
- 読み込みは通ったが推論が失敗する場合は、既存の LM のエラーの扱いに従う。GPU に特有の失敗が起きるかはスパイクで見る。
- **ドライバの中で落ちる（プロセスが死ぬ）場合は捕まえられない。** 既存の再起動と `runmark`（ADR 0051）の記録に任せる。繰り返し落ちるなら GPU を自動で切る仕組みを足すかは、実際に起きてから決める（§5）。
- macOS: `metal` の経路は変えない（`with_devices` を呼ばず、`u32::MAX`）。新しいコードは `cfg(feature = "vulkan")` の中にだけ置く。

### 2.5 設定
- キー（案）: `engine.lm_gpu`。値は既存の `engine.lm` と同じ真偽の書き方。オンは「GPU があれば使う、なければ CPU」、オフは「CPU だけ」。`engine.lm` がオフなら意味を持たない。
- 「オン／オフ／自動」の 3 値にしない。オンがすでに「自動で CPU に落ちる」ので、3 値目の意味が無い。
- `lm-vulkan` でないビルド（macOS の `lm-metal` を含む）では読み捨てる（macOS は今のまま常に Metal）。
- 既定: スパイクのあとに決める（§6 の (a)）。それまでの実装の既定はオフ。
- 設定画面に出すかは §6 の (b)。

### 2.6 CI
- `windows` job には足さず、**別の job（案 `windows-vulkan`）**にする。理由: `windows` job はすでに LM（CPU）のビルドを含み 45 分の枠がある。Vulkan のビルドは SDK の導入とシェーダーのコンパイルが加わり、時間が延びる見込み（推測。未計測）。並べれば `windows` の結果を待たせない。必須のチェックにはしない（ADR 0040 §2.6 のまま）。
- Vulkan SDK: LunarG の公式インストーラを取得し、無人で入れて（`--accept-licenses --default-answer --confirm-command install`）、`VULKAN_SDK` と `<SDK>\Bin` を `GITHUB_ENV` / `GITHUB_PATH` に書く（前例と同じ）。**版と sha256 を ci.yml の env に固定**し、取得したインストーラを照合してから実行する（`xtask/src/data/fetch.rs` の固定と同じ考え方）。SDK のフォルダは版をキーに `actions/cache` で持つ。
- job の中身: `cargo build -p ikari-engine-host --features lm-vulkan`、`backends\` の配置の確認、デバイスの列挙のテスト（モデルを要らないもの。CPU のデバイスが 1 つ以上あること）。ランナーに GPU は無い見込みなので（推測）、「GPU が無い機械で CPU に落ちる」ことの確認を兼ねる。ランナーに `vulkan-1.dll` があるかは未確認。
- 所要時間は最初の PR で測り、PR の説明に書く。

### 2.7 配布（W5 との関係）
- 配るビルドに LM と Vulkan を入れるかは、スパイクで大きさ（`ggml-vulkan` と CPU の各種の DLL）と、GPU の無い機械での起動を確かめてから決める（§6 の (c)・(f)）。
- `vulkan-1.dll`（Vulkan の実行環境）は同梱しない。GPU のドライバが入れるもので、無ければ CPU で動く（ユーザーの決定 2）。

## 3. スパイク（main に入れない。捨てるブランチと手順書）
- **作るもの**: `lm-vulkan` の最小の実装（§2.2〜§2.4 の読み込みと選択）と、LM の 1 回の時間を出す口（§2.1）。release ビルドを artifact にする（`windows-artifact` と同じ形。`backends\` を含む）。
- **流すもの**（項目 38 を拡張。同じ機械で CPU と GPU を交互に）:
  1. `cargo run --release -p ikari-eval --features lm-vulkan -- --ranker full --set eval\sets\homophones-ext.tsv --lm <モデル> --dict <辞書>` を、GPU オフとオンで（切り替えの口はスパイクで決める）。締め切りは既定と、十分大きい値の 2 通り。
  2. 同じことを `--features lm`（静的な CPU）でも流し、動的な CPU と差が無いかを見る。
  3. エンジンを起動して、最初の 1 回とアイドルの解放のあとの 1 回の時間（NF-3）。
  4. GPU の無い状態（`backends\ggml-vulkan.dll` を外す、`vulkan-1.dll` の無い機械、またはその両方）で、エンジンが起動して CPU で動くこと。
  5. 全セットの 1 位（NF-5）と、DLL の大きさの合計。
- **出すもの**（これがあれば決められる）: §2.1 の NF-1〜NF-6 の表を、機械ごと（CPU の型、GPU の型、ドライバの版、VM か実機か）に CPU と GPU の 2 列で。手順 4 の結果。GPU での読み込みや推論の失敗の有無とログ。
- **機械**: GPU を積んだ Windows が要る（§6 の (e)）。miniPC の VM は、パススルーが動けば追加で 1 回（「動けば良い」）。CI のランナーでは GPU の数字は取れない（手順 4 の一部だけ）。
- **成功**: 手順 4 がすべての機械で通る（エンジンが落ちない）こと。GPU の数字は、速くても遅くても「成功」（決めるための材料）。
- **失敗（設計を見直す）**: `dynamic-backends` のビルドが Windows で通らない、`backends\` から読めない、GPU の無い機械でエンジンが起動しない、CPU のデバイスだけを渡しても GPU に載る。どれかが起きたら、本 ADR に結果を書いて方式を考え直す。

## 4. 却下した案

| 案 | 却下の理由 |
| --- | --- |
| 直接リンク（`vulkan` だけ。`vulkan-1` を exe の依存にする） | `vulkan-1.dll` の無い機械で exe が起動しない見込み（§1.2）。ユーザーの決定 2 に反する。LM をオフにしていても起動できなくなる |
| azooKey-Windows の方式（backend ごとに DLL のフォルダを分け、起動役が PATH で選ぶ） | 切り替えにエンジンの再起動と、起動役（判断）が要る。PATH で DLL を選ぶのは、探索の順に頼り、W6 で見る DLL の差し込みの面を広げる。動的バックエンドなら 1 つの配置でプロセスの中で選べる |
| CUDA | NVIDIA の GPU だけ。ビルドに CUDA Toolkit が要り、CI と配布物が重くなる。内蔵 GPU の機械（作者の miniPC を含む）で使えない。Vulkan で足りないと分かってから、同じ `backends\` に足す形で検討できる |
| DirectML | llama.cpp のバックエンドに無い（researcher の調査の範囲で見つからない）。推論の層を替えることになり、macOS と共通の llama.cpp の経路から外れる |
| GPU にしない（CPU だけ） | ユーザーの決定 1 に反する。ただし、スパイクで GPU が速くならなければ、GPU の既定はオフのままになり、結果としてこれに近くなる（それは実測で決める） |
| `lm` そのものを `dynamic-backends` にする（feature を分けない） | `lm` のビルドに Vulkan SDK が要るようになり、CPU だけの開発と CI（#343）が重くなる。CPU だけの利用者にも DLL の束を配ることになる |
| 設定を 3 値（オン／オフ／自動） | §2.5。オンがすでに自動で落ちる |

## 5. 範囲外と懸念

- 範囲外: macOS の変更（Metal の挙動はそのまま）。候補の生成（制約つきのやり直し。ロードマップの検討中の項目）。モデルの追加や置き換え。複数の GPU の選択。
- 懸念:
  - 内蔵 GPU で速くならない可能性（§1.3）。その場合、作った仕組みは「選べるが使われない」ものになる。スパイクで分かるので、本実装はスパイクの結果を見てから始める。
  - アイドルの解放のたびに GPU の初期化がかかる（§1.1）。NF-3 が大きければ、GPU のときだけ解放の時間を変えるかを検討する（値は計測で決める）。
  - ドライバの中で落ちる場合は捕まえられない（§2.4）。
  - `backends\` から DLL を読むのは、エンジンのフォルダに書ける者が DLL を差し込める面になる。インストール先の権限と合わせて W6 で見る。
  - llama-cpp-2 の版を上げると、`dynamic-backends` の挙動（配置、名前）が変わりうる。上げるときは `windows-vulkan` の job で確かめる。
  - GPU の数値の誤差で、CPU と 1 位が変わりうる（NF-5）。学習と評価の再現性に効く。

## 6. ユーザーの判断が要る点（推奨つき）

### 決定済み（ユーザー、2026-11-14）

- **(c) 配布物の大きさは気にしない**（上限は設けない）。
- **(e) スパイクの機械は、作者の miniPC（N150。内蔵 GPU）だけ**でよい。ほかに自由に使える機械はない（会社の PC は私用に使えない）。N150 は遅いので、性能の数字は**参考値として記録する**にとどめ、詰めない。
- **上の 2 点以外（(a)(b)(d)(f)(g)）は推奨どおり。**
- **GPU を使う（ユーザー決定、2026-11-14）。** 冷えた最初の 1 回が約 18 秒かかるのは、VM か N150 の遅さのせいかもしれず、これ以上は追わない。もっと高性能な機械なら性能は出るはず。**暖機（モデルの読み込みと GPU の初期化、シェーダーの準備）は、エンジンの起動のときにやる。** これにより (a) は、スパイクの結果を受けて「GPU を使う（自動で CPU に落ちる）」に進める。LM は 1.0.0 では既定オフのまま（ADR 0040）なので、効くのは LM をオンにした人だけ。
- **GPU の既定はオン（ユーザー決定、2026-11-14）。** 設定 `engine.lm_gpu` の既定は真。GPU が使えない機械（GPU も `vulkan-1.dll` も無い、初期化に失敗）では、自動で CPU に落ちる。オフにしたい人は `engine.lm_gpu = 0`。
- **アイドル解放後の読み直し（ユーザー決定、2026-11-14）: 解放までの時間を、GPU に載せているときだけ長くする。** 「GPU で読み直すのに 18 秒かかるのは N150 のせいだろう」。CPU のときの既定は今のまま（300 秒）。ユーザーが `engine.idle_unload_secs` を明示したときは、それを優先。GPU のときの既定は **1800 秒（30 分）**（計測なしの暫定値。ユーザーが了承）。実装は PR #367。
- スパイクの実測（作者の N150 の VM（2 vCPU）、内蔵 GPU、zenz small Q5_K_M、homophones-ext 1266 件、`--lm-budget-ms 5000 --lm-deadline-ms 5000`、同じビルドで `IKARI_LM_GPU` だけ変える。**参考値**）:

| | CPU | GPU（Vulkan、内蔵） |
| --- | --- | --- |
| 推論 1 回 p50 / p95 / p99（n≈2530） | 433 / 596 / 906 ms | 175 / 191 / 210 ms |
| モデルの読み込み | 0.8 秒 | 12 秒 |
| 最初の 1 回（暖機） | 0.1 秒 | 6 秒 |
| 評価の全体の時間 | 1176 秒 | 466 秒 |
| 1 位 / MRR（NF-5） | 88.0%（1114 件）/ 0.933 | 88.5%（1120 件）/ 0.936 |

  読み: 温まったあとの推論は GPU が約 2.5 倍速く、p99 は約 4 倍良い。一方、冷えた最初の 1 回は GPU が約 18 秒かかる（CPU は約 1 秒）。アイドルの解放（§1.1）のたびにこれがかかるので、NF-3 が実際の論点になる。1 位は 6 件の差（GPU の誤差の範囲かは、全セットでの確認が要る）。`ggml-vulkan.dll` を含む配布物の大きさは、ユーザー決定で気にしない。

### 推奨と選択肢（記録）

- **(a) GPU の既定**: 推奨は「スパイクまではオフ。スパイクの後、§2.1 の条件を満たした機械の種類があればオン（自動で CPU に落ちる）を検討」。LM そのものが既定オフなので、GPU の既定が効くのは LM をオンにした人だけ。
- **(b) 設定のキーと画面**: 推奨はキー `engine.lm_gpu`（真偽）。画面には、LM のオンオフの近くに 1 つだけ出す（W4-s の一覧に足す）。スパイクで GPU が速くならなければ、画面に出さず設定ファイルだけにする。
- **(c) 配布物の大きさ**: 推奨は「スパイクで大きさを測ってから決める」。許容の上限をユーザーが決める。
- **(d) Vulkan だけで始めるか**: 推奨は Vulkan だけ。CUDA は、Vulkan で NVIDIA の数字が足りないと分かってから。
- **(e) スパイクの機械**: 推奨は、GPU を積んだ Windows の実機を 1 台（どの機械を使えるかはユーザーに聞く。単体の GPU と内蔵 GPU の両方があれば理想）。miniPC の VM は任意。
- **(f) 配るビルド**: 推奨は、配るビルドを 1 つにして `lm-vulkan` を入れる（LM は既定オフのまま、使う人は設定でオンにするだけ）。大きさ（c）が許容を超えるなら、LM なしだけを配る。
- **(g) 実装の順**: 推奨は、スパイクの結果を本 ADR に追記し、ユーザーが §2.1 の数字と (a) を決めてから、§2.2〜§2.6 を本実装する（スパイクで GPU が速くなくても、§2.3〜§2.4 の CPU への落とし方と CI は入れるかを、そこで決める）。

## 参考
- [ADR 0040](0040-windows-feasibility.md) §2.6、§6 の S4、§7 の W4
- [ADR 0042](0042-technology-policy.md)
- [ADR 0058](0058-offline-resource-install.md) §1.2
- [実機の確認の手順書](../windows-real-device-check.md) 項目 37・38
- 前例: myime（unok/myime、MIT。Vulkan を動的に読み、既定は CPU）、azooKey-Windows（backend ごとの DLL のフォルダ）
