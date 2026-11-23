# ADR 0081: Windows のインストーラに VC++ の実行時ライブラリ（vc_redist）を同梱するか、不要にできるか

- 状態: 提案（2026-11-23 調査・ADR 案。決定はユーザー）
- 日付: 2026-11-23
- 対象: `windows/installer/ikari.iss`、`.github/workflows/release.yml`、`xtask/src/scan.rs`、`xtask/src/scan/stage.rs`、`xtask/src/check/installer.rs`、`lm/`、`windows/candidate-ui/`、`windows/settings-ui/`
- 前提: [ADR 0067](0067-windows-install-layout-and-launch-chain.md) §3.3・§4.4・§4.5、[ADR 0069](0069-distribution-authenticity.md) §4.1、[ADR 0070](0070-update-and-vulnerability-response.md) §6、[ADR 0072](0072-lm-cage-levels.md) §5
- 判断の置き場所: ビルド設定と検査（`scan-product`・静的検査）は Rust（`xtask`）と workflow、導入条件と実行順は Inno Setup（`windows/installer/ikari.iss`）。本 ADR 案ではコードや設定は変えない
- 関連: #572、#567（PR #570）、#615、#99、[publish-checklist.md](../publish-checklist.md) §4

## 1. コンテキスト（#572 の問い）

Windows 版を他の利用者の PC に配布する（#99、#567）にあたり、VC++ の実行時ライブラリ（Visual C++ 再頒布可能パッケージ、`vc_redist`）が入っていない素の Windows 環境でも安全に動作させる必要がある。

[ADR 0067](0067-windows-install-layout-and-launch-chain.md) §4.5（2026-11-17）では、いったん **「(a) `vc_redist` を sha256 と Microsoft 署名者の確認付きで同梱して入れる、加えて `/DEPENDENTLOADFLAG:0x1000`（要検証）と `scan-product` の LM の子許可リストを足す」** 決定が記録された。一方、現時点の [release.yml](../../.github/workflows/release.yml)（21 行目）および [ikari.iss](../../windows/installer/ikari.iss) には `vc_redist` がまだ入っておらず、実機（VM）での確認も未実施である（#572）。

本調査では、現在の実装（`crt-static`、`scan-product --stage`、`ikari.iss`、.NET / WinUI 3 の発行設定）を**確認できた事実**と**未確認の推測**に切り分けて整理し、今後の検証手順と採否の選択肢（ADR 案）を示す。

---

## 2. 現状（コードで確認できた事実）

### 2.1 Rust 製の製品バイナリ（`crt-static` が有効な 6 本）

- [.cargo/config.toml](../../.cargo/config.toml)（8〜9 行目）は `[target.x86_64-pc-windows-msvc]` に対して `rustflags = ["-C", "target-feature=+crt-static"]` を設定している。
- [.github/workflows/release.yml](../../.github/workflows/release.yml)（129 行目）でも、`RUSTFLAGS` 上書き時に `.cargo/config.toml` が無視されないよう `-C target-feature=+crt-static` を明示している。
- [xtask/src/check/product.rs](../../xtask/src/check/product.rs) が `.cargo/config.toml` と、Windows のビルド機（`USERPROFILE` を含む `RUSTFLAGS` 行）の両方に `target-feature=+crt-static` が含まれることを PR ごとに静的検査している。
- [xtask/src/scan/stage.rs](../../xtask/src/scan/stage.rs)（18〜26 行目の `PRODUCT_FILES`）は、次の 6 本に対して [xtask/src/scan.rs](../../xtask/src/scan.rs) の `check_bytes`（開発用の文字列、llama.cpp 印、測定ツール印、通信 DLL、動的 CRT の DLL 名の走査）を実行する（`release.yml` 257 行目）。
  1. `ikari-engine-host.exe`
  2. `ikari-setup-helper.exe`
  3. `ikari-data.exe`（通信 DLL 検査のみ名前で除外）
  4. `tip/ikari_tip.dll`
  5. `candidate/ikari_candidate.dll`
  6. `settings/ikari_settings_core.dll`
