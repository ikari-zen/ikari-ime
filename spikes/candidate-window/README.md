# 候補ウィンドウの方式のスパイク（W3-0、issue #319、ADR 0040 §2.4 / S10）

**捨てるコード。main に入れない。** ブランチ `spike/w3-0-candidate-window`。結果は ADR 0040 §2.4 に追記する（末尾の表の雛形）。

Mac では `cargo check --target x86_64-pc-windows-msvc` が通るところまで確かめてある。**動かしたことはない（すべて未確認）**。実機で最初に動かすときに、ビルドの失敗や実行時のエラーが出うる。

## 何を比べるか

同じ固定の入力（`shell-core` の `CandidatePanelModel` 相当。候補 9 個、1 ページ 5 件で 2 ページ、選択位置、選択行の注釈、右の薄いラベル、下の注釈行。`src/model.rs`）を、次の 3 方式で出す。

| 記号 | 方式 | このスパイクの実装 | 実際の構成との違い |
| --- | --- | --- | --- |
| A | WebView2（tao + wry）の別プロセス | `src/win/a_webview.rs`。HTML と CSS を描く 1 つの exe | 実際は TIP とは別の exe。TIP との往復（パイプ）は入っていない |
| B | 自前描画 | `src/win/b_gdi.rs`。Win32 の窓に GDI（二重バッファ）で描く。Direct2D / DirectWrite は**未実装** | 実際は TIP の DLL の中か別 exe。ここは単独の exe（DLL の中の場合の、アプリを巻き込む影響は測れない） |
| C | `ITfCandidateListUIElement` | `src/win/c_tsf.rs`。UI 要素のオブジェクトを作り、自分のスレッドの `ITfUIElementMgr::BeginUIElement` に渡して `show` を記録。窓は B を同時に出す | **他のアプリが UI 要素を受け取るかは、TIP として入らないと見えない（W2-b のあと）。このスパイクでは確かめられない** |

共通: `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`、前面のアプリのキャレット（`GetGUIThreadInfo`）の下に出す。TSF には接続しない。配置は `shell-core` の `panel` が計算する予定だが、ここでは「キャレットの下の左端」に置くだけ（画面の端での反転や 2 枚のモニタでの置き直しは**していない**）。

## 実機の準備

1. Windows 11 x64 の VM（miniPC 上。ADR 0040 §2.6）。スナップショットを取っておく。
2. Rust（`rustup`、`stable-x86_64-pc-windows-msvc`）と Visual Studio Build Tools（「C++ によるデスクトップ開発」）。
3. WebView2 ランタイム（Evergreen）。Windows 11 には通常入っている（**未確認**。VM のイメージによる）。A の「ランタイムが無い場合」の確認では、アンインストールするか、入っていない VM で流す。
4. 対象のアプリ: Notepad、Edge、VS Code、Word（持っていれば）。Chrome も（ADR §2.4）。
5. このリポジトリのこのブランチを clone する。

## ビルドと実行

```powershell
cd spikes\candidate-window
cargo build --release
# 方式ごと。起動すると --delay-ms の間待つので、その間に対象のアプリをクリックして文字入力の状態にする
.\target\release\candidate-window-spike.exe a --delay-ms 4000
.\target\release\candidate-window-spike.exe b --delay-ms 4000
.\target\release\candidate-window-spike.exe c --delay-ms 4000
```

オプション: `--delay-ms`（既定 3000。窓を出すまでの待ち）、`--updates`（選択を動かす回数。既定 50）、`--tick-ms`（更新の間隔。既定 100）、`--hold-ms`（更新後に窓を残す時間。既定 5000）。

出力の読み方（`RESULT` の行を集める）:

| 出力 | 意味 |
| --- | --- |
| `RESULT <A/B> first_paint` | 表示の要求から最初の描画が終わるまで（QPC）。A は JS で `requestAnimationFrame` を 2 回待った時点（実際の画面への反映より少し早い近似）。B は `WM_PAINT` の `BitBlt` 後 |
| `RESULT A webview_created` | A の WebView を作るまで（A のコールドスタートの大半の見込み。**推測**） |
| `RESULT <A/B> cold_start_total` | プロセスの作成から今まで。`--delay-ms` を含むので、`delay` を引いて読む |
| `RESULT <A/B> update ...` | 選択を動かしてから描き終わるまでの中央値、p99、最大。`--updates` を 200 以上にすると p99 が意味を持つ |
| `RESULT FOCUS unchanged / CHANGED` | 窓を出す前後で、前面のウィンドウが同じか |
| `RESULT MEM ... total` | 自分と子孫のプロセス（WebView2 の `msedgewebview2.exe` 群）の合計。`private` を「常駐のメモリ」の目安にする |
| `RESULT C ui_element show=...` | `BeginUIElement` が返した `show`（`false` なら、アプリが自前で描くので TIP は窓を出さない）。**単独のプロセスなので、Notepad などの扱いを表すものではない** |

