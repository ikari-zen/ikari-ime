# ikari-npu-probe の使い方（会社 PC で手で動かす。ADR 0075 §4.5）

この文書は zip に `README.txt` として入る。モデルとツールの取り決め（IR の形、詰め方、NLL）は `README.md` の「契約」。

- 通信しない。インストーラなし、管理者の権限なし。ドライバを入れない・更新しない。
- 書くのはこのフォルダの中だけ（`report.txt`、`cache\`）。例外は AppContainer のプロファイル（確認の間だけ作り、終わったら消す。残ったら `--cleanup`）。
- **実行は Windows で一度もしていない**（Mac でのビルドと、Mac の OpenVINO CPU での試しだけ）。動かなかったら、画面に出た文をそのまま書き写す。

## 持ち込むもの（私物の機械で用意して、Google ドライブ経由で渡す）

1. ツールの zip（CI の artifact `ikari-npu-probe-<SHA>`）。中身: `ikari-npu-probe.exe`、OpenVINO の DLL 一式、llama.cpp の DLL（CPU と Vulkan。`backends\`）、`licenses\`、この文書。
2. モデルの zip（**ツールの zip には入れない**。zenz は cc-by-sa-4.0 で、再配布は別途確認）。中身を `models\` に置く:
   - `zenz-v3.2-small-Q5_K_M.gguf`（今の製品と同じ）
   - `zenz-v3.2-small-L128-fp16.xml/.bin`、`zenz-v3.2-small-L128-int8.xml/.bin`（B = 1）
   - `zenz-v3.2-small-B32-L128-fp16.xml/.bin`、`zenz-v3.2-small-B32-L128-int8.xml/.bin`（B = 32）
   - `SHA256SUMS`（変換の記録。ツールが照らして report.txt に出す）
   - IR の作り方は `README.md` の「作り方」（lane/npu-adr の `convert/convert.py`）

## 手順

1. 確かめるだけ: デバイス マネージャーの「ニューラル プロセッサ」に Intel(R) AI Boost があり、警告が無いこと（無くても、ほかの行は測れる）。
2. ツールの zip を展開する（フォルダの名前は `ikari-npu-probe` で始まったままにする。AppContainer の許可を足すフォルダを限るため）。モデルの zip の中身を、その中の `models\` に置く。
3. `ikari-npu-probe.exe` をダブルクリック（またはコマンド プロンプトで実行）。目安 10〜20 分（**推測**）。途中で自分を何度も起動し直す（装置ごと、キャッシュありの初回、檻の確認）。
4. 終わると画面に 10 行前後の要約が出る。**その画面を書き写す**（持ち出せるのは目で見たものだけ）。詳細は `report.txt`。
5. 消す: `ikari-npu-probe.exe --cleanup` のあと、フォルダを丸ごと削除。NPU のドライバ自身が別の場所にキャッシュを持つかは未確認。

## 要約の読み方

```
ikari-npu-probe 0.1.0 (<commit>) / Windows 11 … / OpenVINO 2026.4.1 / NPU ドライバ …
llama-cpu    ○ p50 …ms p99 …ms 初回 …s mem …MB 1位一致 基準
llama-vulkan ○ …                                    ← NPU が勝つべき相手
ov-npu-fp16  ○ B32 p50 …ms p99 …ms 初回 …s/キャッシュ …s mem …MB 1位一致 …%
ov-npu-int8  × <載らなかった理由>
檻   低IL: cpu○ vk○ ovc○ ovg○ npu× | AC: …
判定 ov-npu-fp16(B32): 速さ×(Vulkan の 1.20 倍) 初回○(0.80s) 一致○(100.0%) → × 採らない
```

- ○ = 載った、× = 載らなかった、- = 測れない（モデルが無い、装置が見えない）。
- p50・p99 は 1 件（候補 30 個前後）の時間。初回はコンパイル込み、「キャッシュ」は 2 回目の起動の初回。
- 1 位一致は、この機械の llama-cpu（動かなければ fixture の参照）と比べた割合。
- 檻: cpu = llama-cpu、vk = llama-vulkan、ovc / ovg / npu = OpenVINO の CPU / GPU / NPU。低IL = 低い整合性、AC = AppContainer（能力なし）。
- 判定の基準は仮置き（ADR 0075 §5）: p50 が Vulkan の 0.8 倍以下、キャッシュありの初回 2 秒以下、1 位の一致 95% 以上。

## オプション

| オプション | 意味 |
|---|---|
| `--list` | 装置とモデルの有無だけを出す（数秒） |
| `--cleanup` | AppContainer のプロファイルと `cache\` を消す |
| `--no-appcontainer` | AppContainer の確認を飛ばす |
| `--only llama-cpu,ov-npu-int8` | 測る装置を絞る |
| `--reps N` / `--budget-secs S` | 1 つの装置で全件を流す回数（既定 20）と時間の上限（既定 120 秒） |
| `--no-wait` | 終わったあと Enter を待たない |

`llama-ov-npu`（llama.cpp の OpenVINO バックエンド）は作っていない（後回し）。