- `xtask/src/scan.rs`（43〜51 行目の `DYNAMIC_CRT`）が拒否する動的 CRT の前置きは `vcruntime`、`msvcp`、`msvcr`、`ucrtbase`、`api-ms-win-crt-`、`concrt`、`vcomp`（`.dll` で終わるトークンを検出）である。よって上記 6 本はビルド時に動的 CRT へ依存しない前提が検査で守られている。

### 2.2 LM の子（`ikari-lm-host.exe`）と `llama.cpp` / `ggml` DLL

- `release.yml`（188 行目）は `cargo build --release --locked -p ikari-lm --bin ikari-lm-host --features vulkan` でビルドする。
- [lm/Cargo.toml](../../lm/Cargo.toml)（15 行目）で `vulkan = ["llama", "llama-cpp-2/vulkan", "llama-cpp-2/dynamic-backends"]` が定義されており、`llama*.dll`、`ggml*.dll`、および `backends\*.dll`（`ggml-vulkan*.dll` 等）が動的ライブラリとして stage 直下および `backends\` にコピーされる（`release.yml` 233〜234 行目）。
- 非 Vulkan の静的ライブラリ構成（`--features llama`）について、[.github/workflows/ci.yml](../../.github/workflows/ci.yml)（114〜120 行目）および [ADR 0067](0067-windows-install-layout-and-launch-chain.md) §4.4 追記に、`llama.cpp` が動的 CRT（`/MD`）でビルドされ `LLAMA_STATIC_CRT=1` でも `/MT` にならずリンクが失敗したため、`RUSTFLAGS="-C target-feature=-crt-static"` でビルドする経緯が記録されている。また ADR 0067 §4.5 には `VCOMP140.dll`（OpenMP）に静的版が無いことも記されている。
- 最新の `xtask/src/scan/stage.rs`（#615 反映後）における検査の扱い:
  - `check_lm_child`（115〜131 行目）は、開発用の文字列・測定ツールの印・通信 DLL だけを検査しており、`find_dynamic_crt` と `find_llama` の対象から外している（5 行目の注記でも、LM の子は動的な C ランタイムを持ってよい扱い）。
  - ただし `stage.rs` の 10〜11 行目のとおり、他社の部品（`llama*.dll`、`ggml*.dll`、`backends\*.dll`、.NET / Windows App SDK 一式）のファイル中身自体は検査していない。
  - さらに `TOP_LEVEL_DLL_PREFIXES = &["llama", "ggml"]`（`stage.rs` 53 行目・150〜158 行目）により、stage 直下の DLL は `llama` または `ggml` で始まる名前だけしか許されていない（たとえば `vcruntime140.dll` 等を直下に置く構成は現在の `scan-product --stage` が拒否する）。
  - 同様に `TOP_LEVEL_EXES`（46〜51 行目）は `ikari-engine-host.exe`、`ikari-setup-helper.exe`、`ikari-data.exe`、`ikari-lm-host.exe` の 4 本だけを許している。
- 起動時の防御として、`lm/src/host/os.rs`（338〜356 行目）と `windows/engine-host/src/run.rs`（58〜78 行目）は `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)`（`0x200 | 0x800`）を呼んでいる。これはプロセス起動後の動的な `LoadLibrary` 等に効くが、EXE 本体の PE import 表による起動前の暗黙 DLL 読み込みを直接は制御しない。

### 2.3 候補窓・設定画面（.NET 10 / WinUI 3）

- [IkariCandidate.csproj](../../windows/candidate-ui/IkariCandidate.csproj) および [IkariSettings.csproj](../../windows/settings-ui/IkariSettings.csproj) はいずれも以下を指定して発行される（`release.yml` 243 行目、`dotnet publish -c Release ...`）。
  - `TargetFramework`: `net10.0-windows10.0.19041.0`
  - `RuntimeIdentifier`: `win-x64`
  - `UseWinUI`: `true`、`Microsoft.WindowsAppSDK` `2.6.5`
  - `WindowsPackageType`: `None`、`WindowsAppSDKSelfContained`: `true`、`SelfContained`: `true`
- これらは自己完結発行であり、.NET ランタイムおよび Windows App SDK の一式が `candidate\`・`settings\` 配下に出力されるが、その中に含まれるネイティブランタイムや WinUI 3 の DLL がシステム側の VC++ ランタイム（`vcruntime140.dll` 等）を要求するかはリポジトリ内で自動検査されていない。

### 2.4 インストーラ（`ikari.iss` と静的検査）の制約

- [ikari.iss](../../windows/installer/ikari.iss) は管理者権限（`PrivilegesRequired=admin`）で動き、インストール先は `{commonpf64}\ikari-ime` 固定、作業用フォルダは Program Files 直下の `{commonpf64}\ikari-ime.setup`（`{code:SetupWorkDir}`）に作成する（171〜179・293〜301 行目）。
- [xtask/src/check/installer.rs](../../xtask/src/check/installer.rs) は以下を検査して禁止・制限している。
  - `{tmp}`・`ExtractTemporaryFile`・`{localappdata}`・`GetEnv(`・`powershell`・`cmd.exe`・`ShellExec` 等の使用禁止（44〜63 行目）。
  - `[Files]` の配置先は `{app}` または `{code:SetupWorkDir}` のみ、`{app}` 配下はすべて `Check: AppDirReady` または `TipNeedsCopy` が必須（80〜82 行目）。
  - `Exec(` は `RunTrusted` 内の 1 か所のみ、`RunTrusted` の起動対象は `SysTool(`、`WorkHelper`、`AppHelper`、`Helper` のみに制限（84 行目・355〜373 行目）。
- そのため、外部の `vc_redist.x64.exe` を同梱・実行する場合は、stage の `TOP_LEVEL_EXES` や `ikari.iss` / `installer.rs` の許可規則（作業用フォルダ配置・信頼済み実行対象）を設計どおりに広げる作業が必要になる。

---

## 3. 未確認の事実と推測

以下は現時点のコードと Mac 上の調査からは確定できておらず、Windows ランナーまたは VM での確認が要る項目である。

1. **各成果物の実際の PE import 表と最小要求ランタイム版（未確認）**:
   - `release.yml` の `build-windows` が出力する実際のバイナリ（`ikari-lm-host.exe`、`llama*.dll`、`ggml*.dll`、`backends\*.dll`、`candidate\` および `settings\` 配下の `.exe`/`.dll`、`PRODUCT_FILES` 6 本）について、`dumpbin /dependents`（または `llvm-objdump -p`）で VC++ ランタイム（`vcruntime140*.dll`、`msvcp140*.dll`、`vcomp140*.dll`、UCRT 等）への実際の依存有無と要求版をまだ計測していない。
   - なお `xtask/src/scan.rs` の `find_dll_names` は PE ヘッダの import 表パーサではなくバイナリ中の ASCII トークン検索である（`scan.rs` 6 行目にも「PE の import 表の解析は 2 歩目」と記載がある）。
2. **VC++ ランタイム未導入のクリーン Windows VM での起動挙動（未確認）**:
   - Microsoft Visual C++ 2015–2022 再頒布可能パッケージ（x64）が入っていない素の Windows 10 / 11 VM で、(a) TIP・`ikari-engine-host.exe`、(b) 自己完結発行した `ikari-candidate.exe` / `ikari-settings.exe`、(c) `ikari-lm-host.exe` がそれぞれ起動できるか・どう失敗するか（エラーダイアログが出るか、終了コードのみで終わるか等）をまだ試していない。
   - `.NET 10` / `WindowsAppSDK 2.6.5` の `SelfContained=true` 出力が `vcruntime140*.dll` 等を自身の出力フォルダに含むか、OS 側の導入を期待するかも未確認である。
3. **未確認: `/DEPENDENTLOADFLAG` の定数と隣接 DLL 読み込みの両立**:
   - ADR 0067 §4.5 は `ikari-lm-host.exe` と `llama` の DLL にリンカオプション `/DEPENDENTLOADFLAG:0x1000`（System32 のみ検索）を付けるとし、「隣の DLL を読む形と両立するか VM で確かめる」としている。
   - **コードで確認できた事実**: `windows-sys`（0.61.2）の Win32 定数は `LOAD_LIBRARY_SEARCH_APPLICATION_DIR = 0x200`、`LOAD_LIBRARY_SEARCH_SYSTEM32 = 0x800`、`LOAD_LIBRARY_SEARCH_DEFAULT_DIRS = 0x1000` である（`lm/src/host/os.rs` 339〜340 行目でも `0x200 | 0x800` を使用）。すなわち `0x800` が System32 のみ、`0x1000` が `DEFAULT_DIRS`（アプリケーションディレクトリ + User dirs + System32 の組み合わせ）に対応する。
   - **未確認（Windows ビルド機・VM で検証が必要）**: (a) `0x800`（`LOAD_LIBRARY_SEARCH_SYSTEM32`）を付けた場合に同フォルダの `llama*.dll`・`ggml*.dll` の暗黙 import が失敗しないか、(b) MSVC リンカの `/DEPENDENTLOADFLAG` として `0x1000`（`DEFAULT_DIRS`）や `0xA00`（`0x200 | 0x800`）のどれが有効に受け付けられ、隣接 DLL と両立しつつ PATH 探索を止められるか、を実際にリンクして VM で検証する必要がある。
4. **推測: `vc_redist.x64.exe` 内部の一時展開先（未確認）**:
   - **推測（未確認）**: `ikari.iss` 自身が `{tmp}` を使わず `{commonpf64}\ikari-ime.setup` から実行しても、Microsoft 公式の `vc_redist.x64.exe` ブートストラッパ自体は実行時に内部展開用の一時ファイルを `%TEMP%` 等に書き出す可能性がある。標準ユーザーが管理者の資格情報を入れて昇格実行した場合に、標準ユーザーが書ける場所へ一時ファイルが置かれるリスク（ADR 0066 P1、`docs/windows-real-device-check.md` 項目 55 と同種の論点）があるかは Process Monitor で確認していない。
5. **未確認: 個別 DLL 同梱（アプリローカル配置）の再頒布要件とビルド再現性**:
   - もし `vc_redist.x64.exe` を使わず必要なランタイム DLL だけを `C:\Program Files\ikari-ime\` へ同梱する場合、GitHub ランナーの MSVC ツールチェーンから DLL を取り出す方法が ADR 0069 §4.1（版と sha256 の固定、または出所の固定）および Visual Studio 再頒布ライセンス条項と整合するかを確認していない。

---

## 4. 検討した案（決定はユーザー）

### 案 A: `vc_redist.x64.exe` を同梱して条件付きで導入する（ADR 0067 §4.5 維持・推奨候補）

- **内容**:
  1. `release.yml` で版と `sha256` を固定して `vc_redist.x64.exe` を取得し、Authenticode の署名者（Microsoft）を検証する。ステージ検査（`PRODUCT_FILES` 等）とは分離した入力としてインストーラ作成に渡す。
  2. `ikari.iss` は `vc_redist.x64.exe` を作業用フォルダ（`{code:SetupWorkDir}`、管理者のみ書き込み可）に配置し、既存の導入状態・版（`HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64` の `Installed` および `Major` / `Minor` / `Bld` 等）を確認して、未導入または同梱版より古いときだけ `/quiet /norestart` で実行する。
  3. 失敗しても IME 自体の導入・登録は継続し、LM なしで動く仕様を保つ（ADR 0067 §4.5 の 2・ADR 0072 §5）。アンインストール時には削除しない（同 3）。
  4. `xtask/src/check/installer.rs` には信頼済み実行対象・検証手順に対応する検査を足し、`xtask/src/scan.rs`・`stage.rs` には `ikari-lm-host.exe` が取り込んでよい DLL（OS 標準 + VC++ ランタイム + 同梱する `llama`/`ggml`）の許可リスト検査を追加する。
  5. `/DEPENDENTLOADFLAG` はステップ 2 の VM 検証の結果を踏まえ、隣接 DLL と両立する形で適用可否を決める。
- **利点**:
  - ランタイムのセキュリティ修正が Windows Update 等（OS/Microsoft の更新経路）で反映され、ikari 側で単体ランタイム DLL の脆弱性更新を追い続けなくて済む。
  - `ikari-lm-host.exe` だけでなく、仮に WinUI 3 / `.NET` の自己完結ビルド側がシステム VC++ ランタイムを暗黙要求する場合も同時に満たせる。
  - ADR 0067 §4.5 の既存決定・ADR 0067 §3.3 の「通信せず同梱する」方針と整合する。
- **欠点・リスク**:
  - インストーラのファイルサイズが増える（実サイズは未計測）。
  - `vc_redist.x64.exe` の静かな終了時の再起動要求コード（`3010` 等）や失敗時の扱い、`vc_redist` 自身の `%TEMP%` 展開挙動の検証が必要。

### 案 B: 必要な VC++ ランタイム DLL をインストール先（`C:\Program Files\ikari-ime\`）へアプリローカル同梱し、`vc_redist.x64.exe` を使わない

- **内容**:
  - ステップ 1（PE 依存の実測）で必要と判明した VC++ ランタイム DLL（例: `vcruntime140*.dll`、`msvcp140*.dll`、`vcomp140*.dll` 等）を `C:\Program Files\ikari-ime\`（管理者のみ書き込み可）に同梱し、外部インストーラを起動しない。
  - `xtask/src/scan/stage.rs` の `TOP_LEVEL_DLL_PREFIXES` に該当 DLL を追加して許可する。
- **利点**:
  - 外部インストーラ（`vc_redist.x64.exe`）を実行しないため、ブートストラッパによる `%TEMP%` 展開の懸念や、システム共有状態の変更・再起動要求コードの扱いが不要。アンインストールで綺麗に消える。
  - 必要な最小 DLL だけを入れるためサイズ増加を抑えやすい（正確な増分は未計測）。
- **欠点・リスク**:
  - VC++ ランタイムに脆弱性が見つかった際、Windows Update では更新されず、ikari 自身が版を上げて再配布する責任を負う（ADR 0070 §6 の運用負担増）。
  - ステップ 1 で WinUI 3 / `.NET`（`candidate\`、`settings\`）もシステム VC++ ランタイムを要求することが分かった場合、それらのサブフォルダまたは検索パスにも配慮が必要になる。
  - ランナーの MSVC から DLL を拾う場合、ADR 0069 §4.1 の「道具と版の固定・確認」や Visual Studio の再頒布条件（ライセンス・sha256 管理）の整理が必要。
  - System32 のみに限る探索フラグ（`/DEPENDENTLOADFLAG:0x800`）はアプリローカル DLL と両立しない可能性が高く、アプリディレクトリを許す形（`0x1000` 等）が有効かを別に検証する必要がある。

### 案 C: `llama.cpp` を完全静的リンク＋OpenMP 無効化等でビルドし、VC++ ランタイムを不要にする

- **内容**:
  - `ikari-lm-host.exe` について CMake / MSVC オプションを見直し、OpenMP（`vcomp140.dll`）無効化や完全 `/MT` 化によって動的 CRT 依存をゼロにできるか試す。
- **利点**:
  - もしアプリ全体で動的 CRT 依存を無くせれば、`vc_redist` の同梱もアプリローカル DLL も不要となり、全バイナリを `scan-product` の `+crt-static` 検査で統一できる。
- **欠点・リスク**:
  - 既知の事実として過去に `LLAMA_STATIC_CRT=1` でも `/MT` にならず、`VCOMP140.dll` に静的版が無い（ADR 0067 §4.4・§4.5）。
  - さらに現在の製品構成（`release.yml` 188 行目）は GPU 対応のため `--features vulkan`（`dynamic-backends`）を採っており、`llama*.dll`・`ggml*.dll`・`backends\*.dll` という複数 DLL を読み込む。EXE 側だけでなくこれら全 DLL を一貫して動的 CRT なしにするか静的化する必要があり、DLL 境界でヒープ/ランタイムを跨ぐ設計変更が大きくなる。
  - **仮に `ikari-lm-host.exe` 側を解決しても、WinUI 3（`Microsoft.WindowsAppSDK`）や `.NET 10` の自己完結成果物がシステム VC++ ランタイムに依存している場合は、製品全体としては VC++ ランタイム不要にならない。**
  - OpenMP 無効化時は CPU フォールバック時の推論速度への影響（未計測）も評価し直す必要がある。

### 案 D: 何もしない（放置）

- **採らない理由（ADR 0067 §4.5 の決定どおり却下）**:
  - VC++ ランタイム未導入の PC で LM が動かなくなるだけでなく、必要なランタイム DLL が見つからない場合に検索パスの先（PATH 上の利用者が書けるディレクトリ等）から同名 DLL を読み込まれるセキュリティリスク（ADR 0067 §4.5 の指摘）を残すため採らない。

---

## 5. 確かめ方とユーザーに聞くこと

### 5.1 実装前に行う確認手順（リード / Windows ランナー・VM）

1. **ステップ 1（PE 依存の実測 — Windows ランナー / `workflow_dispatch`）**:
   - `release.yml` の `build-windows`（`workflow_dispatch`）で出力された stage 一式に対し、`dumpbin /dependents`（または `llvm-objdump -p`）を以下に実行し、取り込む DLL 一覧（特に VC++ ランタイム系 DLL の有無と要求版）を記録する。
     - `PRODUCT_FILES` 6 本（`+crt-static` の実バイナリ確認、`docs/windows-real-device-check.md` 項目 45 (b) の拡張）
     - `ikari-lm-host.exe`、`llama*.dll`、`ggml*.dll`、`backends\*.dll`
     - `candidate\` と `settings\` 配下の `.exe` および主要ネイティブ `.dll`
2. **ステップ 2（VC++ 未導入クリーン VM での挙動確認）**:
   - VC++ 再頒布可能パッケージ（2015–2022 x64）が未導入のスナップショットを用意し、(a) TIP・`ikari-engine-host`、(b) 候補窓・設定画面（.NET 10 / Windows App SDK 2.6.5）、(c) `ikari-lm-host.exe` の起動結果を確かめる。
   - あわせて `ikari-lm-host.exe` に `/DEPENDENTLOADFLAG` を変えてビルドしたテスト（`0x1000` 等）で隣接 `llama*.dll` の読み込み成否を確かめる。
3. **ステップ 3（決定案の実装後確認 — VM）**:
   - [windows-real-device-check.md](../windows-real-device-check.md) §5.9 のインストーラ項目と合わせて、クリーン VM で (a) LM あり変換の成功、(b) 新規・上書き（既存より新/同/旧版）・失敗時フォールバック、(c) アンインストール後の状態（案 A なら `vc_redist` 残存）を確かめる。

### 5.2 ユーザーに聞くこと（決定はユーザー）

- **Q1. 方針の採否**:
  - [ADR 0067](0067-windows-install-layout-and-launch-chain.md) §4.5 の決定どおり **案 A（`vc_redist.x64.exe` 同梱）** をそのまま実装に進めるか。
  - それとも、先にステップ 1（PE 依存調査）を実施して `.NET`/WinUI 3 がシステム VC++ を要求しないか確認したうえで **案 B（アプリローカル DLL 同梱）** と比較してから最終決定するか。
- **Q2. 案 A を採る場合のセキュリティ制約の調整**:
  - `ikari.iss` と `xtask/src/check/installer.rs` について、`{code:SetupWorkDir}\vc_redist.x64.exe` の配置および `RunTrusted` からの実行（`release.yml` での sha256 + Microsoft 署名者検証付き）を許可リストへ追加してよいか。
  - また、`vc_redist.x64.exe` 自身が実行中に `%TEMP%` を参照しうる点について、Process Monitor で実挙動を確認したうえで許容するか（または案 B へ切り替える判断基準を置くか）。
- **Q3. 着手順序**:
  - 実装 PR を切る前に、まずステップ 1（Windows ランナーでの `dumpbin /dependents` 一覧化）とステップ 2（クリーン VM での未導入挙動・`/DEPENDENTLOADFLAG` の確認）を先行させるか。

---

## 参考

- [ADR 0067: Windows の置き場所・登録・起動の連鎖](0067-windows-install-layout-and-launch-chain.md) §3.3・§4.4・§4.5
- [ADR 0069: 配る物の真正性](0069-distribution-authenticity.md) §4.1
- [ADR 0070: 更新と脆弱性対応の運用](0070-update-and-vulnerability-response.md) §6
- [docs/publish-checklist.md](../publish-checklist.md) §4
- [docs/windows-real-device-check.md](../windows-real-device-check.md) 項目 45・§5.9
- `windows/installer/ikari.iss`、`.github/workflows/release.yml`、`xtask/src/scan.rs`、`xtask/src/scan/stage.rs`、`xtask/src/check/installer.rs`、`xtask/src/check/product.rs`
- 関連する Issue: #572、#567（PR #570）、#615、#99
