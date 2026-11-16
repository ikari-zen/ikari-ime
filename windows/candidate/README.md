# windows/candidate — 候補窓（`ikari-candidate.exe`）の核

[ADR 0062](../../docs/adr/0062-windows-candidate-window.md) の候補窓の、判断と通信を持つ Rust の核（C の ABI の DLL `ikari_candidate.dll`）。
画面（WinUI 3、C#）は [`windows/candidate-ui`](../candidate-ui/README.md) にあり、この DLL だけを呼ぶ。

判断の置き場所: Rust（この crate）。命令の状態（最後だけ、持ち主の接続、切れたら隠す、並べる順）、受けたものの上限（ADR 0068 §5）、
行の高さ、横の位置。C# は、核が渡す `RenderState` を描くだけ（UI であるため。macOS の Swift と `shell-ffi` と同じ形）。

## 中身

| モジュール | 役目 | 試験 |
| --- | --- | --- |
| `command` | 命令の型（`proto/ikari/candidate/v1/candidate.proto`） | — |
| `metrics` | 行の高さ（DIP）。行数と行の種類だけで決まる | Mac |
| `placement` | 横の位置（`shell-core::panel::layout_panel_y_down` と同じ規則） | Mac |
| `limits` | 受けたものの上限（行の数、1 行の長さ、座標と倍率）。外れたら切り詰めるか捨てる（ADR 0068 §5。B-1） | Mac |
| `state` | 命令の状態。`seq` と世代、持ち主の接続、切れたら隠す、並べる順。`RenderState` を作る | Mac |
| `receiver` | パイプの受け手（`pipe::Server`、候補窓の口の上限）。接続ごとの番号、切れた知らせ | Mac（`pipe::mem`） |
| `abi` | C の ABI（`include/ikari_candidate.h`）。Windows では候補窓のパイプで受ける | Mac（ABI の数の関数）、Windows の CI |

## W3 の実装の段階（レーン lane/w3。2026-11-16）

ADR 0062 §9.1 のレーン（L1〜L4）のうち、Mac で確かめられる Rust の側と、C# の骨組みと CI のビルドを、この段でやる。

**この段でやる**
1. L2 の核（この crate）: `metrics`、`placement`、`limits`（B-1）、`state`、`receiver`、`abi` の本実装。純粋な部分は Mac の `cargo test`。
2. L1 の純粋な部分（`windows/tip/src/candidate_send.rs`）: `plan_show` / `plan_move` / `plan_hide`（上下の決定と `panel_shown` に渡す内容）、
   送り口（`Hello` を先に送る、`seq` と世代を付ける、送れなければ接続を捨てる、つなぎ直しの間隔）。
3. B-1 の TIP の側（S4。ADR 0068 §5）: 候補窓につないだら、送る前に相手がインストール先の `ikari-candidate.exe` かを照らす
   （`com::connector` の照合をエンジンと共有。違えば閉じて窓なしで続ける）。
4. L3 の骨組み（`windows/candidate-ui`。C#、WinUI 3、パッケージなし・自己完結）: 核を起動し、`RenderState` を受けて候補を並べるだけの窓
   （前面にしない、最前面、タスクバーと Alt+Tab に出さない。スパイク #368 で確かめた作法）。
5. CI: Windows のジョブで、核の DLL と C# の画面をビルドする（`actions/setup-dotnet` は SHA で固定）。製品のビルドの検査に核の DLL を足す。
6. 実機の確認の項目（`docs/windows-real-device-check.md`）に、この段で入るものの「見込み」を書く。

**この段でやらない（次の段）**
- L1 の COM の層: アンカー（合成の先頭の 1 文字、閉じるまで固定、DPI の変換、作業領域。ADR 0062 §6）、`Shell::panel_shown` の呼び出し、
  フォーカスが外れたとき・`Deactivate` の `Hide`。Windows でしか確かめられないので、実機の確認と一緒に入れる。
- L4: engine-host が候補窓を起動して見張る（Job Object。`windows/engine-host/src/candidate_window.rs`）、`build.ps1`・`start-engine.ps1`、
  `candidate-probe.ps1`、配布の一覧。
- L3 の残り: ライトとダーク、Acrylic、角丸、開発者の行の見た目、PNG の口（開発用のビルドだけ）、窓の高さを TIP と合わせる確認。
- 片道の遅れの計測（ADR 0062 §10 の S-2 の代わり）。値は仮置きのまま（`metrics` と `limits`）。
- W3-c（`ITfCandidateListUIElement`）。

## 第 2 段（レーン lane/w3b。2026-11-16）

**この段でやったこと**（Windows では未ビルド。実機の項目 62〜66）
1. L1 の COM の層: `windows/tip/src/candidate_view.rs`（純粋。アンカーを `Hide` まで固定、取れないときの順、
   送れなかったあとの `Show` の送り直し）と、`com::screen`（先頭の 1 文字の `GetTextExt`、物理の座標への変換、
   画面の作業領域と倍率）、`com::edit`（`TsfOp` を `candidate_view` に渡す）、`com::service`（送れたら
   `ShellEvent::PanelShown` を返す。フォーカスが外れたとき・`Shell` を捨てたときに隠し、`Deactivate` で閉じる）。
2. L4: `windows/engine-host/src/candidate_window.rs`（起動し直す間隔と回数の上限、見張りの繰り返し。純粋）と `os.rs`
   （Job Object、`CreateProcessW`）。`windows/dev/build.ps1`（候補窓の発行）、`common.ps1`（インストール先への写し）、
   `candidate-probe.ps1`。
3. C# の依存: `packages.lock.json`（CI でロックのとおりかを見る）、Dependabot の `nuget`、`THIRD_PARTY_NOTICES.md`。

**残り**
- L3: ライトとダーク、Acrylic、角丸、開発者の行の見た目、PNG の口（開発用のビルドだけ）、窓の高さを TIP と合わせる確認。
- 窓の診断のログ（受けた `seq`、描いた矩形、かかった時間）と、窓の側の片道の遅れ。
- アンカーの `clipped` の扱い（ログに出すだけ。ADR 0062 §6.1）、`GetGUIThreadInfo` のキャレット（S2 の結果で）。
- W3-c（`ITfCandidateListUIElement`）。

**仮置きの値**（計測なし。PR で明示してユーザーに確認する）: 行の高さ（`metrics`。macOS の値を写した）、受け取りの上限（`limits`）、
窓を起動し直すまでの間隔（`windows/engine-host/src/candidate_window.rs` の `RESTART_DELAY` = 1 秒）。
Job の UI の制限（クリップボード、画面とシステムの設定の書き換え、サインアウト）で WinUI 3 が描けるかは推測。
