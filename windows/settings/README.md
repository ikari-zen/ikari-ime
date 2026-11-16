# windows/settings — 設定画面の核（Rust。`ikari_settings_core.dll`）

Windows の設定画面（[`windows/settings-ui`](../settings-ui/README.md)。WinUI 3、C#）が呼ぶ核（[ADR 0071](../../docs/adr/0071-settings-ui-and-fetcher.md) §2、
[ADR 0068](../../docs/adr/0068-ipc-endpoints.md) §4）。段の計画は `windows/settings-ui/README.md`。

判断の置き場所: Rust（この crate と `shell-core` の `settings`）。画面は描くことと OS の入口だけ。

| モジュール | 役目 |
| --- | --- |
| `peer` | 設定の口の相手（engine-host）の照合の入口（規則は `pipe::peer`） |
| `abi` | C の ABI（ヘッダは `include/ikari_settings_core.h`。同じに保つ） |

- 通信の手段を持たない（NFR-17）。
- 第 1 段の ABI: 版、高い権限の確認、設定の一覧（`shell-core` の `specs_tsv`）、バイト列の返却。
- 設定の口につなぐ（`Hello`、`GetSettings`、`UpdateSettings`）のは第 2 段。
