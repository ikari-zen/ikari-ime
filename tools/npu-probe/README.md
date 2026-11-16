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

- 測定の 40 件の `P + n` の最大は `fixtures/` の生成の記録を見る（README の末尾「数値の一致」に書く）。L = 128 はその 2 倍以上の余裕として選んだ（ADR 0075 §4.2 の T の仮置きの最大）。

### fixtures の形式（`fixtures/homophones-40.jsonl`）

1 行 1 件の JSON:

```json
{"id": "ext-…", "prompt": "…", "prompt_tokens": [..], "candidates": ["…"], "candidate_tokens": [[..]], "nll_llama_cpu": [..]}
```

- `candidates` は、`ikari-eval --ranker full` が製品と同じ道で LM に渡した候補（その件で最初の、読み全体の採点）。
- `nll_llama_cpu` は llama.cpp の CPU（`IKARI_LM_GPU=0`、Q5_K_M）の NLL。基準（ADR 0075 §4.3 の `llama-cpu`）。
- 一致の測り方（ADR 0075 §4.4）: 1 位の一致 = NLL 最小の候補が基準と同じ件の割合。τ = 件ごとの Kendall の τ-b の中央値。NLL の差 = 候補ごとの |差| の最大と中央値。
