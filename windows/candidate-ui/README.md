# windows/candidate-ui — 候補窓の画面（WinUI 3、C#）

`ikari-candidate.exe` の画面（[ADR 0062](../../docs/adr/0062-windows-candidate-window.md) §3、§7.3）。WinUI 3（Windows App SDK）、
パッケージなし（MSIX にしない）・自己完結（.NET と Windows App SDK を exe と一緒に置く）。

判断の置き場所: Rust の核（[`windows/candidate`](../candidate/README.md)）。ここは、核（`ikari_candidate.dll`）が渡す
`RenderState` を XAML で描き、核が返した位置に置くだけ（UI であるため）。パイプ、命令の状態、受けたものの上限、
並べる順、行の高さ、位置の計算は持たない。

## ファイル

| ファイル | 役目 |
| --- | --- |
| `CoreNative.cs` | 核の C の ABI（`windows/candidate/include/ikari_candidate.h` と同じに保つ） |
| `CoreHost.cs` | 核の起動と停止。核のスレッドからの知らせを UI のスレッドに渡す（最後だけを描く） |
| `CandidateWindow.cs` | 窓（前面にしない、最前面、タスクバーと Alt+Tab に出さない）と、候補の並び |
| `Win32.cs` | 窓の拡張スタイルと角丸（OS の入口） |
| `App.xaml(.cs)` | 起動。核を始められなければ何もせずに終わる |

型は、核と同じ `.proto`（`proto/ikari/…`）から `Grpc.Tools` で作る（二重に書かない）。

## ビルド（Windows だけ）

```powershell
cargo build --release -p ikari-candidate          # 核の DLL（target\release\ikari_candidate.dll）
dotnet build windows\candidate-ui -c Release      # 核の DLL を exe の隣に写す
dotnet publish windows\candidate-ui -c Release -o <出力先>   # 自己完結で配る形
```

- .NET SDK の版は `global.json`（10.0.1xx）。CI は `actions/setup-dotnet` でこの版を入れる。
- 核の DLL が別の場所にあるときは `-p:IkariCoreDll=<パス>`。
- Mac ではビルドできない（XAML のコンパイラが Windows だけ）。確かめは CI の Windows のジョブと VM（`docs/windows-real-device-check.md`）。

## 終了コード

| 値 | 意味 |
| --- | --- |
| 2 | 核を始められない（同じセッションの候補窓がもう動いている、高い権限、ABI の版の違い） |
| 3 | 扱えない例外 |

## まだ無いもの

`windows/candidate/README.md` の第 2 段の「残り」を見る（見た目の仕上げ、PNG の口、診断のログ）。起動と見張りは engine-host（`windows/engine-host/src/candidate_window.rs`）。NuGet の版は `packages.lock.json` で固定する（版を上げたら Windows で `dotnet restore` を流して作り直す）。
