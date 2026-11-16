# windows/settings — 設定画面の核（Rust。`ikari_settings_core.dll`）

Windows の設定画面（[`windows/settings-ui`](../settings-ui/README.md)。WinUI 3、C#）が呼ぶ核（[ADR 0071](../../docs/adr/0071-settings-ui-and-fetcher.md) §2、
[ADR 0068](../../docs/adr/0068-ipc-endpoints.md) §4）。段の計画は `windows/settings-ui/README.md`。

判断の置き場所: Rust（この crate と `shell-core` の `settings`）。画面は描くことと OS の入口だけ。

| モジュール | 役目 |
| --- | --- |
| `peer` | 設定の口の相手（engine-host）の照合の入口（規則は `pipe::peer`） |
| `link` | 設定の口とのつながり（照合のあと `Hello`、読み書き、パスでの書き出し・取り込み） |
| `view` | 画面の行（部品、範囲、選択肢、今の値） |
| `edit` | 画面の操作 → `UpdateSettings`（`SettingsValues::plan` を通す） |
| `text` | 結果の欄の文（状態の欄の文は `shell-core` の `bridge::LinkStatus`） |
| `abi`、`session_abi` | C の ABI（ヘッダは `include/ikari_settings_core.h`。同じに保つ） |

- 通信の手段を持たない（NFR-17）。
- テストは Mac で回る（設定の口の相手は、メモリ上の engine-host の `SettingsHost`）。OS から値を取る `connect_installed` だけが `cfg(windows)`。
