//! 形を固定した採点（ADR 0075 §3「C」。README の「契約」の「1 回の推論の詰め方と NLL」）。KV キャッシュを使わず、
//! 「プロンプト + 候補」の列を B 行 × L 個に詰めて 1 回流し、各位置の確率から NLL を出す。OpenVINO に依らない部分
//! （詰め方と計算）だけを置く。
//!
//! NLL の定義は `ikari_lm::LlamaScorer::nll` と同じ: `-Σ log P(候補のトークン + EOS | プロンプト)`（nat）。
//! 位置 `t` の logits は、位置 `t + 1` のトークンの確率。右の残りは `[PAD]` で埋める（因果なので前の位置に効かない）。
//! 余りの行（候補が B に満たない回）は全部 `[PAD]` にして、出力を読まない。

use serde::{Deserialize, Serialize};

/// 契約の IR の B（B = 1 と B = 32 の 2 つ）
pub const BATCH_SIZES: [usize; 2] = [1, 32];
/// 候補 1 つのトークン数の上限（未満）。`ikari_lm` の llama.cpp の経路と同じ値
pub const MAX_CANDIDATE_TOKENS: usize = 96;
/// 詰め物のトークン（契約: `[PAD]` = 1）
pub const PAD_TOKEN: i64 = 1;
/// 終わりのトークン（契約: `</s>` = 3。GGUF の `eos_token_id` ではない）
pub const END_TOKEN: i64 = 3;

/// 1 件のトークン列（BOS なし）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTokens {
    pub prompt: Vec<i64>,
    pub candidates: Vec<Vec<i64>>,
}

/// 全件のトークン列と、終端に使うトークン
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub eos: i64,
    pub items: Vec<ItemTokens>,
}

/// 1 つの行（プロンプト + 候補。EOS は入れず、最後の位置から確率を読む）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 何番目の候補か
    pub candidate: usize,
    pub ids: Vec<i64>,
    /// プロンプトのトークン数
    pub prompt_len: usize,
    pub eos: i64,
}

/// 1 回の推論に詰める行（`rows` の添字。元の順）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub rows: Vec<usize>,
}

/// 1 件の詰め方
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub rows: Vec<Row>,
    pub chunks: Vec<Chunk>,
    /// 長すぎて（または空で）採点しない候補（NLL は無し = `+inf`）
    pub too_long: Vec<usize>,
}

/// 1 件を、元の順に `batch` 行ずつの回に分ける。`P + n > seq_len`、`n = 0`、`n ≥ 96` の候補は詰めない
pub fn plan(tokens: &ItemTokens, eos: i64, batch: usize, seq_len: usize) -> Plan {
    let mut out = Plan::default();
    for (i, cand) in tokens.candidates.iter().enumerate() {
        let len = tokens.prompt.len() + cand.len();
        if cand.is_empty()
            || tokens.prompt.is_empty()
            || cand.len() >= MAX_CANDIDATE_TOKENS
            || len > seq_len
        {
            out.too_long.push(i);
            continue;
        }
        let mut ids = tokens.prompt.clone();
        ids.extend_from_slice(cand);
        out.rows.push(Row {
            candidate: i,
            ids,
            prompt_len: tokens.prompt.len(),
            eos,
        });
    }
    let all: Vec<usize> = (0..out.rows.len()).collect();
    for group in all.chunks(batch.max(1)) {
        out.chunks.push(Chunk {
            rows: group.to_vec(),
        });
    }
    out
}

/// 1 回の `input_ids`（`batch × seq_len`。行優先）。足りない行はすべて `[PAD]`
pub fn fill(plan: &Plan, chunk: &Chunk, batch: usize, seq_len: usize) -> Vec<i64> {
    let mut ids = vec![PAD_TOKEN; batch * seq_len];
    for (b, &r) in chunk.rows.iter().enumerate().take(batch) {
        let row = &plan.rows[r];
        ids[b * seq_len..b * seq_len + row.ids.len()].copy_from_slice(&row.ids);
    }
    ids
}

