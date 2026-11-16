# npu-probe: zenz-v3.2-small を OpenVINO で動かす測定の道具（スパイク。ADR 0075）

捨てるコード。製品の `lm` には入れない。モデルの実物と IR はリポジトリに入れない（zenz は cc-by-sa-4.0。再配布は別途確認）。

- `convert/`: GGUF → HF GPT-2（FP16）→ OpenVINO IR（固定形）の変換と、Mac の OpenVINO CPU での数値の確かめ（Python）
- `fixtures/`: 測定の 40 件（プロンプト、候補、トークン列、llama.cpp CPU の NLL）

## 契約（Rust の測定ツールはこれを読んで実装する）

ここに書いたものは、レーンの間の取り決め。変えるときは ADR 0075 のレーンに相談する。

### モデルの IR

| 項目 | 値 |
|---|---|
| 元 | `zenz-v3.2-small-Q5_K_M.gguf`（`scripts/lib.sh` の `ZENZ_MODEL_FILE`。製品と同じ）を FP32 に戻して HF の `GPT2LMHeadModel` にしたもの |
| ファイル | `zenz-v3.2-small-L128-fp16.xml` / `.bin`（重みを FP16 に圧縮）、`zenz-v3.2-small-L128-int8.xml` / `.bin`（重みだけ INT8。NNCF の weight compression） |
| 入力 | 名前 `input_ids`、`i64`、形 `[1, 128]`（静的）。attention_mask・position_ids は**無い**（位置は 0..127 の固定） |
| 出力 | 名前 `logits`、`f32`、形 `[1, 128, 6000]`（静的） |
| 語彙 | 6000（`[UNK]`=0、`[PAD]`=1、`<s>`=2、`</s>`=3） |

- 計算の精度は装置に任せる（NPU は FP16）。入出力は上のとおり `i64` と `f32` で、装置によらず同じ。
- IR は機械に依らない。sha256 は `convert/` の実行の記録（`SHA256SUMS`）に出し、ADR 0075 に書く。

### トークン化

- 製品の `LlamaScorer` と同じ: llama.cpp の vocab で `tokenize(text, add_bos = false, parse_special = true)`（GGUF の `tokenizer.ggml.add_bos_token` が false なので **BOS は付けない**）。
- プロンプトは `lm::prompt::build_prompt_for(ModelKind::ZenzV3, …)` の文字列（`EE03 プロフィール EE04 話題 EE02 左文脈 EE07 右文脈 EE00 読み(カタカナ) EE01`）。
- 終わりのトークンは `</s>`（id 3）。GGUF の `eos_token_id`（2 = `<s>`）では**ない**（`find_end_token` が `</s>` を探す）。
- スパイクでは、トークン列は `fixtures/` に入れたもの（llama.cpp で作ったもの）をそのまま使ってよい。自前でトークン化するなら、`fixtures/` の列と全件一致することを先に確かめる。

### 1 回の推論の詰め方と NLL

候補 1 つにつき 1 回流す（形が `[1, 128]` のため）。

1. `p` = プロンプトのトークン列（長さ P）、`c` = 候補のトークン列（長さ n、1 ≤ n < 96）。
2. 入力 = `p ++ c` を左から詰め、右の残りを `[PAD]`（1）で埋める。GPT-2 は因果なので、右の詰め物は前の位置の logits に影響しない（attention_mask は要らない）。
3. `P + n > 128` の候補は流さず、NLL を `+inf`（「長すぎ」）とする。40 件にはない（最長は §「L の決め方」）。
4. NLL = `-log_softmax(logits[P-1])[c0] - Σ_{j=1}^{n-1} log_softmax(logits[P+j-1])[c_j] - log_softmax(logits[P+n-1])[3]`。log_softmax は 6000 語の全体で、f32 で計算する（`LlamaScorer::log_prob` と同じ）。
5. 候補の順位は NLL の小さい順（同じ値は元の順）。

### L の決め方

- 測定の 40 件の `P + n` の最大は 66（`make_fixtures.py build` の出力）。L = 128 は約 2 倍の余裕で、ADR 0075 §4.2 の T の仮置きの最大と同じ。
- 製品のプロンプトは最大 512 トークン（`MAX_PROMPT_TOKENS`）なので、L = 128 はスパイクの 40 件のための値。製品の形は採用が決まってから決める。

### fixtures の形式（`fixtures/homophones-40.jsonl`）

1 行 1 件の JSON:

```json
{"id": "ext-…", "prompt": "…", "prompt_tokens": [..], "candidates": ["…"], "candidate_tokens": [[..]], "nll_llama_cpu": [..]}
```

- `candidates` は、`ikari-eval --ranker full` が製品と同じ道で LM に渡した候補（その件で最初の、読み全体の採点）。
- `nll_llama_cpu` は llama.cpp の CPU（`IKARI_LM_GPU=0`、Q5_K_M）の NLL。基準（ADR 0075 §4.3 の `llama-cpu`）。
- 一致の測り方（ADR 0075 §4.4）: 1 位の一致 = NLL 最小の候補が基準と同じ件の割合。τ = 件ごとの Kendall の τ-b の中央値。NLL の差 = 候補ごとの |差| の最大と中央値。
- `nll_llama_cpu` が `null` の候補（llama.cpp が採点しなかったもの）は比べない。40 件にはない。

## 作り方（Mac。版は `versions.txt` に出る）

