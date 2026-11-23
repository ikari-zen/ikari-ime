# Windows の実機で ikari を通しで試す手順書

Windows 版の ikari（TIP、常駐のエンジン、名前付きパイプ）は、Mac では型と OS に依存しない部分までしか確かめられません。この文書は、Windows 11 の VM で通しで動かし、「実機の確認待ち」を閉じるための手順です（[ADR 0040](adr/0040-windows-feasibility.md) §2.1、§6 の S0〜S11、§7 の W5。issue #331）。

**この文書とスクリプトは、2026-11-14 に Windows 11 の VM（N150 の 2 vCPU）で動かし始めました。** 最初の実行で出た失敗や手順の抜けは直してあります（PR #350 など）。まだ動かしていない項目や、手順が変わりうる箇所は、各項目の結果と「結果の記録」を見てください。

- 対象: 開発用の手順です。配る人向けのインストーラではありません（W5 の残りで作ります）。
- ビルド: CI の artifact と `windows/dev/build.ps1` は、開発用の口（`dev-knobs`。`IKARI_USER_DIR` などの環境変数、計測の印、平らな配置、エンジンの `--data-dir`、デバッグの出力）つきのビルドです。この文書の手順はそれを前提にします。配る製品のビルドには、どれも入りません（ADR 0067 §6）。
- 範囲: LM なし、x64、Windows 11 のデスクトップのアプリ（[ADR 0040](adr/0040-windows-feasibility.md) §2.3）。項目 1〜46 は、候補窓と設定画面が無い時点の確認です（確認の対象は「変換して確定できるか」まで。項目 14）。候補窓（W3）と設定画面（W4-s3）は、#441 で入り、確認は項目 56〜68 にあります。
- 実機で見ないと分からないことは、事実として書いていません。「期待する結果」は、コードと PR の説明から読み取った**見込み**です。違っていたら、それが発見です。

## 1. 準備

### 1.1 VM

- 別のホストの KVM 上の x64 の Windows 11 Pro の VM を使いました（[ADR 0040](adr/0040-windows-feasibility.md) §2.6）。Mac からは `ssh win11` で入ります（鍵認証）。**TIP は全アプリに入る DLL なので、不具合でアプリや Windows が固まることがあります。実機（ふだん使う Windows）には入れません。**
- 基準の状態（準備だけ済み、ikari は未登録）を取っておき、固まったら戻します。実際の VM では、ホスト側の差分ディスク（オーバーレイ）で基準を持ち、基準へ戻す道具を使いました。ゲストの中の変更は、戻すとすべて消えます。VM のスナップショットを使う場合も、考え方は同じです。
- 管理者になれるユーザーで使いました。項目 10（別のユーザーからの接続）のための標準ユーザーは、まだ確かめられていません。2026-11-14 の時点で、`Start-Process -Credential` で別のユーザーを起動すると `0xC0000142` で失敗し、保留にしています。
- ネットワークが要るのは、道具の取得と、Mac からのデータの持ち込みのときだけです。

### 1.2 道具

| 道具 | 何のため | メモ |
| --- | --- | --- |
| Rust（`rustup`。`stable-x86_64-pc-windows-msvc`） | ビルド | `rustup default stable-x86_64-pc-windows-msvc` |
| Visual Studio Build Tools（「C++ によるデスクトップ開発」） | リンカー（`link.exe`）と Windows SDK | Rust のインストーラが案内します |
| Git | リポジトリの取得 | |
| cmake | LM なしのビルドでは要りません。項目 37・38（S4。LM つき）のときだけ | <https://cmake.org/download/>（PATH に通す） |
| Sysinternals DebugView（任意） | TIP のログを見る（`OutputDebugString`） | TIP は、ログをファイルに書きません。DebugView がなくても、`windows\dev\dbgcap.ps1` で同じログを取れます（§1.6） |
| 確認するアプリ | Notepad、Edge、Chrome、VS Code（[ADR 0040](adr/0040-windows-feasibility.md) §6 の S2）。あれば Word | |

### 1.3 リポジトリの取得

```powershell
git clone https://github.com/ikari-zen/ikari-ime.git
cd ikari-ime
```

別の版を試すときは、ブランチを `git switch` で替えます。PowerShell のスクリプトの実行が止められるときは、そのウィンドウだけ `Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass` にします（システムの設定は変えません）。

#### 1.3.1 VM でビルドしない場合（CI の成果物を持ち込む）

