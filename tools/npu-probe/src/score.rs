//! 形を固定した採点（ADR 0075 §3「C」、§4.2）。KV キャッシュを使わず、「プロンプト + 候補 + EOS」の列を
//! B 行 × T 個に詰めて 1 回流し、各位置の確率から NLL を出す。OpenVINO に依らない部分（詰め方と計算）だけを置く。
//!
//! NLL の定義は `ikari_lm::LlamaScorer::nll` と同じ: `-Σ log P(候補のトークン + EOS | プロンプト)`（nat）。
//! 位置 `t` の logits は、位置 `t + 1` のトークンの確率。右詰めの残りは attention mask 0 で埋める。

use serde::{Deserialize, Serialize};

/// 列の長さの組（仮置き。ADR 0075 §4.2）。列はこのうちの入る最も短いものに詰める
pub const SEQ_LENS: [usize; 3] = [32, 64, 128];
/// 1 回に流す列の数の組（仮置き。ADR 0075 §4.2）
pub const BATCH_SIZES: [usize; 3] = [1, 8, 16];
/// 候補 1 つのトークン数の上限（EOS を含む）。`ikari_lm` の llama.cpp の経路と同じ値
pub const MAX_CANDIDATE_TOKENS: usize = 96;
/// 詰めた残りの位置に入れるトークン（mask 0 なので値は結果に効かない）
pub const PAD_TOKEN: i64 = 0;

/// 1 件のトークン列（`prompt` は BOS を含む）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTokens {
    pub prompt: Vec<i64>,
    pub candidates: Vec<Vec<i64>>,
}

/// 全件のトークン列と、終端に使うトークン（zenz は語彙の `</s>`）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub eos: i64,
    pub items: Vec<ItemTokens>,
}

/// 1 つの列（プロンプト + 候補 + EOS）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 何番目の候補か
    pub candidate: usize,
    pub ids: Vec<i64>,
    /// プロンプトのトークン数（BOS を含む）
    pub prompt_len: usize,
}

/// 1 回の推論に詰める列（`rows` の添字）と、使う長さ
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub seq_len: usize,
    pub rows: Vec<usize>,
}

/// 1 件の詰め方
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub rows: Vec<Row>,
    pub chunks: Vec<Chunk>,
    /// 長すぎて採点しない候補（`f32::INFINITY` を返す。`MAX_CANDIDATE_TOKENS` 以上、または最も長い T に入らない）
    pub too_long: Vec<usize>,
}

/// 1 件を、`batch` 行ずつの塊に分ける。長さの組 `seq_lens` は短い順
pub fn plan(tokens: &ItemTokens, eos: i64, batch: usize, seq_lens: &[usize]) -> Plan {
    let mut out = Plan::default();
    let max_len = seq_lens.iter().copied().max().unwrap_or(0);
    for (i, cand) in tokens.candidates.iter().enumerate() {
        let len = tokens.prompt.len() + cand.len() + 1;
        if cand.is_empty()
            || tokens.prompt.is_empty()
            || cand.len() + 1 > MAX_CANDIDATE_TOKENS
            || len > max_len
        {
            out.too_long.push(i);
            continue;
        }
        let mut ids = tokens.prompt.clone();
        ids.extend_from_slice(cand);
        ids.push(eos);
        out.rows.push(Row {
            candidate: i,
            ids,
            prompt_len: tokens.prompt.len(),
        });
    }
    let batch = batch.max(1);
    for &t in seq_lens {
        let fits: Vec<usize> = out
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| bucket(r.ids.len(), seq_lens) == Some(t))
            .map(|(k, _)| k)
            .collect();
        for group in fits.chunks(batch) {
            out.chunks.push(Chunk {
                seq_len: t,
                rows: group.to_vec(),
            });
        }
    }
    out
}

/// 長さ `len` が入る最も短い T
pub fn bucket(len: usize, seq_lens: &[usize]) -> Option<usize> {
    seq_lens.iter().copied().filter(|&t| t >= len).min()
}

/// 1 回の推論の入力（`batch × seq_len`。行優先）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filled {
    pub input_ids: Vec<i64>,
    pub attention_mask: Vec<i64>,
    pub position_ids: Vec<i64>,
}

/// 塊を `batch` 行に詰める（足りない行はすべて詰め物、mask 0）
pub fn fill(plan: &Plan, chunk: &Chunk, batch: usize) -> Filled {
    let t = chunk.seq_len;
    let n = batch * t;
    let mut f = Filled {
        input_ids: vec![PAD_TOKEN; n],
        attention_mask: vec![0; n],
        position_ids: (0..n).map(|k| (k % t) as i64).collect(),
    };
    for (b, &r) in chunk.rows.iter().enumerate().take(batch) {
        let row = &plan.rows[r];
        for (j, &id) in row.ids.iter().enumerate() {
            f.input_ids[b * t + j] = id;
            f.attention_mask[b * t + j] = 1;
        }
    }
    f
}

