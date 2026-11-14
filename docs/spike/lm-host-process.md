# スパイク: LM の推論を別プロセス（ikari-lm-host）に分ける（ADR 0060 Q4）

捨てるコード。マージしない（PR は draft）。数字は参考値。

## 問い

LM の採点を、エンジンの中のスレッド（今の方式）ではなく、標準入出力のパイプでつないだ別の小さなプロセスで動かすと、
起動の時間、1 回の採点の往復、メモリがどう変わるか。

## 作ったもの

- crate `lm-host-spike`（feature `llama` / `metal` のときだけビルド。既定の `cargo build --workspace` には llama.cpp を入れない）
  - `ikari-lm-host <model.gguf>`: 読み込みが終わると `READY` を 1 行。以降、stdin の 1 行 = `プロンプト TAB 候補 TAB 候補 …`、stdout の 1 行 = 空白区切りの NLL（`{:?}` で出すので f32 が欠けずに戻る）。採点は `ikari_lm::LlamaScorer::nll` をそのまま使う。OS の隔離（Sandbox、AppContainer）は作っていない。
  - `lm-host-bench`: 同じ要求の列を (host) 別プロセスと (direct) 同じプロセスの直接呼び出しで流す。
- 要求: `eval/sets/homophones-ext.tsv` の「話題」行から、プロンプト（左文脈の末尾 20 字 + 読み。条件なし）と、候補（同じ読みの expected の全部。6 個に満たなければ他の行の expected で補う）を作る。620 件（うち先頭 20 件は warmup）。
- モデル: zenz-v3.2-small Q5_K_M（`~/Library/Application Support/ikari-ime/models/`。読むだけ）。

## 計測した環境

Mac（Apple Silicon、Metal で全層を GPU）。`cargo build --release -p ikari-lm-host-spike --features metal`。
計測の前にモデルのファイルを一度読んで、ページキャッシュに載せている。順序の影響を見るため、host を先に流す場合と direct を先に流す場合の両方を測った。
`--gap-ms` は要求の間に空ける時間（0 = 連続、200 = 打鍵の合間を想定）。

## 結果（Mac、Metal）

| 条件 | 方式 | 起動〜読み込み完了 | 往復 p50 | p95 | p99 | max | n |
|---|---|---|---|---|---|---|---|
| 連続、host 先 | host | 226 ms | 11.41 ms | 14.60 ms | 19.92 ms | 30.94 ms | 600 |
| 連続、host 先 | direct | 70 ms | 11.24 ms | 13.96 ms | 15.71 ms | 52.39 ms | 600 |
| 連続、direct 先 | host | 117 ms | 11.41 ms | 14.40 ms | 15.88 ms | 19.44 ms | 600 |
| 連続、direct 先 | direct | 69 ms | 11.67 ms | 14.36 ms | 15.69 ms | 30.28 ms | 600 |
| 間 200 ms、host 先 | host | 77 ms | 27.98 ms | 57.31 ms | 73.16 ms | 207.63 ms | 500 |
| 間 200 ms、host 先 | direct | 72 ms | 26.60 ms | 46.65 ms | 64.67 ms | 95.06 ms | 500 |
| 間 200 ms、direct 先 | host | 132 ms | 31.55 ms | 40.05 ms | 44.53 ms | 55.66 ms | 500 |
| 間 200 ms、direct 先 | direct | 101 ms | 28.18 ms | 37.36 ms | 39.00 ms | 55.31 ms | 500 |

- 「起動〜読み込み完了」は、host は `spawn` から `READY` の行を読むまで、direct は `LlamaScorer::load` の時間（llama.cpp のバックエンド初期化を含む）。
- host と direct の NLL は全要求で完全に一致した（最大差 0）。
- 同じ要求のペアごとの差（host − direct）: 連続で p50 0.0〜0.1 ms、p99 4〜9 ms。間 200 ms で p50 2.6〜3.1 ms、p99 24〜49 ms。
- 最初の 1 回（ビルド直後の初回の実行。Metal のシェーダの準備が冷えていたと思われる）だけ、host の往復が p50 30 ms、direct が 11 ms と大きく離れ、起動も 1073 ms / 115 ms だった。同じ順序（host 先）で 2 回目以降は再現しなかった。上の表には入れていない。

### メモリ（RSS。`ps -o rss=`）

| | 値 |
|---|---|
| host の子プロセス（モデル読み込み後） | 263〜303 MB |
| 直接方式のプロセス（読み込み後） | 263〜372 MB（読み込み前の同じプロセスは 78 MB。ばらつきあり） |

分けると、モデルの分のメモリは子に移るだけで、全体はほとんど変わらない。エンジン側は軽くなる（読み込み前の 78 MB はベンチ自体の分で、エンジン本体の数字ではない）。
macOS の RSS は mmap と Metal の割り当ての数え方があいまいで、測るたびにばらつく。

## 分かったこと

- パイプと文字列の往復そのものの追加は小さい。連続で動かした 2 つの順序では p50 の差は 0.1 ms 以下、p95 で 1〜3 ms。採点自体（11 ms）の 1% 前後。
- モデルの読み込みを含む起動は、別プロセスで約 80〜230 ms（直接は 70〜100 ms）。プロセスを起こす分と、バックエンドの初期化が子で重なる分が足される。
- 結果の数値は直接方式と一致する（文字列の往復で失われない）。
- この計測は「同じマシンの上で、ほかに何もしていない」条件で、CPU/GPU の取り合いは入っていない。

## 推測（確かめていない）

- 間を空けた条件で host のほうが p95 以降が大きいのは、順序を替えると差が小さくなる（host 先は p99 差 49 ms、direct 先は 24 ms）ので、主因は別プロセス化より、マシンの電力状態（GPU や CPU のクロックが落ちる）の揺れだと思う。ただし p50 で 2.6〜3.1 ms の差は両順序で出たので、別プロセスの起床の遅れ（スケジューラの切り替え）が数 ms 入る可能性はある。
- Windows（CPU のみ）は、採点に 1 回数十〜数百 ms かかるはずなので、パイプの分（Mac で 0.1 ms 前後）は相対的にもっと小さいと思う。ただし Windows のパイプと CreateProcess のコスト、ウイルス対策ソフトの走査（子プロセスの起動時）は未測定。
- 落ちたときの扱い（エンジンが LM なしで動き続ける）は、パイプが閉じたことで検知できる。今回は作っていない。

## まだ測っていないこと

- N150 の VM（Windows）での数字。
- 隔離（Sandbox、AppContainer）を付けたときの起動と往復。隔離の追加で起動が伸びる可能性がある。
- エンジンが同時に別の仕事をしているとき（CPU の取り合い）の p99。
- 長時間の常駐（アイドルの解放、再起動）。

## Windows で測る手順

CI の artifact（`ikari-windows-dev-<SHA>`）の `target/release/` にある `ikari-lm-host.exe` と `lm-host-bench.exe`、それと `eval/sets/`（artifact に入っている）を使う。

```
lm-host-bench.exe --model <zenz-v3.2-small-Q5_K_M.gguf のパス> --set eval\sets\homophones-ext.tsv --n 600
lm-host-bench.exe --model <同上> --set eval\sets\homophones-ext.tsv --n 500 --warmup 10 --gap-ms 200
```

`--direct-first` で順序を替えて、各条件を 2 回流す。`ikari-lm-host.exe` は `lm-host-bench.exe` と同じフォルダに置く（別の場所なら `--host <exe>`）。RSS は PowerShell の `Get-Process` で取る（初回に数百 ms かかる）。