遅い機械では、VM に Rust と Build Tools を入れてビルドする代わりに、CI（`windows-artifact` job）が作った release ビルドを持ち込めます（`ikari-engine-host.exe` は LM つきで、GPU（Vulkan。ADR 0059）に対応しています。バックエンドの DLL は exe の隣の `backends\` にあるので、フォルダごと持ち込みます。`ikari-engine-host-nolm.exe` は LM なし。Rust、Build Tools、cmake、git は要りません。LM は設定でオンにするまで使われません）。

Mac で（`gh` にログインしておく）:

```sh
gh run list --branch <ブランチ> --workflow ci.yml --limit 1      # 実行の番号を探す（成功しているもの）
gh run download <番号> -n ikari-windows-dev-<コミットの SHA> -D ikari-windows-dev
```

`ikari-windows-dev` を VM に持ち込み（`scp` など）、その中の `windows\dev` で、`build.ps1` を飛ばして `install-data.ps1` 以降を流します。中身はリポジトリと同じ配置（`target\release\`、`windows\dev\`、`shell-core\resources\keymap`）で、`COMMIT` に元のコミット（PR なら先頭のコミット、main なら main のコミット）の SHA が入っています（結果の記録に書いてください）。項目 1 のうち、ビルドと `cargo test` は CI の結果で代えます。

実際に持ち込んだときの中身（2026-11-14）:

- `ikari-engine-host.exe`（LM つき）、`ikari-engine-host-nolm.exe`（LM なし）、`ikari_tip.dll`、`ikari-eval.exe`、`eval\sets`
- 2 回目以降に入れ替えるとき: 持ち込んだものを `target\release\` に上書きし、管理者の PowerShell で `.\register.ps1` を流し直し、`.\start-engine.ps1 -Action restart` でエンジンを起動し直します。`register.ps1` は毎回 `%LOCALAPPDATA%\ikari-ime-dev\install\tip\<日時>\` という新しいフォルダに DLL を置き、登録を解除せずに向け直すので、使用中の DLL に当たっても止まりません（ADR 0063 §3 の 8。項目 29）。`target\release\` のものはどれもコピーの元で、アプリもエンジンも読み込んでいないので、上書きできます。
- エンジンは `start-engine.ps1` が `%LOCALAPPDATA%\ikari-ime-dev\install\ikari-engine-host.exe` に**この名前で**写して（窓なしの `ikari-engine-host-hidden.exe` も、この名前に直して写す。隣の `llama*.dll`、`ggml*.dll`、`backends\` も写す）、写したものを起動します。TIP は、DLL の 2 つ上のフォルダ（`tip` の親）の `ikari-engine-host.exe` だけを相手にするためです（#409）。`target\release\` のエンジンを直接起動しても、TIP はつながりません。
- 以前の手順（`.\unregister.ps1` で外し、使用中の `%LOCALAPPDATA%\ikari-ime-dev\tip\ikari_tip.dll` を改名してから置き直す）は使いません。そのフォルダは、新しい `register.ps1` のあとはどこからも登録されていないので、サインアウトのあとに手で消します（`status.ps1` が残りを知らせます）。

### 1.4 データ（辞書とモデル）

辞書は約 650MB です。**Windows の上で作る手順は、試していません**（`cargo xtask data fetch` と `cargo xtask dict` が Windows で動くかは未確認。取得は `curl`、展開は `tar`（と、ものによって `unzip`、`xz`）に頼っています）。**Mac で作ったデータを持ち込む**のを標準の手順にします。

Mac で（リポジトリの `data/` の下に作られます。ユーザーの辞書は変わりません）:

```sh
scripts/fetch-data.sh          # 元データの取得（SudachiDict、chiVe、zenz など）
scripts/build-dict.sh          # data/build/dict/ に grammar.bin、system.dict、vectors.bin などを作る
```

次の 2 つのフォルダを、`data/` と同じ形のまま、VM に持ち込みます（共有フォルダ、`scp`、USB のどれでも）。

```
<持ち込み先>\build\dict\   grammar.bin  system.dict  vectors.bin  （あれば emoji.tsv  postal.tsv）
<持ち込み先>\raw\zenz\     zenz-v3.2-small-Q5_K_M.gguf            （LM なしなら要りません）
```

- zip にして持ち込むと、展開したファイルに「インターネットから取得」の印が付くことがあります。辞書の読み込みに影響するかは分かりません。読めなかったら、それも記録してください。
- Mac の辞書と Windows で同じ形式として読めるかは、最初の確認（項目 1）で分かります（どちらもリトルエンディアンの見込みです）。
- 実際に持ち込んだ例（2026-11-14）: 辞書を VM の `C:\ikari-data\build\dict\` に置き、`.\install-data.ps1 -Src C:\ikari-data` を流しました（`-DryRun` で計画を見てから。確認が出たら、同じファイルを飛ばして違うものだけ置き換えます。`-Force` を付けると、確認を省けます）。この辞書を、エンジンが開けたことは項目 2 で確かめました（§4）。

### 1.5 スクリプトの設定の置き場所

`windows\dev\` のスクリプトは、場所や名前を `windows\dev\common.ps1` の 1 か所に持っています。

| 何 | 既定 | 変えるとき |
| --- | --- | --- |
| データのディレクトリ | `%LOCALAPPDATA%\ikari-ime`（`IKARI_USER_DIR` があればそちら。2026-11-12 決定、#333） | エンジンの決め方（`core/src/user/dir.rs`）と同じにする。以前に `%APPDATA%\ikari-ime` に置いた開発用のデータは、手で `%LOCALAPPDATA%\ikari-ime` に移すか、消す |
| 導入先 | `%LOCALAPPDATA%\ikari-ime-dev\install`。インストーラの `<インストール先>\ikari-engine-host.exe` と `<インストール先>\tip\<版>\ikari_tip.dll` と同じ形にする（ADR 0063 §3。TIP はこの形のエンジンだけを相手にする。#409） | |
| エンジンの exe | ビルドの結果 `target\release\ikari-engine-host.exe`（窓なしは `-hidden`）。動かすのは、`start-engine.ps1` が起動の前に写す `<導入先>\ikari-engine-host.exe`（名前は常にこれ） | パッケージ名は `ikari-engine-host` で、ADR などが呼ぶ `ikari-engine.exe` ではありません |
| TIP の DLL | ビルドの結果 `target\release\ikari_tip.dll`。登録するのは、`register.ps1` が毎回作る `<導入先>\tip\<日時>\ikari_tip.dll`（と隣の `resources\keymap\`）の**コピー**（アプリが読み込んだ DLL を上書きしないため） | |

スクリプトは、ASCII だけで書いています（Windows PowerShell 5.1 は、BOM のないファイルを日本語の文字コードとして読むため）。

### 1.6 VM への接続と、画面なしの確認（2026-11-14 に使った方法）

- **接続**: Mac から `ssh win11`。SSH の既定のシェルは PowerShell です（`&` で並べられません。並べるなら `cmd /c` を使うか、`.ps1` を `scp` して `-File` で実行します。複雑な引数は `.ps1` にします。窓を出さずに起動するのは `Start-Process -NoNewWindow` です。`-NoWindow` はありません）。呼び出しごとに `Set-ExecutionPolicy -Scope Process Bypass` が要ります。
- **文字コード**: SSH の出力は cp932 になることがあります。結果はファイルに書いて `scp` で Mac に取り、UTF-8 として読みます。
- **セッション**: SSH はセッション 0 で、ウィンドウを出せません。TIP とエンジンのパイプ名には `-s<セッション番号>` が入るため、TIP と同じデスクトップ（セッション 1）で動かす必要があります。SSH から起動したプロセスは、切断で一緒に終わりました。
- **セッション 1 での実行**: `windows\dev\run-in-session.ps1` が、スケジュールタスク（`/IT /RU`）でセッション 1 に指定の `.ps1` を流し、`-OutFile` ができるまで待って表示します。タスクの名前はそのたびに変わります（同じ名前だと、動いているタスクを潰すため）。

  ```powershell
  .\run-in-session.ps1 -Script C:\ikari-dev\windows\dev\uia-probe.ps1 -OutFile C:\ikari-dev\uia-probe.txt
  ```

- **常駐のエンジン**: ログオンでセッション 1 に起動するスケジュールタスク（`ikari-engine-dev`）で動かしました。ADR 0063 のあとは、タスクの実行ファイルを `<導入先>\ikari-engine-host.exe`（`%LOCALAPPDATA%\ikari-ime-dev\install\`。`start-engine.ps1` が写す）に向け直してください。`target\release\` のエンジンには、TIP がつながりません（#409）。
- **画面の代わりに読むもの（画面が見えないときに使う）**:
  - `uia-probe.ps1`: UI Automation で Notepad の部品を一覧にします。Notepad の本文を読めることを、2026-11-14 に確かめました。
  - `ime-test.ps1`（`-Mode Prepare`、`Activate`、`Release`、`Read`）: `Prepare` は、Notepad を終了し、LocalState を消してから（Notepad は前回のタブを復元するため）起動し、本文を空にして、ikari を有効にします。`Read` は本文と有効なプロファイルを UTF-8 で書きます。
  - `dbgcap.ps1`: このセッションの `OutputDebugString`（TIP のログ）を、ファイルに書きます（既定は `C:\ikari-dev\dbg.log`、120 秒）。DebugView は要りません。
  - キーは、VM のホスト側で `virsh send-key` を使って送ります。8 キーずつに分けて送ります。F キーは、明示のトークンを付けて送ります。
- **画面（`virsh screenshot`）の注意**: 2026-11-14 の時点で、デスクトップとタスクバーは映りましたが、Run（Win+R）や Notepad の窓は映りませんでした（原因は未確認）。目視の項目（13、14）は、上の道具で代えて確かめました。
- **読み取りでフォーカスが動くと**: TIP のセッションが切れ、組み立て中の文字が消えることがあります。
- **TIP の初期モード**: TIP は起動時に日本語のモードで始まります（#349）。直す前は、新しく開いた Notepad で打鍵が英語のモードのまま素通しになりました。

## 2. 導入

管理者でない PowerShell で、`windows\dev\` に移ります。

| 順 | コマンド | 内容 |
| --- | --- | --- |
| 1 | `.\build.ps1` | `cargo build --release -p ikari-engine-host -p ikari-tip`（LM なし）。窓を出さないエンジンは `-HideConsole`（項目 6 で使う） |
| 2 | `.\install-data.ps1 -Src <持ち込み先> -DryRun`、次に `-DryRun` なしで | `<データ>\dict\` と `<データ>\models\` にだけ置く。同じファイルは飛ばし、違うファイルを置き換えるときは確認する。学習、ユーザー辞書、プロファイル、`settings.tsv` には触らない。エンジンが動いていると止まる |
| 3 | `.\start-engine.ps1` | エンジンを `<導入先>\ikari-engine-host.exe` に写してから（同じなら写さない。窓なしの exe も、この名前で写す）、別のコンソールで起動し、パイプができるまで最大 10 秒待つ。`-Foreground` はこのコンソールで動かす（Ctrl+C の確認用） |
| 4 | `.\register.ps1 -DryRun`、次に**管理者の** PowerShell で `.\register.ps1` | 計画を表示し、確認を取ってから、DLL とキーマップの表を新しいフォルダ `<導入先>\tip\<日時>\` にコピーし、`regsvr32 /s` で登録する（`HKLM\Software\Classes\CLSID\{83A140BE-…}`、日本語の入力プロファイル、TIP の分類）。前の版の登録は解除せず、新しいフォルダに向け直す。古い版のフォルダは `.\clean-tip.ps1` で消す（使用中のものは残る） |
| 5 | 設定アプリで入力方式を足す | 「設定 > 時刻と言語 > 言語と地域 > 日本語 > 言語のオプション > キーボードを追加する」で「ikari」を足し、入力方式の切り替えで選ぶ（名前と場所は Windows の版で違います） |
| 往復時間の表 | `.\latency.ps1` | `<データ>\logs\tip-latency.log`（TIP の測定。`latency.on` があるときだけ書かれる。項目 18）を、中央値と p95 の表にする（読むだけ） |
| 状態 | `.\status.ps1` | エンジンのプロセス、パイプの名前、登録、データのファイル、ログの末尾を表示する（何も変えない） |

**すでに `%LOCALAPPDATA%\ikari-ime` に学習がある環境では**、`install-data.ps1` は辞書とモデルのファイルだけを置き、そのほかには触りません。それでも、確かめたいデータがあるなら、先に `IKARI_USER_DIR` を空のフォルダに向けて試してください（ユーザー環境変数にして、エンジンとアプリを起動し直します。TIP は、アプリの中でこの変数を読みます）。

## 3. ログの場所と、困ったとき

| 見るもの | 場所 |
| --- | --- |
| エンジンの標準エラー | `start-engine.ps1` が開いたコンソール（`-Foreground` ならこのコンソール） |
| エンジンの起動の失敗 | `<データ>\logs\start-failure.log`（最後の 1 回分。理由の語だけ。入力の内容は持たない） |
| エンジンのログ | `<データ>\logs\engine.log`（時刻、要求の種類、所要時間。4MB で回る） |
| TIP のログ | DebugView（`OutputDebugString`。固定の文と HRESULT だけで、入力した文字は出ない。例: `show candidates sent=1 seq=2 side=below source=client …`、`cannot create the shell`） |
| 登録 | `regsvr32` の終了コード（3 = DLL を読めない、4 = `DllRegisterServer` がない、5 = 登録が失敗）。`status.ps1` の「TIP registration」 |
| 全体の状態 | `.\status.ps1` |
| アプリが固まった | タスクマネージャーでそのアプリを終了する。Windows 全体が固まったら、VM のスナップショットに戻す |

## 4. 確認項目の全体と、出どころ

出どころの略号: **#315**（W1-b: 名前付きパイプ）、**#318**（W2-b: COM の層。11 項目）、**#326**（W4-s2: 設定画面の口）、**#329**（再接続）、**#330**（W5-a: engine-host の main）、**#338**（secure な欄）、**#339**（W4: 英語のモードとモードの表示）、**S0〜S11**（[ADR 0040](adr/0040-windows-feasibility.md) §6）。

| 項目 | 内容 | 出どころ | S |
| --- | --- | --- | --- |
| 1 | ビルドとテストが通る | #315、#326、#330 | S0 |
| 2 | エンジンが起動し、パイプができる | #330 | — |
| 3 | 辞書がないとき、理由を残して止まる | #330 | — |
| 4 | 二重に起動しない | #330 | — |
| 5 | Ctrl+C で終わる | #330 | — |
| 6 | 窓なしのビルド、コンソールを閉じる、強制終了 | #330（質問 2、3） | — |
| 7 | 設定画面のパイプができる | #326 | S11 |
| 8 | パイプの OS 層のテストが Windows で通る | #315 | S3 |
| 9 | リモートからパイプを開けない | #315 | S3 |
| 10 | 別のユーザーからパイプを開けない | #315 | S3 |
| 11 | デスクトップのアプリからつながり、ストアのアプリからはつながらない | #315、#318（IMMERSIVESUPPORT） | S3、S7 |
| 12 | 登録と解除 | #318-1 | S9 |
| 13 | 入力方式に出て、切り替えられる | #318-1 | — |
| 14 | Notepad で、読み → Space → 確定 | #318-2 | — |
| 15 | キーの通知（`OnTestKeyDown`）の扱い | #318-7 | — |
| 16 | 編集セッションの同期 | #318-8 | — |
| 17 | 周辺テキストと位置が取れるか（4 つのアプリ） | #318-3 | S2 |
| 18 | キー 1 回と Space の往復の時間 | #318-2、#315、#329 | S1 |
| 19 | AltGr とショートカット | #318-6 | — |
| 20 | デッドキー | #318-6 | — |
| 21 | IME のオン・オフ（今の挙動の記録） | #318-9 | — |
| 22 | タイマー | #318-10 | — |
| 23 | フォーカスの移動 | #318-11 | — |
| 24 | エンジンを止めたとき、固まらず、戻る | #318-5、#329 | S3 |
| 25 | ログオン直後（エンジンより先に TIP が動く） | #329 | — |
| 26 | パイプが「ふさがっている」ときの引っかかり | #329、#315 | — |
| 27 | つなぎ直しの間隔と、パイプの値 | #329、#315 | S1 |
| 28 | TIP からエンジンを起動する（まだない。現状の記録） | #318-4 | S3 |
| 29 | 使用中の DLL の置き換え | #318-1、#315 | S9 |
| 30 | 使用中の辞書の置き換え | ADR 0058 §14 | S5 |
| 31 | 署名なしのファイルの扱い | ADR 0040 §2.7 | S7 |
| 32 | パスワード欄など（secure な欄の見分け） | #338、ADR 0040 §6 | S8 |
| 33 | 候補窓の方式のスパイク | ADR 0040 §2.4 | S10 |
| 34 | 設定画面の方式のスパイク | ADR 0040 §2.5 | S11 |
| 35 | 英語のモードの切り替えと、打鍵の素通し | #339 | S8 |
| 36 | モードの表示（タスクバーの入力インジケーター、compartment） | #339 | — |
| 37 | LM つきのエンジンのビルドと起動（既定オフ、オンにする） | ADR 0040 §2.6 | S4 |
| 38 | LM の 1 回の時間（`ikari-eval`） | ADR 0040 §6 | S4 |
| 39 | GPU（Vulkan）で LM が動くことと、GPU の無い状態で CPU に落ちること | ADR 0059 | S4 |
| 40 | LM の子が動き、落としてもエンジンが生き残ること | ADR 0060 Q4 | S4 |
| 41 | データのフォルダのアクセス権（ACL） | ADR 0066〜0073、#435 | — |
| 42 | TIP が高い権限・ストアのアプリで素通しになる | ADR 0066〜0073、#435 | S7 |
| 43 | engine-host が高い権限の起動と相手を断る | ADR 0067、0068、#435 | — |
| 44 | LM の子の檻（低い整合性、Job） | ADR 0060、0066〜0073、#435 | S4 |
| 45 | 製品ビルドの検査と、開発用ビルドの従来どおりの動き | ADR 0067、#435 | — |
| 46 | 辞書の検証による起動時間 | ADR 0066〜0073、#435 | S5 |
| 47 | インストール先の検査 | ADR 0067 §3、#441 | — |
| 48 | 登録、更新、古い版の掃除 | ADR 0063、0067、#441 | S9 |
| 49 | ファイアウォールの規則 | ADR 0066 §6.2、0067 §4、#441 | — |
| 50 | 自動起動 | ADR 0067 §4、#441 | — |
| 51 | 止める側（インストーラから合図して終わらせる） | ADR 0068 §6.2、#441 | — |
| 52 | Smart App Control の案内 | ADR 0069 §3、#441 | S7 |
| 53 | OS のエラー報告からの除外 | ADR 0073 §4、#441 | — |
| 54 | 終わりの案内と削除 | ADR 0070 §4、#441 | — |
| 55 | インストーラ自身の一時の場所 | ADR 0067 Q3、#441 | — |
| 56 | 設定画面（WinUI 3）の骨組みが出る | ADR 0071 §6、#441 | S11 |
| 57 | 設定画面と候補窓が、自分から通信しない | ADR 0071 §6、0066 §6.2 | — |
| 58 | 設定の口が、設定画面でない相手を断る | ADR 0068 §4、#441 | S3 |
| 59 | 候補窓の起動、二重起動、高い権限 | ADR 0062、0068 §5、#441 | S10 |
| 60 | 送り手で窓が出て、フォーカスを奪わず、決めた位置に出る | ADR 0062、#441 | S10 |
| 61 | 受けたものの上限（切り詰め）の見た目 | ADR 0068 §5、#441 | — |
| 62 | 候補窓のビルドと CI | ADR 0062、#441 | — |
| 63 | engine-host が候補窓を起動し、見張る | ADR 0062 §5.4、#441 | — |
| 64 | TIP が窓に送り、先頭の 1 文字の下に出る | ADR 0062 §6・§7.4、#441 | S2 |
| 65 | 窓が無い・落ちた・偽物のときに入力が止まらない | ADR 0062 §5.4、#441 | — |
| 66 | 候補窓への片道の遅れ | ADR 0062 §10、#441 | S1 |
| 67 | 設定画面がエンジンにつながり、値を変え、学習を書き出し・取り込みできる | ADR 0068 §4・§7、#441 | S11 |
| 68 | モデルの取得（`ikari-data`） | ADR 0076、#446 | — |
| 69 | LM の子の檻の段 2（AppContainer。GPU を含む） | ADR 0072 §12、#446 | S4 |
| 70 | 取得の起動（設定画面の核 → `ikari-data` → 低い整合性の子）の入れ子の Job | ADR 0076 §3.2・§17、#446 | — |
| 71 | アンインストールでの檻のプロファイルの削除 | ADR 0072 §12.8 の 3、#446 | — |
| 72 | Windows の CI で初めて動く検査 | #446 | — |
| 73 | 別の利用者が持つパイプにつながない | ADR 0066 P1・P2、ADR 0068 §2 の 2 | S3 |
| 74 | vc_redist が入っていない素の Windows での起動（TIP・エンジン、候補窓・設定画面、LM の子）と、失敗の見え方 | ADR 0081 §3 の 2、#572 | — |

この文書の範囲に入らないもの: **S6**（ARM。1.0.0 の後）、W6（セキュリティレビュー）。項目 47 以降は、#441 で入ったものと、#446（マージ前）で入るものを含みます。

## 5. 確認項目

各項目は「やること」「期待する結果」「うまくいかないとき」の順です。「期待する結果」は見込みで、実機で見るまで事実ではありません。結果は §6 の表に書きます。ここでの `<データ>` は、`status.ps1` が最初に表示する `DataDir` です。

### 5.1 エンジン（常駐の exe）

#### 項目 1: ビルドとテストが通る（S0）

- やること: `.\build.ps1`。続けて、リポジトリの根で `cargo test --workspace`（LM なし）。
- 期待する結果: ビルドが通り、`target\release\` に `ikari-engine-host.exe` と `ikari_tip.dll` ができる。テストが通る。Windows の CI（`windows-latest`。必須ではない）の結果が通っていれば、実機でも通る見込み。
- うまくいかないとき: 失敗したテスト名と、メッセージを記録する。リンクの失敗は、Build Tools（C++ と Windows SDK）が入っているかを確かめる。`cargo test` の Windows だけのテスト（`pipe` の `os::windows`）の失敗は、項目 8 に書く。

#### 項目 2: エンジンが起動し、パイプができる

- やること: データを入れて（`.\install-data.ps1 -Src <持ち込み先>`）、`.\start-engine.ps1`。続けて `.\status.ps1`。
- 期待する結果: エンジンのプロセスが動き、`\\.\pipe\ikari-ime-engine-v1-<SID>-s<セッション>` と `\\.\pipe\ikari-ime-settings-v1-<SID>-s<セッション>` の 2 本が見える。`status.ps1` の「Data files」に辞書が `present` と出る。Mac で作った辞書をエンジンが開ける。
- うまくいかないとき: `<データ>\logs\start-failure.log` を見る（終了コード 2 なら、データのディレクトリが決まらない、パイプを作れない。辞書が開けないときは終わらずに常駐し、理由だけが残る。項目 3）。`engine.log` も見る。辞書の形式の問題なら、エラーの文をそのまま記録する。

#### 項目 3: 辞書がないとき、理由を残して常駐し、辞書を置くと変換に戻る（ADR 0062 §5.4 の決定 1。#414）

- やること: 動いているエンジンを止め（`.\start-engine.ps1 stop`。パイプの名前はデータのフォルダによらず 1 つなので）、空のフォルダを作り、`target\release\ikari-engine-host.exe --data-dir <空のフォルダ>` を直接動かす（`start-engine.ps1` は、辞書がないと先に止まるので使わない）。動かしたまま、メモ帳で ikari に切り替えて「この」と打って Space。次に、そのフォルダの `dict\` に辞書を置き、2 秒以上待ってからもう一度打つ。終わりは Ctrl+C。
- 期待する結果: 標準エラーに理由が 1 行出て、プロセスは終わらない。パイプの 2 本が見える。`<空のフォルダ>\logs\start-failure.log` に理由が残る。辞書を置く前は、かなの候補（ひらがな、カタカナ）が出て、打った字は失われず、素通しにもならない。辞書を置いたあとは、漢字の候補が出る。
- うまくいかないとき: 終わってしまったら、終了コードを `$LASTEXITCODE` で見て記録する。ログが作られないときは、フォルダの作成の失敗を疑って、標準エラーを記録する。辞書を置いても変換に戻らないときは、`engine.log` と `start-failure.log` を記録する。

#### 項目 4: 二重に起動しない

- やること: エンジンが動いている間に、もう一度 `target\release\ikari-engine-host.exe --data-dir <データ>` を別のコンソールで動かす（`start-engine.ps1` は、動いていれば何もしない）。
- 期待する結果: 2 つ目はすぐに終わり、終了コードは 0（`engine.lock` を持っている別のエンジンがある、という扱い）。1 つ目は動き続け、パイプも残る。
- うまくいかないとき: 2 つ目が動き続ける、または 1 つ目が落ちたら、記録する（`engine.lock` のロックの挙動。`LockFileEx`）。

#### 項目 5: Ctrl+C で終わる

- やること: `.\start-engine.ps1 -Foreground`（またはエンジンの窓）で、Ctrl+C。事前に、何か変換して学習を発生させておく（項目 14 のあと）と、保存も見られる。
- 期待する結果: 数秒以内に終わる。終了コードは 0。パイプが消える（`.\status.ps1`）。`engine.lock` を別のエンジンが取れる（もう一度起動できる）。
- うまくいかないとき: 止まらない場合は、どのくらい待ったか、標準エラーを記録する。受け付けの終了が待たれているかもしれない（`run_host` の join）。

#### 項目 6: 窓なしのビルド、コンソールを閉じる、強制終了（#330 の質問 2、3）

- やること:
  1. コンソールの窓を閉じる（×）。エンジンが終わるか、学習が保存されているか（`<データ>` の学習のファイルの更新時刻）を見る。
  2. `.\start-engine.ps1 -Action stop`（強制終了。確認が出る）。再起動して、学習が残っているか見る。
  3. 窓なしの exe で起動する（CI の artifact には `ikari-engine-host-hidden.exe` が入っていて、`.\start-engine.ps1` はそれがあれば自動で使う。VM に cargo が無いので `build.ps1 -HideConsole` は使わない。コンソールつきに戻すときは `-Console`）。窓が出ないこと。窓を閉じる操作が無いので、Alt+F4 やキーで止まらないこと（#372）。ログオフとシャットダウンでは `exit=clean` になり学習が残ること。辞書を一時的に外して（`IKARI_USER_DIR` を空のフォルダにして）起動し、`start-failure.log` に理由が残ること。
  4. ログオフと、シャットダウンで、エンジンの終わり方と学習を見る（VM を再起動する前に、スナップショットを取る）。
- 期待する結果: 1 と 2 のあと、学習は最大でも直近 5 秒分を失うだけで、ロックは OS が解放する（`engine.lock` を取り直せる）。3 は窓が出ず、失敗はファイルに残る。4 は、ログオフとシャットダウンの通知を受けられない可能性が #330 に書かれている（窓がないため）。**どうなったかを記録する**（メッセージ専用ウィンドウを足す案の判断材料）。
- うまくいかないとき: `engine.log` の最後の行と、学習のファイルの更新時刻を記録する。

#### 項目 7: 設定画面のパイプができる（S11 の前提）

- やること: `.\status.ps1` で settings のパイプの名前を見る。
- 期待する結果: `ikari-ime-settings-v1-…` が、エンジンのパイプと別にある。設定画面の実装（W4-s3）はまだないので、**つないで中身を確かめることはできない**。Windows 上での `cargo test -p ikari-pipe -p ikari-engine-host`（項目 1 の `cargo test --workspace` に含まれる）が通れば、口の挙動はテストで守られている。
- うまくいかないとき: パイプがなければ、`engine.log` と標準エラーを記録する。

### 5.2 名前付きパイプ（S3）

#### 項目 8: パイプの OS 層のテストが Windows で通る（#315）

- やること: `cargo test -p ikari-pipe`（項目 1 に含まれる）。
- 期待する結果: 次のテストが通る（#315 の確認待ちの一覧）。`round_trip_and_server_pid`（往復、相手の PID）、`concurrent_requests_on_one_connection`（1 本で 8 スレッドの同時の要求）、`first_instance_rejects_a_second_listener_with_the_same_name`、`dacl_allows_only_the_user_and_system`（ACE が自分と SYSTEM の 2 つだけ）、`server_closing_disconnects_the_client`、`peer_drop_gives_eof`（#315 に「推測。要確認」とある）、`shutdown_wakes_a_blocked_reader_and_fails_writes`、`closer_ends_a_waiting_accept`、`missing_pipe_fails_fast`、`current_user_names`、`names_with_nul_are_rejected`。`rejects_remote_clients` は、SMB のサーバーが止まっていても失敗するので、通っても拒否のフラグの効き目とは区別できない（項目 9 で確かめる）。
- うまくいかないとき: 失敗したテスト名と出力を記録する。`CancelIoEx` のあとの `GetOverlappedResult` が戻らない、`ERROR_PIPE_CONNECTED` と `ERROR_NO_DATA` の扱い、は #315 の「実機で見るもの」。テストが止まって戻らないときは、どのテストかを記録する。

#### 項目 9: リモートからパイプを開けない

- やること: 2 台目の Windows（ホストの Windows や別の VM）が用意できるときだけ。エンジンを動かしている VM に、別の機械から `\\<VM の名前>\pipe\ikari-ime-engine-v1-<SID>-s<セッション>` を開く（PowerShell で `[IO.File]::Open(...)`、またはエクスプローラーの検索ボックス）。
- 期待する結果: 開けない（`PIPE_REJECT_REMOTE_CLIENTS`）。
- うまくいかないとき: 機械が 1 台なら、「未確認（2 台目がない）」と記録する（項目 8 のテストだけでは区別できない）。

#### 項目 10: 別のユーザーからパイプを開けない（DACL の実効）

- やること: 標準ユーザーを 1 人作り、そのユーザーで、自分とは別のユーザーのエンジンのパイプを開く（`[IO.Pipes.NamedPipeClientStream]` で、パイプの名前に、エンジンのユーザーの SID を含めて）。
- 期待する結果: アクセス拒否（`ERROR_ACCESS_DENIED`）。DACL は、自分の SID と SYSTEM だけに GRGW を許す。
- うまくいかないとき: 開けてしまったら、重大な発見。接続のコードと結果を記録して、止める。

#### 項目 11: デスクトップのアプリからつながり、ストアのアプリからはつながらない（S3、S7）

- やること: 項目 14 の確認で、デスクトップのアプリ（Notepad など）から変換できることを見る。そのうえで、ストアのアプリ（AppContainer。電卓など、入力欄のあるもの）で ikari に切り替えて打つ。
- 期待する結果: デスクトップのアプリは、パイプにつながる（S3）。ストアのアプリは、ACL のためつながらない見込み（キーは失わず、漢字に変換できないだけ。#318）。IMMERSIVESUPPORT の登録を外すかは、この結果を見て決める（#318）。
- うまくいかないとき: ストアのアプリが固まる、入力できなくなる、ikari に切り替えられない、のどれかを記録する。

### 5.3 TIP（アプリに入る DLL）

#### 項目 12: 登録と解除（S9）

- やること: 管理者の PowerShell で、`.\register.ps1`（先に `-DryRun`）。`.\status.ps1` で確認。解除は `.\unregister.ps1`（登録中の DLL を registry から読んで解除する。`-RemoveFiles` を付けると `tip\<日時>\` のフォルダも消す。使用中のものは残る。確認が出る）。登録したまま HKCU だけで足りるか、という S9 の問いは、**この手順では試せない**（今の DLL は HKLM（`Software\Classes`）にだけ書く。`register.rs` の `TODO(S9)`）。記録だけする。
- 期待する結果: `regsvr32` の終了コードが 0。`status.ps1` の「CLSID registered」に、コピーした DLL のパス（`…\tip\<日時>\ikari_tip.dll`）が出る（`ThreadingModel=Apartment`）。版のフォルダの一覧で、そのフォルダに「registered」が付く。解除のあと、CLSID が消え、入力方式の一覧から「ikari」が消える（開いているアプリは、アプリを開き直すまで残ることがある。**どうなったかを記録する**）。登録が途中で失敗したら、初めての登録なら書いたものを消し、前の版の登録があれば消さずに残す作りになっている（`windows/tip/src/deploy.rs`。ADR 0063 §3 の 2）。
- うまくいかないとき: 終了コード 3 は DLL が読めない（DLL の依存、x64 の PowerShell かどうか）、5 は `DllRegisterServer` の失敗（`HRESULT` を、DebugView で見る）。登録が半端になったら、`.\unregister.ps1` を試し、だめなら VM のスナップショットに戻す。

#### 項目 13: 入力方式に出て、切り替えられる

- やること: §2 の順 5 で「ikari」を足し、Notepad を開いて、言語の切り替え（Win+Space）で「ikari」に替える。
- 期待する結果: 一覧に日本語の入力方式として「ikari」が出て、選べる。選んでも Notepad が固まらない。TIP が読み込まれ、DebugView に TIP のログが出る。
- うまくいかないとき: 出ないとき、登録のプロファイル（日本語 0x0411）と、Windows の言語の設定（日本語を足してあるか）を記録する。固まったら、スナップショットに戻して、DebugView の最後の行を記録する。

#### 項目 14: Notepad で、読み → Space → 確定

- 前提: エンジンが動いている（項目 2）、ikari を選んである（項目 13）。**候補窓がないので、候補の一覧は見えない**。最初の Space のあとに、合成の文字列が 1 位の候補に替わり、Enter で確定するところを見る。
- やること: ローマ字で「kyouhaharedesu」と打ち、Space、Enter。
- 期待する結果: 打つあいだ、かなが下線つきで出て、Space で漢字まじりの文字列（1 位の候補）に替わり、Enter で確定する。2 重に入力されない。取りこぼさない。エンジンの `engine.log` に Convert の要求と所要時間が出る。
- うまくいかないとき: かなのまま変わらないなら、エンジンにつながっていない（`status.ps1` のパイプと、DebugView）。つながらない間は、shell-core が、エンジンなしの候補（かな）で動く（#329）。入力を失う、文字が 2 重になる、は、項目 15 と 16 の材料。

#### 項目 15: キーの通知（`OnTestKeyDown`）の扱い

- やること: 項目 14 を、Notepad、Edge、Chrome、VS Code、（あれば Word）で行う。文字、Space、Enter、BackSpace、矢印、Esc、Shift つきの組み合わせを打つ。
- 期待する結果: どのアプリでも、2 重に入力されず、取りこぼしがない。TIP は、最初に来たキーの通知（ふつうは `OnTestKeyDown`）で実行する作りで、続く `OnKeyDown` は実行せずに「食べた」を返す。
- うまくいかないとき: 次を記録する。`OnTestKeyDown` が呼ばれずに `OnKeyDown` だけが来るアプリ、両者で `lParam` が違うアプリ、2 重入力や取りこぼしが出る操作と、アプリの名前。

#### 項目 16: 編集セッションの同期

- やること: 項目 15 のアプリで、通常の入力欄と、読み取り専用の欄（ブラウザのページの本文など）で打つ。
- 期待する結果: 入力欄では、キーの通知の中で同期の編集セッション（`TF_ES_SYNC`）が取れて変換できる。取れない欄では、`dispatch` を呼ばず、キーがそのままアプリに渡る（TIP の作り）。非同期のセッション（タイマー、フォーカスの移動）が、いつ実行されるか。
- うまくいかないとき: 変換できない欄と、アプリの名前を記録する。DebugView の、編集セッションに関する行を添える。

#### 項目 17: 周辺テキストと位置が取れるか（S2）

- やること: Notepad、Chrome、Edge、VS Code で、文の途中にカーソルを置き、変換を始める（Space）。DebugView を見る。
- 期待する結果: アプリごとに、周辺テキスト（`ITfRangeACP` が取れるか）と、候補窓の位置（`ITfContextView::GetTextExt`）が取れるか。位置が取れたとき、DebugView の `show candidates` の行に `source=client` が出る（第 2 段から。前は `(position found)`）。取れるアプリと取れないアプリの一覧が、S2 の答え。
- うまくいかないとき: 取れないアプリを、アプリの名前と版で記録する。ブラウザ（Chrome、Edge）は、TSF の対応が欄ごとに違うことがある。ページごとに（検索欄、本文の入力欄、`contenteditable`）記録する。

#### 項目 18: キー 1 回と Space の往復の時間（S1）

- 測り方（#334。TIP の測定は**既定でオフ**）: (1) `<データ>\latency.on` を空のファイルとして作る（`New-Item <データ>\latency.on`）。アプリが測定を決めるのは、そのアプリで ikari を最初に使うときなので、**測りたいアプリを開き直す**（TIP の DLL は各アプリの中で動くので、環境変数 `IKARI_TIP_LATENCY=1` はアプリに見えないことがある。ファイルなら、どのアプリにも効く）。(2) 項目 14 の操作（読みを打って Space で変換、確定）を、アプリごとに 200 回以上行う。(3) `.\latency.ps1` で表を見る。(4) 終わったら `latency.on` を消す（消すまで、開き直したアプリは測り続ける）。
- 何が出るか: `<データ>\logs\tip-latency.log` に、200 件ごと（と、入力を離れるとき）に 1 行ずつ追記される。`latency.ps1` は、プロセスと種類ごとの最後の行を、ミリ秒の表にする。`request` = 要求を送ってから返事を受けるまで（パイプの往復 + エンジンの処理。要求の種類は混ざる）、`key` = キーを受けてから `TsfOp` を実行し終わるまで（TIP の内側。`request` を含む。候補窓の描画は含まない）、`first_connect` = 最初の接続の試行から最初の成功まで。各行は直近 1000 件の最小、中央値（`p50`）、`p95`、最大。**入力した文字や読みは、ログに入らない**（数字と種類と失敗の数だけ）。
- 期待する結果: `key` の p95 が 16ms 以下（NFR-02。Space から 1 ページ目まで。p99 は出さない）。`request` の中央値が数 ms 以下の見込み（根拠はない。**測ってみないと分からない**）。`key` と `request` の差が TIP の内側（キーの読み、編集セッション）の時間。`engine.log` の所要時間との差が、パイプの往復。
- うまくいかないとき: ログができないなら、(1) `latency.on` の場所が `status.ps1` の `DataDir` と同じか、(2) アプリを開き直したか、(3) 200 件に届いたか（少ない間は、入力を離れたときに書く）を確かめる。数字と、操作の内容（読みの長さ、LM なし、アプリの名前）を記録する。測定そのもののコスト（200 件に 1 回、キーの処理のあとにファイルへ追記する）が気になるなら、その旨も記録する。

#### 項目 19: AltGr とショートカット

- やること: (a) ドイツ語の配列を足して（「言語のオプション」）、AltGr+Q で `@` が打てるか、ikari が有効のときと無効のときで比べる。(b) US と JIS の配列で、Ctrl+Alt+文字が、ショートカットのままアプリに届く（IME に食われない）か。
- 期待する結果: (a) AltGr+Q が `@` として渡る。(b) Ctrl+Alt+文字が、ショートカットとして残る。
- うまくいかないとき: 配列、キー、期待と結果を記録する。`ToUnicodeEx`（キーボードの状態を変えない旗つき。Windows 10 1607 以降が前提）の挙動を疑う。

#### 項目 20: デッドキー

- やること: ドイツ語やフランス語の配列で、デッドキー（`^` のあとに `e`）を、ikari が有効のときに打つ。
- 期待する結果: アプリの側で `ê` に合成される（TIP は状態を変えない）。
- うまくいかないとき: 打った順と、出た文字を記録する。

#### 項目 21: IME のオン・オフ（今の挙動の記録）

- やること: ikari を選んだまま、IME のオフ（Windows の入力のオン・オフの操作）にして、ローマ字を打つ。
- 期待する結果（見込み）: #339 で、IME のオフ（`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` が 0 になる）を ikari の英語のモードとして読む（打鍵をそのままアプリに渡す）。オフで打つと英字がそのまま入り、オンに戻すと日本語に戻る。**見込みで、実機で確かめる**（項目 35、36）。どうなったかを記録する。
- うまくいかないとき: 文字が打てなくなる、固まる、のどれかを記録する。

#### 項目 22: タイマー

- やること: 読みを打って、キーを押さずにしばらく待つ。時間で起きる動作（`TsfOp::ScheduleTimer` で予約されるもの。どの動作かは shell-core のシナリオを参照）が、各アプリで起きるかを見る。
- 期待する結果: 窓を持たないスレッドのタイマー（`WM_TIMER` と `TIMERPROC`）が、アプリのメッセージループで配られ、予約した動作が起きる。
- うまくいかないとき: 起きないアプリを記録する。DebugView を添える。

#### 項目 23: フォーカスの移動

- やること: 合成の途中（かなが下線つきで出ている状態）で、別のアプリをクリックする、タブを替える、Alt+Tab、を行う。
- 期待する結果: 表示していた文字列がそのまま確定し、2 重にならない（`OnCompositionTerminated`）。戻ったとき、続きが打てる。
- うまくいかないとき: 2 重になった、消えた、固まった、のどれかと、アプリの名前、操作を記録する。

#### 項目 35: 英語のモードの切り替えと、打鍵の素通し（#339、S8）

- 準備: `<データ>\settings.tsv`（`IKARI_USER_DIR` を使っているときはその下）に、`keymap.mode.toggle<TAB>ctrl+e`（Ctrl+E で日本語と英語を切り替える）の 1 行を足し、アプリを開き直す。キーの割り当ては `shell-core` の設定に従うので、ほかのキーでもよい。
- やること: ikari を選んだ Notepad で、(a) Ctrl+E を押し、`abc` と打つ。(b) もう一度 Ctrl+E を押し、`kyou` と打つ。(c) JIS 配列のキーボードがあれば、英数キーを押して `abc`、かなキーを押して `kyou`。(d) Edge、Chrome、VS Code でも (a)(b)。(e) ブラウザのパスワード欄で (a)。
- 期待する結果: (a) `abc` がそのまま入る（打鍵がアプリに渡る。`OnTestKeyDown` が FALSE を返す）。Ctrl+E 自体は文字にならない。(b) 日本語に戻り、「きょう」と下線つきで出る。合成の途中で Ctrl+E を押すと、今の表示のまま確定してから切り替わる。(c) 英数キーで英語、かなキーで日本語（`mode::requested_by_event`）。(d) どのアプリでも同じ。(e) 英語のモードで `OnTestKeyDown` が FALSE を返せば素通しになるか、という S8 の問いの答え（項目 32）。
- うまくいかないとき: 英語のモードで文字が 2 重に入る、取りこぼす、切り替わらないキーを、キーの名前（VK、配列）とアプリの名前で記録する。英数・かなキーが TIP に届かず OS が別に処理しているなら、そのことを記録する（JIS 配列の、キーの VK は `OEM_ATTN`（英数）と `OEM_COPY`（かな））。DebugView に、`compartment SetValue failed` などの行が出ていないかを見る。

#### 項目 36: モードの表示（#339。タスクバーの入力インジケーター、compartment）

- 前提: Windows 11 の VM（タスクバーの右端に、入力インジケーターが出ている）。項目 35 の準備が済んでいる。
- やること: ikari を選び、(a) インジケーターの表示を見る。(b) Ctrl+E で英語、もう一度で日本語にして、そのたびに表示を見る。(c) インジケーターをクリックする。(d) 言語バー（「言語バーの設定」でフローティングを出せる場合）にボタンが出るか。(e) アプリを替えて戻る。
- 期待する結果（**実装は文書の説明からの見込みで、実機で確かめる**）: (a) 日本語のとき「あ」、(b) 英語のとき「A」。(c) クリックで日本語と英語が切り替わる（`ITfLangBarItemButton::OnClick` が `OPENCLOSE` を反転し、compartment の通知で `shell-core` が切り替える）。(d) ボタンが出る。(e) アプリごとにモードを持つ（スレッドごとの compartment）。
- うまくいかないとき: **インジケーターに何も出ない、または別の文字（「JA」など）が出る場合が起きうる**。ボタンのアイコンの資源がまだなく（`GetIcon` は空のアイコンを返す）、文字だけで出るかが、確かめたい点の 1 つ。出ないときは、(1) 言語バーのボタン（`GUID_LBI_INPUTMODE`）の文字が使われるか、アイコンが必要か、(2) `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION` と `OPENCLOSE` の値を替えると表示が替わるか（別の TIP や、スクリプトから書いて試す）、(3) 登録のカテゴリ（`GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT`）が効いているか、を記録する。また、ikari を選んだ直後に、OS が `OPENCLOSE` を 0 にして英語のモードで始まってしまわないか（TIP は最初の入力で日本語の値を書き込む）も記録する。

### 5.4 エンジンとの接続（#329 の再接続）

#### 項目 24: エンジンを止めたとき、固まらず、戻る

- やること: Notepad で ikari を選び、変換できる状態で、`.\start-engine.ps1 -Action stop`（強制終了）。そのまま打ち続ける。数秒後に `.\start-engine.ps1`。変換を試す。もう一度、止めて、すぐに再起動を繰り返す。
- 期待する結果: アプリが固まらない。キーが失われない（エンジンがない間は、エンジンなしの候補で動く。#329）。エンジンが戻ると、次のキーのあとの 1 秒ほどで変換が戻る（つなぎ直しの間隔の上限が 1 秒。#329）。再接続のあとの最初の要求で、Hello とセッションが送り直される。60 秒に 3 回落ちると 5 分休む仕組みがある（`CRASH_LIMIT`）ので、短い間に何度も止めると、戻らなくなるのが見込み（記録する）。
- うまくいかないとき: 戻らないなら、(1) `status.ps1` でパイプがあるか、(2) 5 分休みに入っていないか（止めた回数）、(3) アプリを開き直すと戻るか、を記録する。#329 に、「落ちた通知を取りこぼすと、以後ずっとつながらない余地がある」とあるので、戻らないときは、DebugView の行と、止めた時刻を添える。

#### 項目 25: ログオン直後（エンジンより先に TIP が動く）

- やること: エンジンを動かさないまま、サインアウトとサインインをする（自動起動の仕組みは、まだない。W5）。アプリを開いて、ikari で打つ。そのあと、エンジンを起動する。
- 期待する結果: エンジンがない間は、エンジンなしの候補で動き、固まらない。エンジンを起動すると、変換が戻る（項目 24 と同じ）。
- うまくいかないとき: サインイン直後に TIP が読み込まれるアプリ（エクスプローラー、スタートメニューなど）で、遅い、固まる、がないかを記録する。

#### 項目 26: パイプが「ふさがっている」ときの引っかかり

- やること: ふさがった状態を作るのは難しいので、**自然に起きたときの観察**にする。エンジンの起動の直後に、多くのアプリ（10 個ほど）で一斉に ikari に切り替えて打つ。キーの入力に引っかかり（最大 約 0.3 秒の待ち。`WaitNamedPipeW`、再試行 3 回 × 100ms。#315、#329）が出るか。
- 期待する結果: 引っかからない（エンジンは、接続を渡す前に次の受け口を作る）。
- うまくいかないとき: 起きたアプリ、操作、体感の長さを記録する。

#### 項目 27: つなぎ直しの間隔と、パイプの値

- やること: 項目 24 で、エンジンを起動してから変換が戻るまでの時間を、5 回ほど、秒の単位で測る。
- 期待する結果: 最大 約 1 秒の遅れ。つなぎ直しの間隔（100ms から 2 倍ずつ、上限 1 秒）と、パイプの値（`PIPE_BUFFER_BYTES` = 64KiB、`CONNECT_BUSY_RETRIES` = 3、`CONNECT_BUSY_WAIT_MS` = 100）は**計測で決めた値ではなく、暫定**（#329、#315）。この記録が、見直しの材料。
- うまくいかないとき: 長すぎる（数秒）、短くしたい、のどれかを、測った数字と一緒に記録する。値を変える提案は、この手順書の範囲ではない（ユーザーに聞く）。

#### 項目 28: TIP からエンジンを起動する（現状の記録）

- やること: エンジンを動かしていない状態で、ikari で打つ。エンジンのプロセスが自動で起動するか、`status.ps1` で見る。
- 期待する結果: **起動しない**（TIP からの起動は、W5 でまだ作っていない。`connector.rs` は、つながらなければ `Unavailable` を返すだけ）。確認するのは、起動しないことと、キーを失わないこと。S3 の「起動してよい場合の見分け」は、作ってから確かめる。
- うまくいかないとき: 起動してしまったら、そのプロセスの親を記録する（想定外）。

### 5.5 更新、署名、そのほかの未検証の事項

#### 項目 29: 使用中の DLL の置き換え（S9。ADR 0063）

これまでの結果（2026-11-14、#377）: 読み込まれている DLL への上書きは失敗した。Notepad と Edge を閉じても、explorer と Windows Terminal が持ち続けるので上書きできない。ADR 0063 で、版ごとのフォルダに置いて登録を向け直す形に変えた。以下はその確かめ（ADR 0063 §7 のスパイクの 1〜4、6、9 を、開発用のスクリプトで行う）。**VM のスナップショットを取ってから行う。**モジュールのパスは `Get-Process | ForEach-Object { $_.Modules } | Where-Object FileName -like '*ikari_tip*' | Select-Object FileName -Unique`（管理者の PowerShell。`tasklist /m` はパスを出さない）で見る。

- やること:
  1. 管理者の PowerShell で `.\register.ps1`（版 A）。ikari を入力方式に足し、Notepad と Edge で打つ。`status.ps1` で登録中の版（`tip\<日時 A>\`）を記録し、エンジンのプロセスのパスが `<導入先>\ikari-engine-host.exe` であること（警告「Not the install copy」が出ないこと）と、漢字に変換できること（TIP がエンジンを相手と認めた。#409）を見る。上のコマンドで、explorer と Windows Terminal が版 A のパスを持つことを見る。入力方式の一覧（設定の画面と `Get-WinUserLanguageList` の `InputMethodTips`）を記録する。
  2. Notepad を開いたまま、`.\build.ps1`（または新しい artifact の `ikari_tip.dll` を `target\release\` に上書き）、続けて管理者で `.\register.ps1`（版 B）。上書きの失敗で止まらないこと、`regsvr32` が 0 で終わること、`status.ps1` の「CLSID registered」が版 B を指すことを見る。入力方式の一覧と既定の入力方式が手順 1 と変わらないこと（ikari が消えない、Microsoft IME などが足されない）を記録する。
  3. 開いたままの Notepad で打つ（版 A のまま動くか）。新しく開いた Notepad で打つ（版 B か。上のコマンドのパスで見る）。explorer を再起動（タスク マネージャーから）したあと、explorer のモジュールが版 B になるか。
  4. `.\clean-tip.ps1 -DryRun`、次に `.\clean-tip.ps1`。版 A のフォルダが「kept」と出て丸ごと残る（DLL も `resources\keymap\` も残る）こと、版 A を持つ開いたままの Notepad で打ち続けられることを見る。
  5. サインアウトしてサインインする。全部のプロセスが版 B になるか（上のコマンド）。`.\clean-tip.ps1` で版 A のフォルダが消えるか。
  6. 版 A を持つアプリがある状態で、版 A の `ikari_tip.dll` を `Rename-Item` できるか（ADR 0063 §1.4 の推測の確かめ。改名できたら名前を戻す）。
  7. 撤去の確かめ: アプリを開いたまま、管理者で `.\unregister.ps1 -RemoveFiles`。使用中のフォルダが残って「kept」と出ること、開いているアプリが落ちないこと、新しく開いたアプリに ikari が出ないことを見る。
- 期待する結果: 手順 2 で止まらず、登録が版 B を指す。入力方式の一覧は変わらない（ADR 0063 §3 の 2。weasel の注記からの**推測**）。手順 3 は、開いていたアプリは版 A、新しく開いたアプリと再起動した explorer は版 B（**推測**。ADR 0063 §1.4）。手順 4 は、使用中の版 A は残り、開いていたアプリの入力は続く。手順 5 で全部が版 B になり、版 A を消せる。手順 6 は改名できる見込み（**推測**）。
- うまくいかないとき: `register.ps1` と `clean-tip.ps1` の出力、`regsvr32` の終了コード、DebugView の行、上のコマンドのパス、入力方式の一覧を、そのまま記録する。入力方式の一覧が変わった（ikari が外れた、別の IME が足された）ときは、ADR 0063 §3 の 2 の前提が崩れたので、ADR を直す材料にする。

#### 項目 30: 使用中の辞書の置き換え（S5）

- やること: **本番のデータでなく、使い捨てのフォルダで行う。** `IKARI_USER_DIR` をテスト用の空のフォルダにして、`.\install-data.ps1`、`.\start-engine.ps1`。エンジンが動いている間に、PowerShell で、(a) `<テスト用>\dict\system.dict` を上書きする（コピーしたファイルを、`Copy-Item -Force`）、(b) `Rename-Item <テスト用>\dict dict.prev`、を試す。
- 期待する結果: ADR 0058 §14 は、「mmap 中のファイルを含むディレクトリの rename や古い版の削除が、Windows では失敗することがある」と**推測**している。(a) と (b) が成功するか失敗するかを記録する。エンジンが落ちないか、変換が続くか。
- うまくいかないとき: 失敗したときのエラーの文を、そのまま記録する。終わったら、使い捨てのフォルダを削除する。

#### 項目 31: 署名なしのファイルの扱い（S7）

- やること: ブラウザ、または共有フォルダ経由で持ち込んだ `ikari-engine-host.exe` を、エクスプローラーでダブルクリックして起動する。`ikari_tip.dll` を、登録する（Smart App Control が有効な VM でも、無効な VM でも）。
- 期待する結果: SmartScreen の警告や、Smart App Control のブロックが出るか、出たらどう回避するか（[ADR 0040](adr/0040-windows-feasibility.md) §2.7 は、署名なしで配り、README に警告と回避の手順を書く方針）。ビルドしたものをその場で動かした場合は、「インターネットから取得」の印が付かないので、出ない見込み。
- うまくいかないとき: 出た画面と文言を、そのまま記録する（スクリーンショット）。

#### 項目 32: パスワード欄など（S8。#338）

- 実装: 打鍵のたびに、編集セッションの中で `GUID_PROP_INPUTSCOPE` を読み（文脈の先頭の範囲）、`IS_PASSWORD`、`IS_NUMERIC_PIN`、`IS_ALPHANUMERIC_PIN`、`IS_ALPHANUMERIC_PIN_SET`、`IS_PRIVATE`（61。Chromium 系は password 欄にこれを返す。#369）のどれかがあれば secure として `shell-core` に伝える（`windows/tip/src/secure.rs`、`com/field.rs`）。入力スコープが付いていない欄、読めない欄は通常として扱う。`TS_SS_PASSWORD` という旗は TSF にない。
- やること: 次の欄で、ikari を選んで、ひらがなと Space を打つ。(a) Win32 のパスワード欄（サインインの資格情報のダイアログ、`ES_PASSWORD` の Edit）、(b) PIN 欄（Windows Hello の PIN、設定アプリ）、(c) ブラウザ（Chrome、Edge、Firefox）の `<input type=password>`（特に Edge。期待は素通し。#369）、(d) UWP の `PasswordBox`（設定アプリ、ストアアプリ）、(e) 管理者権限のダイアログ（UAC の画面は、ほかのデスクトップなので、そもそも TIP が動かない見込み）、(f) 通常の欄（Notepad、ブラウザの検索欄）。
- 期待する結果: (a)〜(d) で変換が動かず、打ったキーがそのまま欄に入る（合成の下線も候補も出ない）。DebugView に、入力した文字も、secure かどうかも出ない（出るのは、入力スコープの数値の並び `ikari-tip: input scopes=[…]` だけ。値が変わったときに 1 行。Edge の password 欄で 61 が出ることを確かめる）。(f) では今までどおり変換できる。secure な欄から通常の欄へ移ったとき、すぐ変換できる（次のキーで戻る）。
- 記録すること: (1) 各欄で secure と見分けられたか（変換が止まったか）、(2) 見分けられなかった欄（変換が動いてしまった。入力スコープを付けない独自描画の欄、など）のアプリ名、(3) TSF が TIP のキーの通知をそもそも呼ばない欄（その場合も入力は守られる）、(4) キー 1 回の時間が、項目 18 より増えたか（打鍵のたびに入力スコープを読む。増えるなら、フォーカスの移動で読んで覚える案に変える）。
- うまくいかないとき: 入力できなくなる、固まる、変換が動いてしまった、のどれかと、アプリの名前、欄の種類を記録する。
- 英語のモード（ADR 0026）の secure の素通しは、`shell-core` が同じ `secure` の口で扱う（実装は別の issue）。ここでは日本語のモードで確かめる。

### 5.6 スパイク（捨てるコード。main に入れない）

#### 項目 33: 候補窓の方式のスパイク（S10）

- 場所: ブランチ `spike/w3-0-candidate-window`。手順書は [spikes/candidate-window/README.md](https://github.com/ikari-zen/ikari-ime/blob/spike/w3-0-candidate-window/spikes/candidate-window/README.md)（`cargo build --release` して、`candidate-window-spike.exe a|b|c`）。
- 期待する結果: 手順書の表の雛形に、3 方式（WebView2、自前描画、`ITfCandidateListUIElement`）の遅延、メモリ、アプリへの影響を書く。結果は、ユーザーが方式を決め、ADR 0040 §2.4 に追記する。
- うまくいかないとき: 手順書も、実機で動かしたことがない（手順書の冒頭にある）。ビルドの失敗は、そのまま記録する。

#### 項目 34: 設定画面の方式のスパイク（S11）

- 場所: ブランチ `spike/w4-s0-settings-ui`。手順書は [spikes/settings-ui/README.md](https://github.com/ikari-zen/ikari-ime/blob/spike/w4-s0-settings-ui/spikes/settings-ui/README.md)（WebView2 と Slint の 2 つの最小の実装）。
- 期待する結果: 手順書の表の雛形に、見た目、起動の速さ、画面の入力欄で ikari 自身を使って日本語が打てるか、を書く。結果は、ユーザーが方式を決め、ADR 0040 §2.5 に追記する。
- うまくいかないとき: 手順書に従う。Slint のライセンス（GPL-3.0 など 3 択）は、ユーザーの決定が要る（手順書の比較の表）。

### 5.7 LM（S4。cmake が要る）

#### 項目 37: LM つきのエンジンのビルドと起動

- 前提: cmake が PATH にある（`cmake --version`）。`<データ>\models\zenz-v3.2-small-Q5_K_M.gguf` がある（`install-data.ps1` は、持ち込み先の `raw\zenz\` にあれば置く）。
- やること: (a) `.\build.ps1 -Lm`（cmake がなければここで止まる）。(b) LM をオンにしないまま `.\start-engine.ps1`、変換する。(c) エンジンを止め、`<データ>\settings.tsv` に `engine.lm` を `true` にする行を足して（書式は既存の行にそろえる）、起動し直す。変換する。(d) モデルを一時的に別の名前に替えて、起動し直す。
- 期待する結果（**見込み**）: (a) ビルドが通る（CI の `windows` job の最後の step が同じビルド）。(b) LM なしで今までどおり動く。(c) 起動のログに LM を使う旨が出て、同音異義語の並びが LM なしと変わりうる（`cargo run --release -p ikari-cli --features lm` で同じ文を試せる）。(d) 照合に通らず LM なしで動き、理由がログに出る（落ちない）。
- うまくいかないとき: (a) の失敗は、cmake の版、Build Tools の C++ と Windows SDK、`libclang`（`bindgen`）の有無、エラーの先頭 30 行を記録する。(c) で変換が遅くなる・固まるなら、項目 38 を先に。

#### 項目 38: LM の 1 回の時間と、先行計算の 300ms の中で効く割合（S4）

- やること: リポジトリの根で `cargo run --release -p ikari-eval --features lm -- --ranker full --set eval\sets\homophones-ext.tsv --lm <データ>\models\zenz-v3.2-small-Q5_K_M.gguf --dict <データ>\dict`。続けて、`--lm-budget-ms 300`（既定）と、`--lm` なしの結果を比べる。出力の LM の時間（1 回の平均と p99）、時間切れの割合、1 位の変化を記録する。
- 期待する結果: 数字を得ることが目的で、期待値は置かない（[ADR 0040](adr/0040-windows-feasibility.md) §2.6。この数字で、LM の既定をオフのままにするかを決める）。Mac の同じセットの数字（`eval/reports/README.md`）と並べる。
- うまくいかないとき: CPU の機種（VM の割り当てコア数、AVX2 の有無）を記録する。VM の CPU が実機と大きく違うと、数字が使えない。可能なら実機でも 1 回測る。

#### 項目 39: GPU（Vulkan）で LM が動くことと、GPU の無い状態で CPU に落ちること（ADR 0059）

- 前提: CI の `windows-artifact` を持ち込んである（`target\release\` に `ikari-engine-host.exe`、`ikari-lm-devices.exe`、`ggml*.dll`、`llama*.dll`、`backends\`）。モデルがある。`engine.lm` は `on`。GPU の設定 `engine.lm_gpu` は書かなければ `on`（GPU があれば使い、なければ CPU）。
- やること:
  (a) `target\release\ikari-lm-devices.exe`（モデルは要らない）。出力の `lm devices=` を記録する。
  (b) エンジンを起動して（`start-engine.ps1`）、変換する。`<データ>\logs\` の `lm_load` の行の `dev=`、`gpu=`、`devices=` を見る。タスクマネージャーの GPU の欄で、メモリの使用の変化も見る。
  (c) `<データ>\settings.tsv` に `engine.lm_gpu<TAB>off` を足して、エンジンを起こし直す。`lm_load` の行が `gpu=off_setting`、`dev=cpu` になることを見る。
  (d) `settings.tsv` を戻し、`target\release\backends\ggml-vulkan.dll` を一時的に別の名前にして、エンジンを起こし直す。`devices=cpu`、`gpu=no_gpu` で動くことを見る。戻す。
  (e) 項目 38 の評価ツール（`ikari-eval.exe`）を、環境変数 `IKARI_LM_GPU=1` と `0` で流し、標準エラーの `lm inference:` の行（推論 1 回の平均、p50、p95、p99、読み込み、暖機の時間）を比べる。
- 期待する結果（**見込み**）: (a) 内蔵 GPU なら `cpu,vulkan:IntegratedGpu`（ほかの CPU の種類の `cpu` の行が並ぶことがある）。(b) `gpu=on`、`dev=gpu`。読み込みと暖機に十数秒かかる（スパイクの N150 の VM の値: 読み込み約 12 秒、最初の 1 回約 6 秒）。その間の変換は LM なしで通常どおり。(c)(d) 落ちずに CPU で動く。(e) スパイクの参考値は ADR 0059 §6。
- うまくいかないとき: (a) で `none` や CPU だけなら、`backends\` の場所（exe の隣か）、DLL の欠け、`vulkan-1.dll` の有無（`where vulkan-1.dll`）を記録する。(b) でエンジンが落ちるなら、`<データ>\logs\` の最後の行と、直前の `runmark` を記録する（ドライバの中で落ちる場合は捕まえられない。ADR 0059 §2.4）。

#### 項目 40: LM の子（`ikari-lm-host.exe`）が動き、落としてもエンジンが生き残ること（ADR 0060 Q4）

- 前提: CI の `windows-artifact` を持ち込んである（`target\release\` に `ikari-engine-host.exe` と `ikari-lm-host.exe` が並んでいる。`ggml*.dll`、`llama*.dll`、`backends\` も同じ場所）。モデルがある。`engine.lm` は `on`。
- やること:
  (a) エンジンを起動して（`start-engine.ps1`）、変換する。`Get-Process ikari-lm-host` で子がいること、`<データ>\logs\` の `lm_load` の行に `host=on host_spawns=1 host_failures=0 host_gave_up=0 host_gpu_off=0` があり、`dev=`・`gpu=`・`devices=` が項目 39 の (b) と同じであることを見る。同音異義語の並びが LM ありの並びであることを見る。
  (b) 子を落とす: `Stop-Process -Name ikari-lm-host -Force`。そのまま変換する（数回）。
  (c) 親を落とす: 子がいる状態で、エンジンをタスクマネージャーから「タスクの終了」で落とす。数秒後に `Get-Process ikari-lm-host`。
  (d) 窓なしのエンジン（`start-engine.ps1` が `-hidden` を使う構成）で起動し、変換する。コンソールの窓が出ないことを見る。
  (e) `ikari-lm-host.exe` を一時的に別の名前にして、エンジンを起こし直し、変換する。`lm_load` の行を見る。戻す。LM なしのエンジン（`ikari-engine-host-nolm.exe`）でも同じことを試す。
  (f) 往復の追加: `target\release\ikari-eval.exe --ranker full --set eval\sets\homophones-ext.tsv --lm <モデル> --dict <データ>\dict` を、`--lm-host target\release\ikari-lm-host.exe` あり・なしで流し、標準エラーの `lm inference:` の行（p50、p95、p99、max、load_ms、warm_up_ms）と、`--out-cases` の 2 つのファイルの一致を比べる。GPU（`IKARI_LM_GPU=1`）と CPU（`0`）の両方。
  (g) アイドルの解放: `settings.tsv` に `engine.idle_unload_secs<TAB>60` を足して起こし直し、変換してから 60 秒以上（GPU に載っていれば、これは効かず 1800 秒）待つ。`Get-Process ikari-lm-host`。もう一度変換する。
- 期待する結果（**見込み**）: (a) 子が 1 つ。(b) 変換は止まらず、候補は出続ける（落とした直後の 1 回は LM なしの並び）。次の変換で子が起動し直り（`Get-Process` で pid が変わる）、読み込みが済むと LM ありの並びに戻る。エンジンは落ちない。(c) 子も消える（Job Object）。(d) 窓は出ない。(e) LM つきのエンジンは、子なしで同じプロセスで動く（`lm_load` に `host=on` が無い）。LM なしのエンジンは LM なし（`lm=failed`）で動く。子を戻すと、LM なしのエンジンでも LM が効く。(f) 1 位は全件一致、`--out-cases` は一致、p50 の差はほぼ無い（スパイクの N150 の値: 171.5ms 対 171.8ms）。(g) CPU のときは子が消え、次の変換で起動し直る。
- 記録してほしい数字（締め切りの値を決めるため。ADR 0060 の「Q4 の本実装」）: (f) の `load_ms`、`warm_up_ms`、`max` を GPU と CPU で。(a) の起動から `lm_load` の行までの所要（行の 3 つ目の欄。µs）を、ビルド直後の最初の 1 回と 2 回目以降で。
- うまくいかないとき: (a) で子がいない・`host=on` が無いなら、`ikari-lm-host.exe` が `ikari-engine-host.exe` と同じフォルダにあるかを見る。`host_gpu_off=1` なら、GPU の子が 3 回続けて失敗して CPU の子に切り替わっている（`dev=cpu`）。`host_gave_up=1` なら、CPU の子でも 3 回続けて失敗している（読み込みが 60 秒を超えた、DLL が足りず起動できない、など）。`target\release\ikari-lm-host.exe --model <モデル> --gpu on` を端末で直接動かし、すぐ終わるか（DLL の欠けのダイアログ、終了コード）を見る（正常なら、読み込みのあとに読めない文字を数十バイト出して入力を待つ。これは準備完了の枠。Ctrl+Z、Enter で終わる）。(c) で子が残るなら、`Get-Process ikari-lm-host | Select-Object Id, StartTime` を記録する（親の stdin が閉じても終わらなかったことになる）。

### 5.8 セキュリティの規則（統合 PR #435 で入った実装。設計は [security-design.md](security-design.md)、ADR 0066〜0073）

ここは「規則が守られていること」の確認です。止める口の合図はインストーラの項目 51、候補窓の口の照合は項目 59〜66、設定の口の照合は項目 58・67、LM の檻の段 2 と取得の起動の入れ子の Job は項目 69〜72（#446）にあります。

#### 項目 41: データのフォルダのアクセス権（ACL）

- 前提: エンジンを一度起動して、`%LOCALAPPDATA%\ikari-ime` ができている。別の `--data-dir` で試すとき、動いている dev エンジンとパイプ名（SID とセッション ID だけで決まる）が衝突し、`os error 5`・終了コード `2` になる。先に dev エンジンのタスクを止める（`schtasks /End /TN ikari-engine-dev` と、残るプロセスの終了）。終わったら `schtasks /Run /TN ikari-engine-dev` で戻す。
- やること: (a) `icacls %LOCALAPPDATA%\ikari-ime` を見る。(b) 中のファイル（`settings.tsv`、`learning.tsv`、`logs\` など）を `icacls` で見る。(c) 既存のデータがあるフォルダ（前の版で作ったもの）で、エンジンを起動し、変換して、学習が書かれることを見る。(d) 起動のたびに付け直す処理の時間（起動から最初の変換までの時間が、前と変わらないか）を見る。
- 期待する結果（**見込み**）: (a) 継承なし。本人、SYSTEM、Administrators だけが入っている。(b) フォルダからの継承で同じ。新しくできたファイルも追従する。(c) 読み書きできる。(d) 付け直しで目に見える遅れはない。
- うまくいかないとき: `icacls` の出力を全部残す。エンジンのログの先頭 30 行も。

#### 項目 42: TIP が、高い権限のアプリとストアのアプリで素通しになる

- 前提: 通常のアプリ（メモ帳）、管理者として実行したアプリ（管理者のメモ帳、または管理者の PowerShell）、ストアのアプリ（電卓、新しいメモ帳）が使える。
- やること: (a) 通常のアプリで、読み → Space → 確定（項目 14）。TIP のログの種類が `normal` であることを見る。(b) 管理者のアプリで打つ。種類が `elevated` であることを見る。(c) ストアのアプリで打つ。種類が `sandboxed` であることを見る。(d) (b)(c) の間、エンジンのプロセス（`Get-Process ikari-engine-host`）が起動されないこと、パイプにつながりにいかないこと、設定を読みにいかないことを見る（エンジンを止めた状態から始める）。(e) (b)(c) のアプリで、モードのボタンと半角/全角キーを押す。変換が始まらないことを見て、表示がどうなるか（インジケーターの見た目、変化の有無）を記録する。(f) サインイン画面（Win+L のあと）と UAC の確認の画面で、止まったり落ちたりしないことを見る。(g) 通常のアプリと管理者のアプリを行き来して、通常側の変換が影響を受けないことを見る。
- 期待する結果（**見込み**）: (a) 変換できる。(b)(c) キーがそのまま入る（合成されない）。(d) どれも起きない。(e) 変換は始まらない（表示は記録するだけで、期待値は置かない）。(f) 止まらない、落ちない。(g) 影響なし。
- うまくいかないとき: アプリの名前、種類のログの行、固まったなら直前の操作と DebugView の最後の行を残す。

#### 項目 43: engine-host が、高い権限での起動と、高い権限の相手を断る

- 前提: 管理者の PowerShell と、通常の PowerShell が使える。製品のビルドと、`dev-knobs` のビルドの両方がある。(b) と (e) で別の data-dir を使うときは、別の `--data-dir` で試すとき、動いている dev エンジンとパイプ名（SID とセッション ID だけで決まる）が衝突し、`os error 5`・終了コード `2` になる。先に dev エンジンのタスクを止める（`schtasks /End /TN ikari-engine-dev` と、残るプロセスの終了）。終わったら `schtasks /Run /TN ikari-engine-dev` で戻す。
- やること: (a) 管理者として `ikari-engine-host.exe` を起動し、`$LASTEXITCODE` を見る。何も起動していないこと（パイプ、ロック、ログ）も見る。(b) 通常の権限で起動し、従来どおり動くこと。もう一度起動して、二重起動が `0` で静かに終わること（項目 4）。(c) 管理者のアプリ（管理者の PowerShell から、パイプを開くテスト用の呼び出し）からのパイプ接続が断られ、エンジンが落ちず、次の接続を受けること。(d) 昇格したアプリのあとで、通常のアプリで普通に変換できること。(e) 製品のビルドに `--data-dir X` を渡すと、終了コードが `64` になること。`dev-knobs` のビルドは受けること。
- 期待する結果（**見込み**）: (a) 終了コード `77`。何も作られない。(b) 従来どおり。二重起動は `0`。(c)(d) そのとおり。(e) 製品は `64`、開発用は受ける。
- うまくいかないとき: 終了コード、標準エラー、ログを残す。

#### 項目 44: LM の子の檻

- 前提: CI の `windows-artifact` を持ち込んである（項目 40 と同じ）。モデルがある。`engine.lm` は `on`。Process Explorer が使える。
- やること: (a) 子（`ikari-lm-host.exe`）のハンドルの一覧を見る。(b) 子が入っている Job の設定を見る（プロセスの上限、UI の制限）。エンジンを kill したら子も終わること（項目 40 の (c)）。(c) 子が低い整合性で動いていること（Process Explorer の Integrity の欄）。その状態で Vulkan の GPU に載るか（項目 39 の (b)）、速さが変わらないか（項目 40 の (f) の数字と比べる）、載らないとき CPU に落ちるまでの時間（最悪数分）。(d) 低い整合性の子を作る呼び出し（`CreateProcessAsUserW`）が、通常の権限の engine-host から成功すること。(e) 候補窓の Job（ADR 0062）との入れ子で問題が起きないこと。(f) CI の Windows のテスト `the_child_runs_confined` が通ること。
- 期待する結果（**見込み**）: (a) 標準の 3 つと OS の既定のものだけ。エンジンのパイプ、ログ、学習のファイルは無い。(b) プロセスの上限は 1。UI の制限がある。親が終われば子も終わる。(c) 低い整合性。GPU に載り、速さは項目 40 と同じ程度。載らなければ CPU に落ちて動く（時間を記録）。(d)(e)(f) 成功する。
- うまくいかないとき: ハンドルの一覧、Job の設定、`lm_load` の行（`dev=`、`gpu=`、`host_*`）を残す。

#### 項目 45: 製品ビルドの検査と、開発用ビルドの従来どおりの動き

- 前提: CI の `windows-artifact`（製品のビルドと dev のビルドの両方）。
- やること: (a) CI の `windows-artifact` の `scan-product` が通ること。(b) 製品の TIP の DLL に `dumpbin /dependents ikari_tip.dll` をかけて、`vcruntime140.dll` などの実行時ライブラリが出ないこと（crt-static）。(c) dev の artifact で、`start-engine.ps1`（`--data-dir` を使う）、`latency.ps1`、`dbgcap.ps1`、平らな配置の TIP が従来どおり動くこと。
- 期待する結果（**見込み**）: (a) 通る。(b) OS 標準の DLL だけ。(c) 従来どおり。
- うまくいかないとき: `scan-product` の出力、`dumpbin` の出力、動かなかったスクリプトの名前とエラーを残す。

#### 項目 46: 辞書の検証による起動時間

- 前提: 別の `--data-dir` で試すとき、動いている dev エンジンとパイプ名（SID とセッション ID だけで決まる）が衝突し、`os error 5`・終了コード `2` になる。先に dev エンジンのタスクを止める（`schtasks /End /TN ikari-engine-dev` と、残るプロセスの終了）。終わったら `schtasks /Run /TN ikari-engine-dev` で戻す。
- やること: 辞書を開く時間はログから読めないので、エンジンの起動から pipe ができるまでの時間で測る。検証が入った状態を、前の版（検証が入る前の artifact があれば）と並べる。3 回以上測り、単位を書く。
- 注: 辞書を開く時間を直接ログに出す改善は、別の issue 候補（未実装）。
- 期待する結果（**見込み**）: 悪化は数十 ms の増まで。
- うまくいかないとき: 辞書のサイズ、ディスクの種類、測った値を全部残す。

#### 項目 73: 別の利用者が持つパイプにつながない（ADR 0066 P1・P2、ADR 0068 §2 の 2）

- 前提: 利用者を 2 人使う（ふだんの利用者 A と、標準ユーザーの B。B は「ユーザーの切り替え」で別のセッションにサインインする）。A の dev エンジンは止めておく（`schtasks /End /TN ikari-engine-dev` と、残るプロセスの終了。終わったら `schtasks /Run /TN ikari-engine-dev` で戻す）。`dbgcap.ps1` で A の TIP のログを取る。**未実施**。
- やること: (a) B の側で、A のエンジンのパイプと同じ名前（A の SID と A のセッション番号を含む名前。§5.2 の項目 9 の形）の受け口を作り、A にも開ける DACL を付けておく（PowerShell の `NamedPipeServerStream` と `PipeSecurity`）。(b) A のメモ帳で日本語を打ち、Space。(c) 同じ受け口を、候補窓のパイプ（`ikari-ime-candidate-v1-…`）と設定のパイプの名前でも作り、A の変換（候補窓）と、A の設定画面（状態の欄）を見る。(d) B の受け口を閉じて A のエンジンを戻し、A の変換、候補窓、設定画面が従来どおり動くこと（項目 14、64、67）。(e) A の管理者のアプリ（素通しになる。項目 42）と、ふつうのアプリを行き来しても、(d) が変わらないこと。
- 期待する結果（**見込み**）: (b) 漢字に変換されず、キーは失われない。A の TIP のログに `engine peer rejected reason=no-peer-owner`（B のプロセスのトークンを開けないとき）か `reason=other-user` が出る。B の受け口には、つないだ直後に切断だけが見え、何も届かない。(c) 候補窓は出ず、入力は続く。設定画面は「相手を確認できません」。(d)(e) 従来どおり。
- 追加で見ること（B-01）: (g) A のセッションの中で、別の利用者（`runas /user:B`）が受け口を作る場合も、断られる（`other-user` か `no-peer-owner`）。(h) A のふだんのエンジンのパイプの持ち主が A の SID であること（昇格していないエンジンでも、管理者グループの利用者でも）。(i) 低い整合性で起こした試験用のプログラムから、エンジンのパイプを読み取りだけで開けないこと。
- うまくいかないとき: A の TIP のログの行、B の受け口が受けたバイト数（0 であるべき）を残す。B の受け口に何か届いたら、重大な発見。手順と結果を記録して止める。

### 5.9 インストーラ（骨組み。`windows/installer/`。ADR 0067 §3・§4、0068 §6.2、0070 §4、0073 §4）

前提: Inno Setup 6.6.0 以上（推奨 6.7.4 以上）で `windows/installer/README.md` のとおりに作ったインストーラ。**VM のスナップショットを取ってから**行う（Program Files、HKLM、ファイアウォール、エラー報告の設定を変える）。開発用の導入（`register.ps1`）が入っていれば、先に「7. 撤去」で外す。管理者の PowerShell で見る。

#### 項目 47: インストール先の検査

- やること: (a) 新しい VM で入れる。`icacls "C:\Program Files\ikari-ime" /t` を見る。(b) もう一度同じインストーラを流す（入れ直し）。(c) スナップショットに戻し、入れる前に `C:\Program Files\ikari-ime` を作って `icacls … /grant *S-1-5-32-545:(OI)(CI)M` で Users に書き込みを許してから入れる。(d) 入れたあと、中の 1 つのファイルだけに同じ許可を足して、入れ直す。(e) `C:\Program Files\ikari-ime` を利用者のフォルダへのジャンクションにしてから入れる。(f) `/DIR="C:\ikari"` を付けて入れる。(g) 「既定の所有者」の方針（ローカル セキュリティ ポリシーの「システム オブジェクト: Administrators グループのメンバーによって作成されたオブジェクトの既定の所有者」）を確かめる。
- 期待する結果（**見込み**）: (a) 継承のまま。書き込みは TrustedInstaller、SYSTEM、Administrators だけ。所有者は Administrators。(b) 成功。(c)(d)(e) 「管理者以外が書き込める」の画面が出て、登録・規則・自動起動をせずに終わる（終了コード 10）。(f) 「インストール先は固定」で止まる。(g) 既定（Administrators グループ）。「オブジェクトの作成者」だと、(a) が所有者の検査で落ちる見込み（そのときは結果を記録する。設計の見直しになる）。
- うまくいかないとき: `ikari-setup-helper.exe check-dir "C:\Program Files\ikari-ime"` を管理者で直接流し、出力（`untrusted <パス> <理由>`）を残す。

#### 項目 48: 登録、更新、古い版の掃除

- やること: (a) 入れたあと `reg query "HKLM\Software\Classes\CLSID" /s /f ikari_tip.dll` で `InprocServer32` が `C:\Program Files\ikari-ime\tip\<版>\ikari_tip.dll` を指すこと。入力方式の一覧に ikari を足して、メモ帳で打つ。(b) 版を上げたインストーラを、メモ帳を開いたまま流す。`tip\` の下と `InprocServer32`、入力方式の一覧と既定の入力方式が変わらないこと。開いていたメモ帳と新しく開いたメモ帳で打つ。(c) 同じ版を、メモ帳を開いたまま入れ直す（使用中の DLL を置き直さず、登録だけし直す）。(d) 再起動のあと、古い版のフォルダが消えていること。
- 期待する結果（**見込み**）: (a) 指す。打てる。(b) 新しい版のフォルダができ、登録が向き直る。古い版のフォルダは、使用中なら残る。一覧は変わらない。開いていたメモ帳は古い版、新しいメモ帳は新しい版（項目 29 と同じ）。使用中のファイルの画面は出ない。(c) 「ファイルが使用中」の画面が出ずに終わる。(d) 消えている。
- うまくいかないとき: 画面の文、`tip\` の下の一覧、`Get-Process | ForEach-Object { $_.Modules } | Where-Object FileName -like '*ikari_tip*'` の出力を残す。

#### 項目 49: ファイアウォールの規則

- やること: (a) `netsh advfirewall firewall show rule name=all | Select-String ikari-ime -Context 0,12` を見る。(b) 入れ直したあと、規則が重なっていないこと。(c) 削除のあと、規則が消えていること。(d) 送信の記録を取り（L3。ADR 0066 §6.2）、起動と入力の間に ikari のプロセスからの送信が無いこと。
- 期待する結果（**見込み**）: (a) `ikari-ime: block outbound <exe 名>` が、エンジン、LM の子、候補窓の 3 つ。方向は送信、操作はブロック、プロファイルはすべて、プログラムは Program Files の絶対パス。(b) 3 つのまま。(c) 消える。(d) 無い。
- うまくいかないとき: `netsh` の出力全体を残す。別のファイアウォールの製品が入っているかも書く。

#### 項目 50: 自動起動

- やること: (a) `reg query HKLM\Software\Microsoft\Windows\CurrentVersion\Run /v ikari-ime` を見る。(b) サインアウトしてサインインし、エンジンが動いていること（`Get-Process ikari-engine-host`。ユーザーの列と整合性のレベル）。(c) 別の利用者でサインインし、その利用者のエンジンも動くこと。メモリの量（ADR 0067 §4.1。ikari を使わない利用者の分）を記録する。(d) インストールの直後（サインアウトせず）に、メモ帳で打つ（TIP がエンジンを起動する。§4.2）。(e) 削除のあと、値が消えていること。
- 期待する結果（**見込み**）: (a) `"C:\Program Files\ikari-ime\ikari-engine-host.exe"`（引用符つき、引数なし）。(b) ふつうの権限で動く。(c) 動く。メモリの量を記録する。(d) 変換できる。(e) 消える。
- うまくいかないとき: `logs\start-failure.log` と、タスク マネージャーの「スタートアップ」の表示を残す。

#### 項目 51: 止める側（ADR 0068 §6.2）

- やること: (a) エンジンが動いている状態で、更新のインストーラを流す。エンジンの終了コードと、学習が保存されたこと（最後の変換が学習に入っている）。(b) 別の利用者のセッション（ユーザーの切り替えで残したもの）のエンジンも止まること。(c) エンジンの止める口を壊した状態（デバッガで止める、など）で流し、強制の終了に落ちること。(d) 合図から終わるまでの時間を 3 回以上測る（helper の待ち時間 `SIGNAL_WAIT_MS` の仮置き 5 秒を決めるため）。`ikari-setup-helper.exe stop "C:\Program Files\ikari-ime"` を管理者で直接流すと、数が出る。
- 期待する結果（**見込み**）: (a) 合図で終わる（`signaled` と `exited` が同じ）。学習は保存されている。(b) 合図で終わる（別のセッションへの合図が効く。効かなければ、強制の終了に落ちる。ADR 0068 §10）。(c) `terminated=1`。(d) 1 秒未満。
- うまくいかないとき: helper の出力（`found=… signaled=… exited=… terminated=… remaining=…`）と、エンジンのログの最後を残す。

#### 項目 52: Smart App Control の案内（ADR 0069 §3 の 5）

- やること: (a) Smart App Control の状態（Windows セキュリティ → アプリとブラウザーの制御）と、`reg query HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy /v VerifiedAndReputablePolicyState` の値を並べて記録する（オン、評価、オフのそれぞれ。VM で作れる分だけ）。(b) オンのとき、インストーラそのものが起動できるか。起動できれば、案内（署名が付くまで使えない）が出ること。(c) 評価のとき、案内が出ないこと、TIP の DLL が読み込まれるか。
- 期待する結果（**見込み**）: (a) オン = 1、評価 = 2、オフ = 0。(b) インストーラが止められる見込み（そのときは案内は README だけになる）。起動できれば案内が出る。(c) 出ない。読み込まれる。
- うまくいかないとき: 止めたときの画面の文と、イベント ビューアーの CodeIntegrity の記録を残す。

#### 項目 53: OS のエラー報告からの除外（ADR 0073 §4）

- やること: (a) 入れたあと、`reg query "HKLM\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications"` を見る（32 ビットの側 `WOW6432Node` にも無いか）。(b) エンジンを、タスク マネージャーから「ダンプ ファイルの作成」でなく、落とす（`windows/dev` のスクリプトか、デバッガで例外を起こす）。`C:\ProgramData\Microsoft\Windows\WER\ReportArchive` と `ReportQueue` に ikari の報告ができないこと。(c) 削除のあと、値が消えていること。
- 期待する結果（**見込み**）: (a) 3 つの exe の名前が 64 ビットの側にある。(b) できない。(c) 消える。
- うまくいかないとき: (a) の出力と、`ReportArchive` の下のフォルダの名前を残す。

#### 項目 54: 終わりの案内と削除

- やること: (a) 新しく入れたときの最後の画面（入力方式の追加の案内）。(b) 更新のときの最後の画面（ふつう・セキュリティの修正を含む `/DSecurityUpdate=1` の 2 種類）。(c) 新しい版の上に古い版を入れようとしたときの確かめの画面。`/VERYSILENT` のときは入らないこと。(d) 削除（設定 → アプリ）。使用中の TIP の DLL があるときの画面、削除のあとの `C:\Program Files\ikari-ime`、再起動のあと。(e) 削除のあと、利用者のデータ（`%LOCALAPPDATA%\ikari-ime`、ほかの利用者の分も）が残ること。(f) 入力方式の一覧の ikari の表示（削除のあと）。
- 期待する結果（**見込み**）: (a)(b) ADR 0070 §4 の文のとおり。(c) 確かめが出て、「いいえ」で止まる。`/VERYSILENT` は止まる側（入らない）。(d) 再起動を求める画面が出ることがある（Inno の標準。すぐの再起動はしなくてよい）。再起動のあと、フォルダは消える。(e) 残る。(f) 消えている（残っていたら記録する）。
- うまくいかないとき: 画面の写真と、`C:\Program Files\ikari-ime` の残りの一覧を残す。

#### 項目 55: インストーラ自身の一時の場所

- やること: インストーラを管理者で流している間に、Process Monitor で、Setup の書き込み先を記録する（パスに `Temp` を含むもの）。`is-` で始まるフォルダの `icacls` を見る。標準の利用者に昇格の資格情報を入れて流す場合と、管理者の利用者が UAC で流す場合の 2 つ。
- 期待する結果（**見込み**）: Inno 自身の部品（`setup.tmp` など）は、昇格した利用者の TEMP の、名前を推測できないフォルダに置かれる。ikari のファイル（helper、DLL、exe）は、TEMP には書かれない（`C:\Program Files\ikari-ime.setup` と、インストール先だけ）。
- うまくいかないとき: Process Monitor の記録（パスと操作の列だけ）を残す。結果でインストーラの道具を本決定にする（ADR 0067 Q3。仮）。

#### 項目 74: vc_redist が入っていない素の Windows での起動と、失敗の見え方（ADR 0081 §3 の 2、#572）

- 状態: **未実施**。ADR 0081 は案 A（`vc_redist.x64.exe` を同梱して導入）で承認済み。素の Windows での実測は、実装前の必須ではなく、この確認項目として行う。
- 前提: VC++ 2015–2022 の再頒布可能パッケージ（x64）が入っていない素の Windows 10 / 11 の VM（スナップショットを取る）。製品の形の stage 一式（`release.yml` の出力）を入れる。
- やること: (a) TIP・`ikari-engine-host.exe` が起動するか。(b) 候補窓・設定画面（.NET 10、Windows App SDK 2.6.5 の自己完結発行）が起動するか。(c) `ikari-lm-host.exe`（LM の子）が起動し、モデルで変換できるか。(d) 起動できないとき、どう見えるか（エラーダイアログ、無言の終了、終了コード、イベントビューアの「Application」）。(e) 失敗しても、IME が LM なしで変換を続けること。
- 期待する結果（**見込み**）: 未確認。(a)〜(c) が起動する想定だが、確かめていない（ADR 0081 §3 の 1・2）。(e) は ADR 0067 §4.5 の 2 と ADR 0072 §5 の仕様どおり。
- うまくいかないとき: 失敗したときの画面の写真、イベントビューアの該当行、終了コードを残す。`dumpbin /dependents` の出力が取れていれば、それも残す。

#### 項目 75: 検査に落ちたとき、削除の入口と新しい `unins000.*` が残らない（#609 D-06）

- 状態: **確認済み（2026-11-23。VM。stage はリリースの試しのビルド（run 45147748391）の導入物から組み直した版で、設定画面と候補窓を除いたもの。`ISCC` 6.7.4 の警告なし）**。
- やること: (a) 項目 47 の (c) の方法（`C:\Program Files\ikari-ime` を先に作り、`icacls … /grant *S-1-5-32-545:(OI)(CI)M`）で、空のフォルダに入れる。(b) 項目 47 の (d) の方法（正常に入れた版の `NOTICE` だけに Users の書き込みを足し、版を上げて入れる）。(c) 検査に通る普通の導入では、`reg query "HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{1CE51AC3-4849-442B-BFBE-36FA3EEC855B}_is1" /v DisplayVersion` が出る。いずれも `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` で流し、終了コードと、`reg query` と `dir C:\Program Files\ikari-ime\unins*` を見る。
- 期待する結果: (a) 終了コード 10。削除の入口（上の `reg query`）が無い。`unins000.exe`・`.dat` が無い。`tip\`、`HKLM\…\Run`、ファイアウォールの規則、エラー報告の除外、作業用の `ikari-ime.setup` も無い。(b) 終了コード 10。前の版の削除の入口と `unins000.*` はそのまま。新しい版は置かれず、登録も向き直らない。(c) 入口が登録される。
- 結果: (a) 期待どおり（空のフォルダと、別のファイルが残っているフォルダの両方。終了コード 10、入口・`unins000.*` ともに無し）。(b) 期待どおり（終了コード 10。`tip\` は前の版のまま、`InprocServer32` も前の版）。(c) 期待どおり。注意: (a) と (b) の検査は**置く前**の検査で落ちるため、「置いたあとの検査だけが落ちる」道（`RemoveNewUninstaller` が実際に消す道）は、VM では作れず**未確認**。
- 備考: 確認用の `/LOG=` は、管理者だけが書ける場所（`C:\t609\logs`）へ渡した。

#### 項目 76: 手順の途中の失敗で打ち切る、helper が使えないときの通知（#609 D-08）

- 状態: **直した。確認済み（2026-11-23。VM。直す前は × だった。下の「直す前の結果」）**。直し方は、`StepOk(RunTrusted(…, Code), Code, …)` を `Result := RunTrusted(…, Code); Result := StepOk(Result, Code, …);` の形に分けた（`RegisterTip`、`AddFirewallRules`、`ExcludeFromErrorReporting` の 3 か所）。`cargo xtask check` の静的検査が、この形を禁じる（`xtask/src/check/installer.rs`）。
- やること: (a) 偽の helper（引数を記録し、`stop` だけ終了コード 3 を返す）を stage に入れた版で、更新を流す。(b) 壊れた `ikari_tip.dll`（11 バイトの文字列。`regsvr32 /s` を直接流すと終了コード 3）を入れた版を、新規と更新で流す。終了コードと、`Run`・ファイアウォールの規則・エラー報告の除外・前の版の `tip\` を見る。
- 期待する結果: (a) 「動いているエンジンを止めきれませんでした」の通知が出る（`/SUPPRESSMSGBOXES` では、ログに `Defaulting to OK for suppressed message box` が出る）。導入は続く。(b) `DllRegisterServer` の手順で打ち切り、終了コード 11。後ろの手順（規則、`Run`、エラー報告、前の版の掃除）は走らない。
- 結果: (a) 期待どおり（`stop` の直後に通知。導入は続き、終了コード 0）。**(b) 期待と違う。新規でも更新でも、終了コード 0 で終わり、規則 4 つ、`Run`、エラー報告の除外まで書かれた。更新では、前の版の `tip\` が消され、`InprocServer32` は消えた前の版を指したまま（削除の入口の版は新しい版）**。
- 直したあとの再確認（2026-11-23）: (a) 壊れた `ikari_tip.dll` を新規で入れる → 終了コード 11。`Run`、ファイアウォールの規則（0 件）、エラー報告の除外は書かれない。`InprocServer32` は空。削除の入口と `unins000.*` は残る（登録の手順まで進んだあとの失敗なので、検査に落ちたときとは違う）。(b) 正常な版の上に、壊れた版を入れる → 終了コード 11。前の版の `tip\0.0.1` は消されず、`InprocServer32` は前の版を指したまま（打ち切りのあとも前の版は使える）。ただし巻き戻しはしない: 削除の入口の `DisplayVersion` は新しい版になり、新しい版の exe や DLL と、壊れた `tip\0.0.4` は置かれたまま（登録はされない）。`Run`、規則、エラー報告の除外は前の版のまま。(c) 項目 75、77、78 と通常の導入・削除は、直したあとも同じ結果。**未確認**: `AddFirewallRules`（`netsh`）と `ExcludeFromErrorReporting`（helper）の失敗は VM で落とせなかった。コードの形が直っていることは、静的検査で確かめる。
- 直す前の結果の原因（**推測を確かめたもの**）: `RegisterTip` が `StepOk(RunTrusted(…, Code), Code, …)` と書いてあり、`Code`（var 引数）の値が、`RunTrusted` の実行より**前**に渡されて 0 になる（引数の評価の順）。`RegisterTip` だけを `Result := RunTrusted(…, Code); Result := StepOk(Result, Code, …);` に分けたコピーでは、終了コード 11 になり、規則・`Run`・エラー報告は書かれなかった。同じ書き方の `AddFirewallRules`（`netsh`）と `ExcludeFromErrorReporting`（helper）にも当たる見込み（**未確認**。ファイアウォールのサービスは SSH から止められず、`netsh` を落とせなかった）。`AppDirReady` 側の `CheckAppDir`（`and (Code = HelperOk)`）は別の式で、正しく動く（項目 75）。
- うまくいかないとき: `ikari.iss` の `StepOk(` を検索し、第 1 引数の中に `RunTrusted(` が無いことを見る。

#### 項目 77: 更新で、前の版の LM の部品を置く前に掃除する（#609 D-10 の (a)）

- 状態: **確認済み（2026-11-23。VM）**。
- やること: 導入済みの状態で、`llama-old.dll`、`ggml-old-extra.dll`（直下）、`backends\ggml-stale-backend.dll`、`backends\sub\deep.dll`、対象外の `other-keep.dll` を置き、同じ版を入れ直す。
- 期待する結果: `llama*`・`ggml*` の古い物と、`backends\` の中の古い物（部分フォルダを含む）は消え、同梱の物は置き直される。`other-keep.dll` は残る。
- 結果: 期待どおり。直下は `ggml-base.dll`、`ggml.dll`、`llama-common.dll`、`llama.dll` と `other-keep.dll` だけになり、`backends\` は `ggml-vulkan.dll` などの同梱の物だけ（`sub\` も消えた）。終了コード 0。

#### 項目 78: 更新が「信頼できない」と判定されない（新規、同じ版、新しい版、古い版）

- 状態: **確認済み（2026-11-23。VM）**。
- やること: 順に `/VERYSILENT` で流す: 0.0.1（新規）→ 0.0.1（同じ版）→ 0.0.2（新しい版）→ 0.0.1（古い版）。そのたびに終了コード、`DisplayVersion`、`tip\` の一覧、`InprocServer32` を見る。
- 期待する結果: 最初の 3 回は終了コード 0（10 にならない）で、`tip\` と登録が新しい版に向く。古い版は、`/VERYSILENT` では確かめの既定（いいえ）で止まる。
- 結果: 期待どおり。0、0、0 で、`tip\0.0.1`、`0.0.1`、`0.0.2`（前の版のフォルダは消えた）。古い版は終了コード 1 で、`DisplayVersion`・`tip\`・登録は 0.0.2 のまま。
- 備考: 通常の導入と削除（項目 48、54 の範囲の最小限）も、このとき通った。試しのビルド（0.0.0）の導入、`unins000.exe /VERYSILENT` での削除（フォルダ、削除の入口、`Run`、規則 4 つ、エラー報告の除外が消え、`InprocServer32` も無くなる）。画面の確認（最後の案内、削除の再起動の画面）は **未確認**（SSH のみ）。

### 5.10 候補窓の本体（W3。ADR 0062、ADR 0068 §5）

第 1 段で入ったのは、候補窓の exe（`dist\target\release\candidate\ikari-candidate.exe`。WinUI 3、自己完結。第 2 段で `dist\candidate` から移した）と、確認用の送り手（`dist\target\release\ikari-candidate-send.exe`）です。項目 59〜61 は、窓を手で起動し、送り手で出して確かめます（engine-host を止めておく。動いていると、engine-host が起動した窓が先にパイプを取る）。第 2 段（項目 62〜66）で、TIP が窓に送り、engine-host が窓を起動して見張るようになりました。**第 2 段の Rust の COM の層、engine-host の起動の層、C# と CI の変更は、Windows で 1 度もビルド・実行していません**（Mac の `cargo clippy --target x86_64-pc-windows-msvc` まで）。どちらもセッション 1 で流します（`run-in-session.ps1`。SSH のセッション 0 では窓が出ない。§1.6）。

#### 項目 59: 候補窓の起動、二重起動、高い権限

- 前提: CI の `windows-artifact` を持ち込んである。
- やること: (a) `dist\target\release\candidate\ikari-candidate.exe` を起動し、プロセスが残ること、窓が見えないこと、タスクバーに出ないことを見る。(b) もう 1 つ起動し、すぐ終わること（`$LASTEXITCODE`）。(c) 1 つ目を止め、管理者の PowerShell から起動し、すぐ終わること。(d) 1 つ目の常駐のメモリ（`Get-Process ikari-candidate | Select-Object Id, WorkingSet64, PrivateMemorySize64`）を、起動の直後と 10 分後に記録する。(e) 起動から常駐までの時間（冷えたとき、2 回目）を記録する。
- 期待する結果（**見込み**）: (a) 残る。見えない。(b) 終了コード `2`（同じセッションの候補窓がもう動いている）。(c) 終了コード `2`（高い権限では受け付けを始めない）。(d)(e) 値を記録するだけ（期待値は置かない。ユーザーが見て決める）。
- うまくいかないとき: 終了コード、イベントビューアの「Application」のエラー（.NET Runtime、Application Error）を残す。起動してすぐ落ちるなら `dist\target\release\candidate` に `ikari_candidate.dll` があるかを見る。

#### 項目 60: 送り手で窓が出て、フォーカスを奪わず、決めた位置に出る

- 前提: 項目 59 の (a) の候補窓が動いている。メモ帳を前面にしてある。
- やること: (a) `ikari-candidate-send.exe 300 300` を流し、5 秒の間に窓が出て、そのあと消えることを見る（VM のコンソールの画面。Q3）。`uia-probe.ps1` か Spy++ で、前面の窓とフォーカスがメモ帳のままか、窓の矩形、拡張スタイル（`WS_EX_NOACTIVATE`、`WS_EX_TOOLWINDOW`）、最前面かを記録する。(b) 窓が出ている間に、メモ帳に打鍵が続けて入ること。Alt+Tab とタスクバーに窓が出ないこと。(c) `ikari-candidate-send.exe 300 300 above` で、窓の下端がアンカー（y=300）の上に来て、1 位が下（アンカーに近い側）に並ぶこと。(d) `ikari-candidate-send.exe 1900 1030` で、窓が作業領域（1920x1040 とみなす）の右と下にはみ出さないこと。(e) 表示の拡大を 150%・200% にして、`ikari-candidate-send.exe 300 300 below 1.5`（200% なら 2）で、窓の左上がアンカーの左下とほぼ一致し、行の高さが倍率に合うこと。(f) 送り手の 5 秒の間に、送り手を Ctrl+C で止め、窓が消えること（持ち主の接続が切れたら隠す）。(g) ライトとダークのテーマで、文字と選択の行が読めるか（見た目の仕上げは次の段なので、記録だけ）。
- 期待する結果（**見込み**）: (a) 出て消える。前面とフォーカスはメモ帳のまま。両方のスタイルが付き、最前面。(b) そのとおり。(c) 上に出て、並びは逆。(d) はみ出さない。(e) ほぼ一致（差をピクセルで記録する。許す差はユーザーが決める）。(f) 消える。(g) 記録のみ。スパイク（#368）の 150% の最初の 3 回の `topmost=False` が、ここでも出るかを記録する。
- うまくいかないとき: 送り手の出力（`connect:` で失敗するなら、候補窓が別のユーザーかセッションで動いている）、窓の矩形、拡張スタイル、DPI、前面の窓を残す。

#### 項目 61: 受けたものの上限（切り詰め）の見た目

- 前提: 項目 60 と同じ。
- やること: `ikari-candidate-send.exe 300 100 below 1 10 long`（上限より多い行と長い文字を送る）で、窓の見た目を見る。
- 期待する結果（**見込み**）: 行は 16 に、1 行は 256 文字に切り詰められて出る。窓の幅は作業領域の幅まで（長い行は窓の右端で切れて見える）で、作業領域の中に収まる。候補窓は落ちない（ADR 0068 §10 の推測の確認）。
- うまくいかないとき: 窓の矩形と、落ちたならイベントビューアのエラーを残す。上限の値は仮置きなので、見た目の印象（長すぎる・短すぎる）も備考に書く。

#### 項目 61b: 見えない文字と向きを変える文字の除去（#450 FP-5）

- 前提: 項目 61 と同じ。**別の候補窓が動いていると、パイプ名（SID とセッション番号で決まり、変えられない）が衝突する**ので、`start-engine.ps1 -Action stop` で engine-host（と、それが起動した窓）を止め、項目 59 の (a) のとおり `ikari-candidate.exe` を手で起動してから流す。終わったら手で起動した窓を止め、engine-host を起動し直す。
- やること: `ikari-candidate-send.exe 300 100 below auto 15 invisible`（`scale` を省くか `auto` にすると、その画面の表示の倍率（DPI / 96）になる。2026-11-22 以降）。7 行（zero-width、bidi-override、bidi-isolate、line-separator、control、tag、zwj-kept）が出る。送った文字列は、標準出力に `sent <名前>: ab<U+200B>cd…` の形で出る（窓のログはない。確かめるのは画面）。1 行目の候補とモデルには、注釈 `note<U+200B><U+202E>-x` も付く。
- 1 画面は 5 行で、ページ送りの手段がないので、6〜7 行目（tag、zwj-kept）と注釈は上の「全部」では見えない（2026-11-22 の確認）。**1 行ずつ送る**: 第 6 引数を `invisible:<n>`（n は 1〜7 の行番号。その行を 1 行目の候補として送る）、`invisible:note`（注釈とモデルの注釈を 1 候補だけに付けて送る）にして、`ikari-candidate-send.exe 300 100 below auto 15 invisible:1`（`scale` を画面の倍率と違う値にすると、窓が内容より小さく（大きく）なって文字が切れる。#525）から `invisible:7`、`invisible:note` まで 8 回流す（窓は流すたびに Hide されるので、続けて流してよい）。範囲外の n は `bad row` で止まる。各回の見込みは次のとおり（1: `abcdefgh`、2: `abcdefghi`、3: `abcdef`、4: `abcdef`（1 行）、5: `abcdefg`（1 行）、6: `abcd`、7: 家族の絵文字、note: 候補 `漢字` と注釈 `note-x`）。
- 期待する結果（**見込み**）: 窓の各行が、zero-width は `abcdefgh`、bidi-override は `abcdefghi`（除かれなければ `def` が逆順に見える）、bidi-isolate は `abcdef`、line-separator と control は 1 行のまま（改行・タブで行が割れたり、豆腐が出たりしない）、tag は `abcd`、zwj-kept は家族の絵文字（ZWJ は残るので 1 つに見える。フォントによっては 3 つに分かれて見える）。注釈は `note-x`（窓が注釈を描く場合）。窓は落ちない。単体テスト（`cargo test -p ikari-candidate --example candidate-send`、CI）は、同じ文字列が `limits::sanitize_text` で上の見込みになることを確かめる。
- うまくいかないとき: 画面の写真（どの行が見込みと違うか）、送り手の出力、窓が落ちたならイベントビューアのエラーを残す。

#### 項目 62: ビルドと CI（第 2 段。Windows で未ビルド）

- やること: (a) CI の Windows のジョブが通ること（候補窓のビルド、製品のビルドの検査、artifact）。artifact に `target\release\candidate\ikari-candidate.exe` があること。(b) NuGet のロック（`windows/candidate-ui/packages.lock.json`。Mac の .NET SDK 11 で作った）が、CI（.NET SDK 10。`CI=true` のとき、ロックのとおりでなければ失敗する）で通ること。(c) VM で `windows\dev\build.ps1`（.NET SDK があるとき候補窓も発行する）が通り、`target\release\candidate-publish\ikari-candidate.exe` ができること。
- 期待する結果（**見込み**）: (a)(b)(c) 通る。(b) が落ちたら、Windows で `dotnet restore windows\candidate-ui` を流してロックを作り直し、差分をコミットする（SDK の版で間接の依存が変わる可能性。推測）。
- うまくいかないとき: CI のログの最初の誤り。`DEPENDENTS.txt` に、TIP の DLL の新しい依存（`api-ms-win-shcore-scaling-l1-1-1.dll`。画面の倍率）が出ているかも記録する。

#### 項目 63: engine-host が候補窓を起動し、落ちたら起動し直し、終わると窓も終わる（ADR 0062 §5.4）

- 前提: artifact を持ち込み、`start-engine.ps1` で起動した（インストール先 `%LOCALAPPDATA%\ikari-ime-dev\install\candidate\` に窓が写る）。セッション 1 で `candidate-probe.ps1` を流す（`run-in-session.ps1`）。
- やること: (a) 起動の直後に `candidate-probe.ps1`。ikari-candidate のプロセスが 1 つ、親が engine-host、パスがインストール先の写しであること。パイプ（`ikari-ime-candidate-v1-…`）があること。(b) `Stop-Process -Name ikari-candidate -Force` で窓を落とし、約 1 秒後に新しいプロセスがあること。続けて 3 回落とすと、5 分の間は起動し直さないこと（engine-host の出力に `candidate window restarts in 300 s`）。(c) `start-engine.ps1 stop` で engine-host を止めると、候補窓も終わること。`-Foreground` で起動して Ctrl+C で止めたときも同じ。(d) 候補窓のフォルダを消して engine-host を起動すると、`no candidate window next to the engine` を出し、入力は続くこと。(e) SSH（ほかの Job の中）から起動したときと、ログオンで起動したとき（W5 の後）の両方で (a) が成り立つか（入れ子の Job。ADR 0072 §10）。(f) engine-host の起動の時間が、窓なしと比べて延びないこと。
- 期待する結果（**見込み**）: そのとおり。窓の Job の UI の制限（クリップボード、画面とシステムの設定の書き換え、サインアウト）のもとでも窓が描けること（項目 64 で見る）。
- うまくいかないとき: engine-host の標準エラーの行（`candidate window …`）、`candidate-probe.ps1` の出力、イベントビューアのエラー。Job に入れられないなら、engine-host がどの Job の中にいるか（Process Explorer の Job のタブ）。

#### 項目 64: TIP が窓に送り、先頭の 1 文字の下に出て、閉じるまで動かない（ADR 0062 §6、§7.4）

- 前提: 項目 63 の (a)。TIP を登録してある（`register.ps1`）。メモ帳で日本語を打つ。`dbgcap.ps1` で TIP のログ（`show candidates sent=… seq=… side=… source=… clipped=… anchor=x,y,w,h scale=…`）を取る。
- やること: (a) 「かんじ」と打って Space。窓が合成の**先頭の文字**の左下に出ること。`candidate-probe.ps1 -Repeat 10` で、窓の矩形、前面の窓がメモ帳のままか、最前面か。ログの `anchor` と窓の左上の差（ピクセル）。(b) ↓ で選択を動かし、ページを送っても、窓の横の位置が変わらないこと。(c) 長い読みで合成を折り返したとき、窓が 1 行目の先頭の下に出ること。(d) 画面の下端の近くで変換すると、窓が上に出て、1 位がアンカーに近い側（下）に並び、↑/↓ の向きが macOS と同じこと（`panel_shown` が効いている）。(e) 表示の拡大 150%・200% と、DPI に対応しないアプリで、(a) の差とログの `scale`。(f) 2 枚の画面で、右の画面で変換したとき、窓がその画面の作業領域に収まること。(g) Enter、Esc、ほかのアプリのクリック（フォーカスが外れる）で窓が消えること。(h) 窓の出ている間に入力方式を切り替える（TIP の `Deactivate`）と窓が消えること。(i) 位置が取れないアプリ（ログの `source=last-known` か `mouse-screen`）で、どこに出るか。
- 期待する結果（**見込み**）: (a) 左上がほぼ一致（許す差はユーザーが実測を見て決める）。フォーカスはメモ帳のまま。(b) 動かない。(c) 1 行目の先頭の下。(d) 上。向きは macOS と同じ。(e) ほぼ一致（DPI に対応しないアプリは `LogicalToPhysicalPointForPerMonitorDPI` で物理の座標に直る、が推測）。(f) 収まる。(g)(h) 消える。(i) 同じアプリの前の位置、なければマウスのある画面の横の中央、上から 3/4。
- うまくいかないとき: DebugView の行、`candidate-probe.ps1` の出力、アプリの DPI の扱い（タスクマネージャーの「DPI 認識」の列）、`clipped=1` のときの位置。

#### 項目 65: 窓が無い・落ちた・偽物のときに入力が止まらない（ADR 0062 §5.4 の決定 2、3）

- やること: (a) 変換中に `Stop-Process -Name ikari-candidate -Force`。そのまま ↓ と Space で選択を動かし、確定できること。約 1 秒後に窓が戻り、次の ↓ で、同じ位置に窓が出ること。(b) 窓が無い間は、ログが `sent=0 side=none` で、↑/↓ の向きが下に出たときと同じであること。(c) 候補窓のフォルダを消して engine-host を起動し直し、変換と確定ができること。(d) 候補窓を固める（Process Explorer の Suspend）と、打鍵が止まる時間が 1 回あたり書きの締め切り（200ms）程度で、続けて止まらないこと。(e) インストール先でない場所の `ikari-candidate.exe` を先に起動してパイプを取らせると、TIP が送らないこと（`engine peer rejected reason=…`）。
- 期待する結果（**見込み**）: そのとおり。キーは素通しにも失われもしない。
- うまくいかないとき: DebugView の行と、打鍵が止まった時間（`latency.ps1`）。

#### 項目 66: 片道の遅れ（ADR 0062 §10 の S-2 の代わり。N150 の VM は参考値）

- やること: 項目 64 の (b) を 1 分続け、TIP の往復時間（`latency.ps1`）を、窓のない状態（項目 65 の (c)）と比べて記録する。
- 期待する結果: 値を記録するだけ（期待値は置かない。窓に使える時間（ADR 0062 Q4）は、この値を見てユーザーが決める）。候補窓の側の「受けてから描くまで」は、窓の診断のログがまだ無いので測れない（次の段）。
- うまくいかないとき: 記録のみ。

#### 項目 56: 設定画面（WinUI 3）の骨組みが出る（ADR 0071 §6。W4-s3 の第 1 段）

- 前提: CI の artifact の `dist\settings\`（自己完結。`ikari-settings.exe` と `ikari_settings_core.dll`）を VM に写してある。Mac では C# をビルドしていない（CI の Windows のジョブだけ）。
- やること: (a) `ikari-settings.exe` を起動し、窓に「input」と「conversion」の画面、節の名前、設定のキー・種類・既定値の行が、`macos/Resources/settings-specs.tsv` と同じ並びで出ることを見る。(b) ライトとダークのテーマ、表示の拡大 150%・200% で、文字がぼけずに読めるか。(c) キーボードだけ（Tab、矢印、PageDown）で一覧をたどれるか。ナレーターで行が読み上げられるか。(d) 下の「入力の確認用」の欄で、ikari 自身で日本語を打てるか。(e) 管理者として起動したとき、窓を出さずに終わる（終了コード 2）。
- 期待する結果（**見込み**）: (a)〜(d) そのとおり。(e) 終了コード 2。
- うまくいかないとき: 終了コード、イベントビューアのアプリのエラー、画面の写しを残す。

#### 項目 57: 設定画面と候補窓が、自分から通信しない（ADR 0071 §6、ADR 0066 §6.2 の L3）

- 前提: 項目 56 と同じ。送信を記録する道具（Windows のファイアウォールのログ、Resource Monitor の「ネットワーク」、または Sysinternals の TCPView）を用意する。
- やること: 記録を始めてから、`ikari-settings.exe` を起動して 2 分おき、一覧をたどり、閉じる。候補窓（項目 47）も同じように動かす。
- 期待する結果（**見込み**）: どちらの exe にも、外への接続が 1 つも記録されない。
- うまくいかないとき: 接続の先（ホスト名か IP）、ポート、時刻を残す（WinUI 3 か .NET の実行時のどれが出したかを、プロセスとモジュールで見分ける）。

#### 項目 58: 設定の口が、設定画面でない相手を断る（ADR 0068 §4。S3）

- 前提: エンジン（`ikari-engine-host.exe`）が動いている。
- やること: 設定の口（`\\.\pipe\` の下の設定の名前。`windows/engine-host` の `settings_pipe`）に、PowerShell など設定画面でないプロセスからつなぎ、エンジンのコンソール（または `logs\`）を見る。
- 期待する結果（**見込み**）: つないだ直後に切られ、エンジンのコンソールに `ikari-pipe: refused a connection (not-the-peer)` が 1 行出る。エンジンのパイプ（入力）は使えたまま。インストール先の `settings\ikari-settings.exe` からはつながる（項目 59）。
- うまくいかないとき: エンジンの出力、つないだプロセスのパスを残す（ログにはパスを出さない作りなので、手で書く）。

#### 項目 67: 設定画面がエンジンにつながり、値を変え、学習をパスで書き出し・取り込みできる（ADR 0068 §4・§7。W4-s3 の第 2 段）

- 前提: 項目 56 の `dist\settings\` を、インストール先の `settings\` の下に置く（engine-host の exe の隣の `settings\ikari-settings.exe`。照合はこの置き場所だけを通す）。エンジンが動いている。学習のあるデータで試すときは、先に `<データ>` を写しておく。
- やること: (a) 設定画面を開き、状態の欄が「エンジンにつながっています」になる。(b) 「候補」の節のスイッチ（`paging.wrap`）と数（`paging.size`）を変え、結果の欄に「変更しました」が出て、`<データ>\settings.tsv` に 1 行ずつ書かれる。範囲の外の数（20）を打つと、上限に丸められる。(c) 「学習を書き出す…」で、デスクトップに書き出す。同じ名前を選ぶと、保存の窓が上書きを確かめ、確かめたら置き換わる。(d) 「学習を取り込む…」で (c) のファイルを選ぶと、先に件数が出て、確認の窓のあとに取り込まれる。壊れたファイル（中身を 1 行だけにしたもの）では、何も変えずに理由が出る。(e) ネットワークの場所（`\\<別の機械>\共有`）を選ぶと、「ネットワークの場所やデバイスは使えません」と出て、エンジンは何も読まない。(f) エンジンを止めてから「つなぎ直す」を押すと、状態の欄が「エンジンが動いていません」になり、部品は押せなくなる。(g) 設定画面を `settings\` でない場所（デスクトップ）に写して起動すると、状態の欄が「相手を確認できません」になる（エンジンのコンソールに `ikari-pipe: refused a connection (not-the-peer)`）。
- 期待する結果（**見込み**）: どれもそのとおり。(c) の書き出しは、設定の口に全文が流れない（engine-host がファイルに書く）。
- うまくいかないとき: 状態の欄と結果の欄の文、エンジンのコンソールの出力、`settings.tsv` の差分を残す（学習のファイルの中身は残さない）。

#### 項目 68: モデルの取得（`ikari-data`。Hugging Face から。ADR 0076。D-2b・D-2c）

- 前提: **Windows の低い整合性の子（`transfer`）、Job Object、`ureq` と SChannel は、Windows で 1 度もビルド・実行していない**（Mac の `cargo clippy --target x86_64-pc-windows-msvc` まで。スパイクの結果は ADR 0076 §16）。インストール先に `ikari-data.exe` を置く（`<インストール先>\ikari-data.exe`。ファイアウォールの送信ブロックはかけない）。ふだんの（中の整合性の）ログオンで、デスクトップから流す。VM の空きは 1GiB 以上。取得する前に、`inbox\` と `<データ>\fetch\` が空であることを確かめる（作業の場所は 2026-11-19 からデータの場所の下。ADR 0078 §3.1。古い `LocalLow\ikari-ime\` は使わないので、開発機では手で消す）。
- やること: (a) 中の整合性のコマンドプロンプトで `ikari-data.exe model fetch --preset zenz-small` を流し、標準出力の行（`progress`、最後に `done`）と終了コード（0）を記録する。`inbox\zenz-small\` に `manifest.json` とモデルがあり、`.partial` と `fetch\download.bin` が残っていないこと。(b) 流している間に、プロセスエクスプローラで `ikari-data.exe` が 2 つ（親は中、子は低の整合性）で、子の Job に入っていることを見る。(c) 取得の間の送信を記録する（項目 57 と同じ方法）。(d) 流している間に、もう 1 つ同じコマンドを流し、`error busy` で終わること。(e) 流している間にコマンドプロンプトを閉じ（または設定画面の核から取り消し）、2 つの `ikari-data.exe` が終わること。そのあと `ikari-data.exe cleanup` で、`fetch\` と `.partial` が消えること。(f) 回線を途中で切り（VM のネットワークを無効にする）、`error stalled`（無通信 60 秒）か `error network` で終わり、作業の場所が消えること。(g) 管理者のコマンドプロンプトで流し、`error refused` で終わること。(h) `ikari-data.exe transfer --url https://huggingface.co/ --max-size 10 --out-handle 4` を中の整合性で流し、`error refused` で終わること。`--out C:\Windows\Temp\x` を付けると、行を出さずに終了コード 2 で終わること。(i) プロキシを使う環境で `HTTPS_PROXY`（認証なし）を設定して (a) を流し、取れること。`http://` のプロキシの URL でも通ること。(j) Windows 10（TLS 1.2 まで）の VM で (a) を流す。(k) 設定画面の核の ABI（`ikari_settings_fetch_start`、`_poll`、`_cancel`）を、画面ができたあとに同じ手順で確かめる。(l) 書く先のハンドル渡し（ADR 0078 §6 の段 1）: 流している間に、`<データ>\fetch\download.bin` があり、`LocalLow\ikari-ime\` に何も作られないこと。プロセスエクスプローラで、子（低）のハンドルの一覧に `download.bin` が 1 つだけあること（権利は追記と待つことだけ）。取得のあと、エンジンが束を取り込むこと。
- 期待する結果（**見込み**）: (a) 取れて、`done zenz-small <表の sha256>`。大きいモデルでも、転送の回数の上限（5）の中で最後まで取れる（スパイクは先頭の 8MiB まで）。(b) そのとおり。(c) 送信するのは `ikari-data.exe` だけで、行き先は表のホストと転送先だけ。(d) `busy`。(e) 2 つとも終わり、残りが消える。(f) どちらかの語で終わる。(g)(h) `refused`。(i) 取れる。(j) 取れる。(l) そのとおり（子に渡したハンドルへの追記、読めないこと、切り詰められないことは Windows の CI のテストで確かめる。ふだんのログオンでは未確認）。
- うまくいかないとき: 行と終了コード、`ikari-data.exe` と子のプロセスの整合性、イベントビューアの「Application」のエラーを残す。`tool` なら、低い整合性の子を起こせていない（原因はコードの再読が要る）。仮置きの値（転送 5 回、接続 30 秒、無通信 60 秒、ディスクの余裕 256MiB、大きさの見張りの間隔 50 ミリ秒）は計測なし。この項目で、実際の時間と値を記録する。
- macOS（`ikari.app/Contents/Helpers/` に置いたとき。D-2e のあと）: 設定アプリから起動して、Sandbox の中で取れること（ADR 0076 §11 の 8）。この PR では未実装。

### 5.11 LM の檻の段 2、取得の起動、削除（#446。マージ前）

#446 で入るものの確認です。項目 68 と重なる分（低い整合性の子、プロキシ、取得、取り消し）は、そちらを見てください。

#### 項目 69: LM の子の檻の段 2（AppContainer。GPU を含む。ADR 0072 §12）

- 状態: 試験用の親からの測定は **VM で確認済み（ADR 0072 §12.2〜§12.5。セッション 1 の中の整合性の親。能力なしの AppContainer、CPU と GPU の両方）**。**製品の形の engine-host から起こした場合は未実施**。実装は #446 で入る（製品の既定は段 1 のまま。段 2 を選べるのは dev-knobs のビルドだけ）。
- 前提: dev-knobs のビルドを、Program Files の下に置く（開発用の配置 `%LOCALAPPDATA%\ikari-ime-dev\install` は AppContainer に開いておらず、DLL が読めず子が起動しない。ADR 0072 §12.5）。モデルがある。`engine.lm` は `on`。環境変数 `IKARI_LM_CAGE=appcontainer`（張れなければ起動しない）と `prefer`（張れなければ段 1）をエンジンに渡す。
- やること: (a) `appcontainer` で起動して変換し、`lm_load` の行の `box=appcontainer`、`dev=`、`gpu=` を見る。Process Explorer で子が AppContainer（整合性は低）であること。(b) 最初の採点の時間を、起動のたびに 3 回以上測る（GPU のとき。暖機で隠れて、利用者の最初の変換が遅れないか）。CPU のときの 1 回の採点（p50）も、項目 40 の値と比べる。(c) 子が、データのフォルダ、利用者のほかのファイル、外へのネットワークに触れないこと（ADR 0072 §12.4 の表と同じ結果になるか）。(d) 許可の足し方: 段 2 を使ったあと、`prefer` で段 1 に落とす（または段 2 を張れない状態を作る）と、モデルが段 1 の子で読め、`box=low` で動くこと。(e) `prefer` で段 2 が張れないとき、段 1 に落ちて変換が止まらないこと。`appcontainer` で張れないときは、LM なしで動き続けること。(f) 開発用の配置で `appcontainer` にすると、子が起動せず、LM なしで動き続けること。
- 期待する結果（**見込み**）: (a) `box=appcontainer`、GPU に載る。(b) 起動のたびの最初の採点が数秒かかる（ADR の実測は 3.8〜5.1 秒。暖機で隠す前提の実装は D-4 で、今は隠れない）。CPU の p50 は段 1 と同じ程度（実測 168ms 前後）。(c) 拒まれる。(d)(e) そのとおり。(f) 起動しない。エンジンは落ちない。
- うまくいかないとき: `lm_load` の行、子のプロセスの整合性、`icacls` でモデルの許可の一覧、イベントビューアの「Application」のエラーを残す。ドライバ（Vulkan）の版も書く。

#### 項目 70: 取得の起動の入れ子の Job（設定画面の核 → `ikari-data` → 低い整合性の `transfer`。ADR 0076 §3.2・§11 の 2・§17）

- 状態: 管理者の SSH からは **VM で確認済み（ADR 0076 §17。外の Job の UI の制限が入れ子の中にもかかる前提で、https のファイル 11,358 バイトを取れた）**。**ふだんの（中の整合性の）ログオンでの、デスクトップの設定画面からの起動は未実施**（ADR 0076 §11 の 2。前提のうち、外の Job の制限が内側の孫に効くことは Microsoft の説明に依存）。実装は #446 で入る。
- 前提: インストール先に `ikari-data.exe` と設定画面の核（`ikari_settings_core.dll`）がある。ふだんのログオンで、デスクトップから流す。
- やること: (a) 設定画面（項目 56、67 のあと。取得のボタンが付いたとき）から、またはその核の C の口（`ikari_settings_fetch_start`、`_poll`、`_cancel`）を呼ぶ確認用の道具から、モデルの取得を始める。プロセスエクスプローラで、設定画面の核が起こした `ikari-data.exe`（中の整合性、Job）と、その子の `ikari-data.exe transfer`（低い整合性、Job）が入れ子になっていること。(b) 取得が最後まで終わること。(c) 進み具合が戻り、途中の取り消しで 2 つとも終わり、`<データ>\fetch\download.bin` が次の取得の起動で消えること。(d) 子が、さらにプロセスを作れないこと、UI の操作（クリップボード、画面の設定、サインアウト）を拒まれること。(e) 設定画面を強制終了して、`ikari-data.exe` と `transfer` が残らないこと。
- 期待する結果（**見込み**）: (a)(b) そのとおり。入れ子の Job に入れられず起動に失敗する場合は、取得が「起動しない」側に倒れる（ADR 0076 §17）。(c)(d)(e) そのとおり。
- うまくいかないとき: プロセスの親子と整合性（プロセスエクスプローラ）、`AssignProcessToJobObject` の失敗を示す表示の文、終了コードを残す。

#### 項目 71: アンインストールでの檻のプロファイルの削除（ADR 0072 §12.8 の 3）

- 状態: **未実施**。`DeleteAppContainerProfile` が、無くても成功し、呼んだ利用者の分のフォルダだけ消すことは VM で確認済み（ADR 0072 §12.5）。インストーラの削除の手順から helper の `cage-profile-delete` を呼ぶ配線は、#446 で入る。
- 前提: 項目 69 を済ませ、`%LOCALAPPDATA%\Packages\` の下に檻のプロファイルのフォルダ（名前は `ikari_lm::host::CAGE_PROFILE_NAME`）ができている。
- やること: (a) アンインストールを流し、エンジンを止めたあとにプロファイルが消えること（`%LOCALAPPDATA%\Packages\` のフォルダ）。(b) プロファイルが無い状態でアンインストールしても、エラーの画面が出ないこと。(c) ほかの利用者のプロファイルが残ること（残るのは中身の空のもの。ADR の想定）。(d) 削除の画面に出る文と順序（エンジンの停止 → プロファイル → ファイルの削除）。
- 期待する結果（**見込み**）: (a)(b) そのとおり。(c) 残る（害は小さい。ADR 0072 §12.8）。(d) 失敗しても削除は続く。
- うまくいかないとき: `ikari-setup-helper.exe cage-profile-delete` を管理者でなく利用者として直接流し、出力と終了コードを残す。

#### 項目 72: Windows の CI で初めて動く検査

- 状態: **未実施**（#446 の PR 説明で、Windows CI で初めて動くものとして挙がっている）。
- やること: #446 の Windows のジョブで、(a) `ikari-sandbox` のテスト（Job、低い整合性、渡すハンドルの限定）が通ること。(b) インストーラの `.iss` の静的な検査の追加分が通ること。(c) 通信の crate の検査（配る対象ごとの `cargo tree`、`deny.toml` の `wrappers`）が通ること。落ちたら、VM で同じテストを流して違いを見る（CI のランナーのセッションや権限が、VM と違うことがある。ADR 0072 §12.3）。
- 期待する結果（**見込み**）: 通る。
- うまくいかないとき: CI のログの最初の誤りと、そのテストの名前を残す。

## 6. 結果の記録の書き方

次の表を、コピーして使います（この文書に書き込まず、issue のコメントか、別のメモに書いてください。この文書は手順書として保ちます）。1 項目 1 行。「結果」は、○（期待どおり）、△（一部、または条件つき）、×（期待と違う）、－（できなかった。理由を備考に）のどれか。「見たこと」は、事実だけを書き、推測は備考に、「推測」と付けて書きます。

```
実施日:
VM（Windows の版とビルド番号）:
ikari のコミット（git rev-parse --short HEAD）:
データ（辞書のビルド日、Mac のどのコミットで作ったか）:
配列（JIS / US / ほか）:

| 項目 | 結果 | 見たこと（数字、アプリの名前、版） | ログ・画面の場所 | 備考（推測は「推測」と付ける） |
| --- | --- | --- | --- | --- |
| 1 |  |  |  |  |
| 2 |  |  |  |  |
（項目 72 まで。番号は 47〜55 インストーラ、56〜58・67 設定画面、59〜66 候補窓、68 モデルの取得、69〜72 #446 の檻と取得の起動）

スクリプトで直したところ（ファイル、直した内容、理由）:
-
手順書で足りなかった、間違っていたところ:
-
新しく見つけたこと（想定外の挙動、固まった、など）:
-
```

- 数字（時間）は、単位と、測った回数も書きます。
- 固まったときは、固まる直前の操作と、DebugView の最後の行を残してから、スナップショットに戻します。
- すべての項目が閉じる（○、または、直した上で○）まで、1.0.0 には進みません（[ADR 0040](adr/0040-windows-feasibility.md) §2.1）。結果を見て、コードを直す項目は、別の issue にします。

## 7. 撤去

実機（VM）を元に戻すときの手順です。対象を限り、本番のデータ（学習など）には触れません。

1. エンジンを止める: コンソールで Ctrl+C、または `.\start-engine.ps1 -Action stop`（確認が出る）。
2. 管理者の PowerShell で `.\unregister.ps1 -RemoveFiles`（確認が出る。登録中の DLL の登録を解除し、`<導入先>\tip\`（`%LOCALAPPDATA%\ikari-ime-dev\install\tip\`）の中の版のフォルダだけを消す）。使用中で消せなかったフォルダは残るので、アプリを閉じるか、サインアウトして、もう一度。エンジンのコピー（`<導入先>\ikari-engine-host.exe` と隣の DLL、`backends\`）は、エンジンを止めたあと手で消す。ADR 0063 より前の `%LOCALAPPDATA%\ikari-ime-dev\tip\` が残っていれば、それも手で消す。
3. Windows の入力方式の一覧から、「ikari」が消えていることを確かめる（残っていたら、「言語のオプション」から削除する）。
4. データ: `<データ>`（`%LOCALAPPDATA%\ikari-ime` など）は、**自動では消しません**。要らなければ、中身を確かめて、手で削除してください（学習のファイルが入っています）。
5. ビルドの成果物: リポジトリの `target\` を消せば戻ります（`cargo clean`）。
6. 確実に戻したいときは、準備のときのスナップショットに戻します。

## 8. 関連

- [ADR 0040](adr/0040-windows-feasibility.md)（Windows の方針、§6 の未検証の事項、§7 の W5）、[ADR 0058](adr/0058-offline-resource-install.md)（リソースの配置。§14 の Windows への持ち越し）
- issue: #331（この手順書）、#81（Windows 対応）、#128（開発環境）
- PR（確認待ちの出どころ）: #315（W1-b）、#318（W2-b）、#326（W4-s2）、#329（再接続）、#330（W5-a）
- スクリプト: [../windows/dev/common.ps1](../windows/dev/common.ps1)、[build.ps1](../windows/dev/build.ps1)、[install-data.ps1](../windows/dev/install-data.ps1)、[register.ps1](../windows/dev/register.ps1)、[unregister.ps1](../windows/dev/unregister.ps1)、[clean-tip.ps1](../windows/dev/clean-tip.ps1)、[start-engine.ps1](../windows/dev/start-engine.ps1)、[status.ps1](../windows/dev/status.ps1)、[latency.ps1](../windows/dev/latency.ps1)
