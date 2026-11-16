//! 測る入力（ADR 0075 §4.2）。lane/npu-adr が作った `fixtures/homophones-40.jsonl`（README の「契約」の
//! 「fixtures の形式」）を exe に埋める。会社 PC では辞書を読まない。
//!
//! 1 行 1 件: プロンプト、候補、トークン列（llama.cpp の語彙。BOS なし）、llama.cpp の CPU の NLL（Mac で取った参照）。

use serde::{Deserialize, Serialize};

/// 埋めた入力
pub const EMBEDDED: &str = include_str!("../fixtures/homophones-40.jsonl");
/// 入力の元の評価セット（報告に出す名前）
pub const SET_NAME: &str = "homophones-ext";

/// 1 件の入力
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// LM に渡すプロンプト（`ikari_lm::build_prompt_for` の結果）
    pub prompt: String,
    /// LM に渡す候補（並べ替えの対象）
    pub candidates: Vec<String>,
    /// プロンプトのトークン列（BOS なし）
    pub prompt_tokens: Vec<i64>,
    pub candidate_tokens: Vec<Vec<i64>>,
    /// llama.cpp の CPU の NLL（参照。採点しなかった候補は null）
    #[serde(default)]
    pub nll_llama_cpu: Vec<Option<f32>>,
}

/// JSON の行を読む。空の行は飛ばす。候補の無い件、トークン列の数が候補と合わない件は誤り
pub fn parse(text: &str) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let item: Item =
            serde_json::from_str(line).map_err(|e| format!("入力の {} 行目: {e}", n + 1))?;
        if item.candidates.is_empty() {
            return Err(format!("入力の {} 行目（{}）: 候補が無い", n + 1, item.id));
        }
        if item.candidate_tokens.len() != item.candidates.len() {
            return Err(format!(
                "入力の {} 行目（{}）: candidate_tokens の数（{}）が候補の数（{}）と違う",
                n + 1,
                item.id,
                item.candidate_tokens.len(),
                item.candidates.len()
            ));
        }
        items.push(item);
    }
    if items.is_empty() {
        return Err("入力が空".into());
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取り決めの件数（ADR 0075 §4.2）
    const FIXTURE_ITEMS: usize = 40;

    #[test]
    fn embedded_fixture_is_readable() {
        let items = parse(EMBEDDED).expect("埋めた入力が読めない");
        assert_eq!(items.len(), FIXTURE_ITEMS);
        for i in &items {
            assert_eq!(i.nll_llama_cpu.len(), i.candidates.len(), "{}", i.id);
            assert!(!i.prompt_tokens.is_empty(), "{}", i.id);
        }
    }

    #[test]
    fn parse_rejects_mismatched_tokens() {
        let line = r#"{"id":"x","prompt":"p","candidates":["a","b"],"prompt_tokens":[1],"candidate_tokens":[[2]]}"#;
        assert!(parse(line).unwrap_err().contains("candidate_tokens"));
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse("\n\n").is_err());
    }
}