/// 1 行の logits（`seq_len × vocab`）から、その行の候補の NLL を出す
pub fn row_nll(logits: &[f32], vocab: usize, row: &Row) -> f32 {
    if row.prompt_len == 0 || row.ids.len() <= row.prompt_len {
        return f32::INFINITY;
    }
    let mut nll = 0.0f32;
    // 位置 t の logits が、t + 1 のトークンを予測する。最初の候補のトークンはプロンプトの最後の位置から
    for t in (row.prompt_len - 1)..(row.ids.len() - 1) {
        let Some(at) = logits.get(t * vocab..(t + 1) * vocab) else {
            return f32::INFINITY;
        };
        nll -= log_prob(at, row.ids[t + 1]);
    }
    nll
}

/// `logits` の softmax のうち、`token` の対数確率（`ikari_lm::LlamaScorer` と同じ計算）
pub fn log_prob(logits: &[f32], token: i64) -> f32 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|&l| (l - max).exp()).sum();
    match usize::try_from(token).ok().and_then(|i| logits.get(i)) {
        Some(&l) => l - max - sum.ln(),
        None => f32::NEG_INFINITY,
    }
}

/// IEEE 754 の半精度を単精度にする（出力が FP16 の IR のため）
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exp = u32::from((bits >> 10) & 0x1f);
    let frac = u32::from(bits & 0x3ff);
    let out = match (exp, frac) {
        (0, 0) => sign,
        // 非正規化数
        (0, _) => {
            let mut e: i32 = -14;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            let f = f & 0x3ff;
            sign | (((e + 127) as u32) << 23) | (f << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, _) => sign | 0x7fc0_0000,
        _ => sign | ((exp + 127 - 15) << 23) | (frac << 13),
    };
    f32::from_bits(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> ItemTokens {
        ItemTokens {
            prompt: vec![1, 2, 3],
            candidates: vec![vec![4], vec![5, 6], vec![]],
        }
    }

    #[test]
    fn plan_builds_rows_and_skips_empty() {
        let p = plan(&tokens(), 9, 8, &SEQ_LENS);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0].ids, [1, 2, 3, 4, 9]);
        assert_eq!(p.rows[1].ids, [1, 2, 3, 5, 6, 9]);
        assert_eq!(p.too_long, [2]);
        assert_eq!(
            p.chunks,
            [Chunk {
                seq_len: 32,
                rows: vec![0, 1]
            }]
        );
    }

    #[test]
    fn plan_splits_by_batch_and_length() {
        let mut t = tokens();
        t.candidates = vec![vec![4]; 3];
        t.candidates.push(vec![7; 40]);
        t.candidates.push(vec![7; 200]);
        let p = plan(&t, 9, 2, &SEQ_LENS);
        assert_eq!(p.too_long, [4]);
        let shapes: Vec<(usize, usize)> =
            p.chunks.iter().map(|c| (c.seq_len, c.rows.len())).collect();
        assert_eq!(shapes, [(32, 2), (32, 1), (64, 1)]);
    }

    #[test]
    fn fill_pads_with_mask_zero() {
        let p = plan(&tokens(), 9, 4, &[8]);
        let f = fill(&p, &p.chunks[0], 3);
        assert_eq!(f.input_ids.len(), 24);
        assert_eq!(&f.input_ids[..8], &[1, 2, 3, 4, 9, 0, 0, 0]);
        assert_eq!(&f.attention_mask[..8], &[1, 1, 1, 1, 1, 0, 0, 0]);
        assert_eq!(&f.attention_mask[8..16], &[1, 1, 1, 1, 1, 1, 0, 0]);
        assert!(f.attention_mask[16..].iter().all(|&m| m == 0));
        assert_eq!(&f.position_ids[8..11], &[0, 1, 2]);
    }

    #[test]
    fn row_nll_sums_candidate_and_eos() {
        // 語彙 2。どの位置も P(0) = P(1) = 1/2
        let row = Row {
            candidate: 0,
            ids: vec![0, 1, 0],
            prompt_len: 1,
        };
        let logits = vec![0.0f32; 3 * 2];
        let nll = row_nll(&logits, 2, &row);
        assert!((nll - 2.0 * std::f32::consts::LN_2).abs() < 1e-5, "{nll}");
    }

    #[test]
    fn row_nll_uses_the_previous_position() {
        // 位置 0 は 1 を強く、位置 1 は 0 を強く予測する
        let row = Row {
            candidate: 0,
            ids: vec![0, 1, 0],
            prompt_len: 1,
        };
        let logits = vec![-20.0, 20.0, 20.0, -20.0, 0.0, 0.0];
        assert!(row_nll(&logits, 2, &row) < 1e-3);
    }

    #[test]
    fn log_prob_out_of_vocab_is_negative_infinity() {
        assert_eq!(log_prob(&[0.0, 0.0], 5), f32::NEG_INFINITY);
        assert_eq!(log_prob(&[0.0, 0.0], -1), f32::NEG_INFINITY);
    }

    #[test]
    fn f16_conversion() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!((f16_to_f32(0x0001) - 5.960_464_5e-8).abs() < 1e-12);
        assert!((f16_to_f32(0x3555) - 0.333_251_95).abs() < 1e-6);
    }
}