/// 1 行の logits（`seq_len × vocab`）から、その行の候補の NLL を出す（候補の各トークンと EOS）
pub fn row_nll(logits: &[f32], vocab: usize, row: &Row) -> f32 {
    if row.prompt_len == 0 || row.ids.len() <= row.prompt_len {
        return f32::INFINITY;
    }
    let at = |t: usize| logits.get(t * vocab..(t + 1) * vocab);
    let mut nll = 0.0f32;
    // 位置 t の logits が t + 1 のトークンを予測する。最初の候補のトークンはプロンプトの最後の位置から
    for t in (row.prompt_len - 1)..(row.ids.len() - 1) {
        let Some(l) = at(t) else {
            return f32::INFINITY;
        };
        nll -= log_prob(l, row.ids[t + 1]);
    }
    let Some(l) = at(row.ids.len() - 1) else {
        return f32::INFINITY;
    };
    nll - log_prob(l, row.eos)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> ItemTokens {
        ItemTokens {
            prompt: vec![10, 11, 12],
            candidates: vec![vec![4], vec![5, 6], vec![]],
        }
    }

    #[test]
    fn plan_builds_rows_and_skips_empty() {
        let p = plan(&tokens(), END_TOKEN, 32, 128);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0].ids, [10, 11, 12, 4]);
        assert_eq!(p.rows[1].ids, [10, 11, 12, 5, 6]);
        assert_eq!(p.too_long, [2]);
        assert_eq!(p.chunks, [Chunk { rows: vec![0, 1] }]);
    }

    #[test]
    fn plan_splits_by_batch_and_skips_too_long() {
        let mut t = tokens();
        t.candidates = vec![vec![4]; 3];
        // P + n = L は入る、L + 1 は入らない
        t.candidates.push(vec![7; 5]);
        t.candidates.push(vec![7; 6]);
        let p = plan(&t, END_TOKEN, 2, 8);
        assert_eq!(p.too_long, [4]);
        let sizes: Vec<usize> = p.chunks.iter().map(|c| c.rows.len()).collect();
        assert_eq!(sizes, [2, 2]);
        assert_eq!(p.rows[3].candidate, 3);
    }

    #[test]
    fn fill_pads_rows_and_remainder() {
        let p = plan(&tokens(), END_TOKEN, 4, 6);
        let ids = fill(&p, &p.chunks[0], 3, 6);
        assert_eq!(ids.len(), 18);
        assert_eq!(&ids[..6], &[10, 11, 12, 4, 1, 1]);
        assert_eq!(&ids[6..12], &[10, 11, 12, 5, 6, 1]);
        assert!(ids[12..].iter().all(|&x| x == PAD_TOKEN));
    }

    #[test]
    fn row_nll_sums_candidate_and_eos() {
        // 語彙 4。どの位置も一様（P = 1/4）。候補 1 トークン + EOS = 2 項
        let row = Row {
            candidate: 0,
            ids: vec![0, 1],
            prompt_len: 1,
            eos: 3,
        };
        let logits = vec![0.0f32; 4 * 4];
        let nll = row_nll(&logits, 4, &row);
        assert!((nll - 2.0 * 4f32.ln()).abs() < 1e-5, "{nll}");
    }

    #[test]
    fn row_nll_uses_the_previous_position() {
        // 位置 0 は 1 を、位置 1 は EOS（3）を強く予測する
        let row = Row {
            candidate: 0,
            ids: vec![0, 1],
            prompt_len: 1,
            eos: 3,
        };
        let mut logits = vec![-20.0f32; 2 * 4];
        logits[1] = 20.0;
        logits[4 + 3] = 20.0;
        assert!(row_nll(&logits, 4, &row) < 1e-3);
        // logits が足りなければ評価できない
        assert_eq!(row_nll(&logits[..4], 4, &row), f32::INFINITY);
    }

    #[test]
    fn log_prob_out_of_vocab_is_negative_infinity() {
        assert_eq!(log_prob(&[0.0, 0.0], 5), f32::NEG_INFINITY);
        assert_eq!(log_prob(&[0.0, 0.0], -1), f32::NEG_INFINITY);
    }
}