A でランタイムが無いときは `RESULT A WebView2 runtime NOT FOUND` を出して B に切り替える（案。製品での扱いは ADR で決める。インストーラが Evergreen Bootstrapper を同梱して入れる、入れられなければ B に切り替える、の組み合わせが候補）。

コールドスタートは、再起動の直後の 1 回目（ディスクキャッシュが冷えている）と、2 回目以降を分けて記録する。

## 測るもの

各方式（A、B、C は B と同じ実装なので、C は `show` の記録と B の再確認）について。

1. **表示までの時間**: `first_paint` を、各アプリ（Notepad ほか）で 10 回ずつ。コールド（初回）と、ウォーム（2 回目以降）を分ける。更新の中央値と p99（`--updates 300`）。
2. **メモリ**: `RESULT MEM` の合計（表示直後と終了前）。
3. **フォーカスを奪わないか**: `RESULT FOCUS` と、目でも見る。窓が出ている間に、対象のアプリでキーを打って入力が続くか、タイトルバーが非アクティブ表示にならないか、Alt+Tab に出ないか、タスクバーに出ないか。
4. **高 DPI**: VM の表示を 100% / 150% / 200% にして（サインアウトが要る）、窓が崩れないか、字がにじまないか。B は `PER_MONITOR_AWARE_V2`、A は WebView2 の標準の扱い。
5. **2 枚のモニタ**: 仮想モニタを 2 枚にし、(a) 副モニタ上のアプリで正しい位置に出るか、(b) DPI の違う 2 枚（100% と 150%）で、またぐ位置でもサイズが合うか。画面の端での反転は未実装なので、端で見切れるのは既知。
6. **アプリごとの扱い**: Notepad（新しい Notepad は `GetGUIThreadInfo` のキャレットが取れないことがある。取れないときは `source=` に出る。**未確認**）、Edge、VS Code、Word。ちらつき、最前面（全画面の動画や、別の最前面のアプリの上に出るか）。
7. **窓の側を強制終了**: 窓の exe をタスクマネージャーで終了し、対象のアプリが生きているか（A、B はどちらも別の exe なので、生きている見込み。DLL の中に置いた場合は、このスパイクでは測れない）。A は、WebView2 の子プロセスだけを終了したときの窓の状態も見る。
8. **C**: `RESULT C ui_element` の値を記録する。他のアプリが UI 要素を受け取るかは、W2-b の TIP ができてから別に確かめる（ここでは「未確認」とする）。

## 判断の目安

- NFR-02（`docs/ikari-ime_spec.md`）: 「スペースから第一候補の表示まで 16ms 以内（60Hz の 1 フレーム）。基準機での実測で確定」。ADR 0040 §2.4 は「Space から 1 ページ目まで p99 16ms 以下」に窓の描画を足して体感で遅れないか、とする。
- コアの変換の時間も同じ 16ms の中に入る。窓の描画に使える割合は決まっていない。**暫定の仮置き**: 窓の更新の p99 がコアの時間と足して 16ms に収まるか、を見る。窓の割り当ての数値は、実測のあとにユーザーが決める（ここでは決めない）。
- 満たす案のうち、見た目、メモリ、アプリへの影響（巻き込み、フォーカス）で選ぶ。A が WebView2 の有無に依存すること、B が見た目を自前で作る手間（影、角丸、組版）を持つことを、結果と合わせて評価する。

## 結果の雛形（ADR 0040 §2.4 に追記する）

測った環境: Windows ___（ビルド番号）、VM、CPU/メモリ、日付、ランタイムのバージョン。

| 項目 | A: WebView2 | B: 自前描画（GDI） | C: UI 要素 |
| --- | --- | --- | --- |
| first_paint コールド（ms） | | | |
| first_paint ウォーム 中央値 / p99（ms） | | | |
| 更新 中央値 / p99（ms、n=） | | | |
| メモリ 合計 private（MB） | | | |
| フォーカスを奪うか（Notepad / Edge / VS Code / Word） | | | |
| 100% / 150% / 200% | | | |
| 2 枚のモニタ（同じ DPI / 違う DPI） | | | |
| ちらつき、最前面 | | | |
| 窓の exe を強制終了したときのアプリ | | | |
| ランタイムが無いときの扱い | | | （対象外） |
| `show` の値 | （対象外） | （対象外） | |
| 見た目の所見 | | | |
| 未確認として残ったもの | | | |

所見（推測は推測と書く）: ___
