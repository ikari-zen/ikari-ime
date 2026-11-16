# windows/settings-ui — 設定画面（WinUI 3、C#）

`ikari-settings.exe` の画面（[ADR 0071](../../docs/adr/0071-settings-ui-and-fetcher.md) §2。WinUI 3、パッケージなし・自己完結。
候補窓（[`windows/candidate-ui`](../candidate-ui/README.md)）と同じ一式）。WebView2 は採らない。WinUI 3 にすることは、VM の
確認（ADR 0071 §6、`docs/windows-real-device-check.md` の項目 56〜59）が済むまで仮。

判断の置き場所: Rust（核 [`windows/settings`](../settings/README.md) の `ikari_settings_core.dll` と、その下の `shell-core` の
`settings`）。ここは、核が渡す設定の一覧を描くことと、OS の入口（窓、ファイルを選ぶ窓）だけ（UI であるため）。何を出すか、
範囲、既定値、検査、相手の照合、パスの検査は持たない。

## 守ること（ADR 0071 §2.3、ADR 0068 §4）

- 通信しない。通信のライブラリを呼ばない（NFR-17）。モデルの取得は、あとの段で `ikari-data` を起こすボタンだけ。
- 画面に出す文字（設定のキー、既定値、あとの段の辞書の語やアプリの名前）は、文字としてだけ出す（`TextBlock`。リンクにしない）。
- 高い権限では動かない（`app.manifest` の `asInvoker`。核も高い権限では何も始めない）。
- エンジンとは設定の口（`Port::Settings`）だけで話す。話すのは核（Rust）で、つなぐ前に相手が engine-host かを照らす。

## 段の計画（W4-s3）

### 第 1 段（この段。Mac で確かめられる Rust 側と、画面の骨組み）

| 中身 | 置き場所 | 確かめ |
| --- | --- | --- |
| B-2: engine-host は、設定の口につないできた相手が、インストール先の設定画面の exe かを照らしてから最初の枠を読む（S3） | `pipe/src/os/windows.rs`（受け口の照合の口）、`windows/engine-host/src/settings_pipe.rs` | 規則は Mac（`pipe::peer`）。OS 層は Windows の CI |
| B-2: 設定画面の核は、つなぐ前に相手が engine-host かを照らす（規則の入口だけ。つなぐのは第 2 段） | `windows/settings/src/peer.rs` | Mac |
| B-2: Windows の設定の口で、学習の全文（`ExportData` と `ImportData` の中継）を流さない | `windows/engine-host/src/settings_pipe.rs` | Mac |
| B-2: 書き出しと取り込みのパスの検査（絶対パスだけ、ネットワークとデバイスの場所を拒む）と、engine-host がそのパスを読み書きする関数 | `shell-core/src/bridge/transfer_path.rs`、`windows/engine-host/src/transfer.rs` | Mac |
| 設定の一覧（`shell-core` の `settings::specs`）を C の ABI で渡す核 | `windows/settings/`（`ikari-settings-core`。cdylib） | Mac |
| 一覧を画面ごと・節ごとに並べて出す骨組み（読むだけ。書き換えない） | `windows/settings-ui/` | Windows の CI、VM |
| CI の Windows のジョブで、核と画面をビルドし、artifact に `dist\settings` を入れる | `.github/workflows/ci.yml` | CI |

### 第 2 段（済み）

- 核が設定の口につなぐ（相手の照合のあと `Hello`、`GetSettings`、`UpdateSettings`。`windows/settings/src/link.rs`）。画面は核が渡す行
  （部品、範囲、選択肢、今の値。`view.rs`）を描き、部品（スイッチ、数、選択肢）で書き換える。書く値は `SettingsIntent` →
  `SettingsValues::plan`（`edit.rs`）。Bundle ID の並びとキー割り当ては、まだ読むだけ。
- 書き出しと取り込みは `bridge.proto` の `ExportToFile`・`ImportFromFile`（パスだけ）。ファイルを選ぶ窓は OS のもの。上書きの確認は
  保存の窓が済ませる。取り込みは先に確かめて件数を見せ、確認のあとに取り込む。
- 状態の欄（文は `shell-core` の `bridge::LinkStatus`。ADR 0068 §7、Q3）と結果の欄（文は核の `text.rs`）。
- CI の製品の検査（`scan-product`）に核の DLL を足した。

### 第 3 段以降

- ユーザー辞書、学習（状態、消去。確認のあとにだけ送る）、プロファイル、LM の案内。文言の表（多言語化は後）。
- モデルの取得のボタン（`ikari-data` を起こす。ADR 0071 §3。D-2 のあと）。

## ファイル

| ファイル | 役目 |
| --- | --- |
| `CoreNative.cs` | 核の C の ABI（`windows/settings/include/ikari_settings_core.h` と同じに保つ） |
| `CoreSession.cs` | 核のつながりを持ち、核の呼び出しを UI のスレッドの外で行う |
| `ViewRow.cs` | 核が返した画面の行（タブ区切り）を、描くための形に分ける（中身は解釈しない） |
| `SettingsWindow.cs` | 窓、状態と結果の欄、書き出しと取り込み（ファイルを選ぶ窓）、画面ごと・節ごとの部品 |
| `App.xaml(.cs)` | 起動。核の ABI の版が合わない、高い権限なら何もせずに終わる |

## ビルド（Windows だけ）

```powershell
cargo build --release -p ikari-settings-core     # 核の DLL（target\release\ikari_settings_core.dll）
dotnet build windows\settings-ui -c Release      # 核の DLL を exe の隣に写す
dotnet publish windows\settings-ui -c Release -o <出力先>   # 自己完結で配る形
```

- .NET SDK の版は `windows/candidate-ui/global.json` と同じ（このフォルダにも同じ `global.json` を置く）。
- 核の DLL が別の場所にあるときは `-p:IkariCoreDll=<パス>`。
- Mac ではビルドできない（XAML のコンパイラが Windows だけ）。

## 終了コード

| 値 | 意味 |
| --- | --- |
| 2 | 核を使えない（ABI の版の違い、高い権限）。エンジンにつながらないときは終わらず、状態の欄に理由を出す |
| 3 | 扱えない例外 |