venv とモデルと IR は、リポジトリの外（作業用のディレクトリ）に置く。Python は `-I` で動かす。

```sh
python3.12 -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install torch transformers accelerate gguf openvino nncf scipy numpy
# 1. IR（GGUF は data/raw/zenz/ のもの。scripts/fetch-data.sh --only zenz）
"$WORK/venv/bin/python" -I tools/npu-probe/convert/convert.py data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf "$WORK/ir"
# 2. 確かめ（OpenVINO の CPU。--precision f16 は NPU の計算の精度に近い）
"$WORK/venv/bin/python" -I tools/npu-probe/convert/check.py tools/npu-probe/fixtures/homophones-40.jsonl \
  "$WORK/ir/zenz-v3.2-small-L128-fp16.xml" "$WORK/ir/zenz-v3.2-small-L128-int8.xml" --precision f32
```

fixtures を作り直すとき（変換や辞書が変わって、LM に渡る候補が変わったとき）:

```sh
python -I tools/npu-probe/convert/make_fixtures.py select eval/sets/homophones-ext.tsv "$WORK/sub40.tsv"
git apply tools/npu-probe/convert/dump-nll.patch   # 一時。llama.rs の nll が 1 回ごとに JSON を 1 行書く。CPU に固定する。コミットしない
cargo build --release -p ikari-eval --features lm
IKARI_NPU_DUMP="$WORK/dump.jsonl" target/release/ikari-eval --set "$WORK/sub40.tsv" --ranker full \
  --lm data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf --lm-budget-ms 60000 --lm-deadline-ms 60000 --repeat 1
git checkout lm/src/llama.rs
python -I tools/npu-probe/convert/make_fixtures.py build "$WORK/sub40.tsv" "$WORK/dump.jsonl" tools/npu-probe/fixtures/homophones-40.jsonl
```

- GGUF → HF は transformers の GGUF 読み込み（Q5_K・Q6_K を FP32 に戻す）。HF → IR は `torch.export` → `ov.convert_model`（TorchScript の trace は transformers 5 のマスクの組み立てで落ちる）。INT8 は NNCF の `compress_weights(INT8_ASYM)`（重みだけ。50 層すべて per-channel）。

## 数値の一致（2026-11-16、Mac の CPU。OpenVINO 2026.4.1、transformers 5.5.4、torch 2.15.0、nncf 3.4.0）

基準は `nll_llama_cpu`（llama.cpp の CPU、Q5_K_M）。40 件、候補 1233 個。

| 比べたもの | 1 位の一致 | τ の中央値（最小） | \|NLL の差\| の中央値（最大） |
|---|---|---|---|
| HF（torch、FP32 に戻した重み） | — | — | 0.135（1.454） |
| IR FP16、OpenVINO CPU、計算 f32 | 40/40 | 0.987（0.953） | 0.135（1.452） |
| IR FP16、OpenVINO CPU、計算 f16 | 40/40 | 0.986（0.948） | 0.143（1.377） |
| IR INT8、OpenVINO CPU、計算 f32 | 40/40 | 0.987（0.953） | 0.143（1.274） |
| IR INT8、OpenVINO CPU、計算 f16 | 40/40 | 0.982（0.944） | 0.184（2.049） |

- IR（FP16、計算 f32）と HF（torch）の差は、先頭 8 件で中央値 0.001、最大 0.014。**書き出しは正しい**。
- llama.cpp との差（中央値 0.135）は、書き出しでなく、llama.cpp の CPU の計算（量子化した重みと、活性を量子化して掛ける内積）と、FP32 に戻した重みの計算との差（**推測**。HF と IR が同じ差を持つことからの推定で、llama.cpp の中は見ていない）。順位は 1 位が全件で一致する。
- 所要（参考。Mac の CPU、候補ごとに 1 回）: 1 件（約 31 回）の p50 は 計算 f32 で約 2.1 秒、f16 で約 1.1 秒。llama.cpp の CPU（KV を複製してまとめて 1 回）は 1 件の p50 が 38ms。B = 1・L = 128 で候補ごとに流す形は、Mac の CPU では桁で遅い。NPU で測る意味は残るが、B > 1 の形が要る見込みが高い（ADR 0075 §4.2）。

| ファイル | 大きさ | sha256 |
|---|---|---|
| `zenz-v3.2-small-Q5_K_M.gguf`（元） | 73,871,936 | `29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673` |
| `zenz-v3.2-small-L128-fp16.xml` | 613,044 | `ef4d12c7b1a7472a9c32dbffe57e68008c5b5085d68e61448a28e14d5f78ef4e` |
| `zenz-v3.2-small-L128-fp16.bin` | 188,758,025 | `4e8f9e0da8cc8aac8035ada1d5e3e339d86518ff7c00fc858b31357c268c26a5` |
| `zenz-v3.2-small-L128-int8.xml` | 648,710 | `fe720b83ef5b5279bb029551b59f36566be4a3e850f3816cd3b4a7a31e2443c4` |
| `zenz-v3.2-small-L128-int8.bin` | 95,331,497 | `98113e07fc6d8c5c88960553dd91c7a9211b62bd2f0742512a83b74b8a2d2eb5` |

- IR の sha256 は、同じ版で作り直して一致するかは確かめていない（未確認）。Rust のツールは、上の sha256 でなく、渡した zip の中の `SHA256SUMS` を正とする。
