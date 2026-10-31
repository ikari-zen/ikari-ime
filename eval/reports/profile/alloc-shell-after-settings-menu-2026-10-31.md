# 確保の前後: 設定とメニューの複製（issue #161、直す順番の 1 の前半）

- 前: 親コミット e3389df（main 相当）。後: 83fa3c0（`perf/shell-core-settings-menu-clones`）。どちらも `cargo xtask perf alloc --detail`（repeat 200、`dispatch` 250600 回）と `cargo xtask perf shell --impl rust`（5 回の中央値）
- 負荷平均（測定の前）: 前 3.92、後 2.92（P1 は前 3.9、後 3.1 前後）。静かな状態ではない（3 を少し超える）ので、時間は参考
- 直したこと: (1) `SettingsStore` と `SettingsBackend` が設定を `Arc<SettingsSnapshot>` で持ち、状態機械へ渡すときに複製しない（`make_client`、`sync_settings`）。(2) 英語のモードの打鍵（`Text`）で設定と入力先の ID を借りる（以前は毎打鍵で複製）。(3) `menu::build` の `Vec` に容量を取る

## `dispatch` 1 回あたり

| | 前 p50 | 後 p50 | 前 p99 | 後 p99 | 前 max | 後 max |
|---|---:|---:|---:|---:|---:|---:|
| 確保の回数 | 15 | 15 | 144 | 144 | 194 | 180 |
| 確保のバイト数 | 380 | 380 | 9243 | 9174 | 78070 | 76999 |

## 確保の場所（1 回の dispatch あたりの回数）

| 場所 | 前 | 後 |
|---|---:|---:|
| `SettingsSnapshot::clone`（types.rs:542、546） | 1.16 + 0.73 | 0（消えた） |
| `menu::build`（menu.rs:131。`Vec` の伸長） | 0.84（125 MB） | 0（消えた） |
| `menu::radio`（menu.rs:115。項目名の `String`） | 1.40 | 1.40 |
| `MenuItem::clone`（types.rs:333） | 1.15 | 1.15 |

## P1（キー 1 回。5 回の中央値）

| | 前 | 後 |
|---|---:|---:|
| p50 | 834 ns | 875 ns |
| p99 | 7.79 us | 7.83 us |

差は測定の揺れの範囲（min〜max が重なる）。

## 分かったこと

- 設定の複製とメニューの組み立ては、測り手が `dispatch` ごとに新しい `Shell` を作る（最初の `activate` と最初の `finish`）ことによる、**起動 1 回あたり**の費用だった。本番では打鍵ごとではなく、入力先を切り替えたとき（設定の複製）と、設定・LM・分野が変わったとき（メニュー）にだけ起きる（メニューは `menu_inputs` で、すでに版が同じなら作り直さない）。確保の回数の p50、p99 は変わらない。
- 打鍵ごとの複製は、英語のモードの `Text` だけだった（設定と入力先の ID を毎回複製）。測り手のシナリオには少ない。
- 残り（`menu::radio` と `MenuItem::clone`）は、`MenuItem.title: String` と `ShellAction::ShowMenu(MenuModel)`（公開の型）が所有する文字列を求めるため、型を変えずには減らせない。メニューが変わったときだけの費用なので、型を変えてまでは直さない。
