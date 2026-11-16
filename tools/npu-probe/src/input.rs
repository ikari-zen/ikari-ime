//! 測る入力（ADR 0075 §4.2）。`eval/sets/homophones-ext.tsv` の 40 件と、各件で今の変換が LM に渡す
//! プロンプトと候補。会社 PC では辞書を読まないので、Mac で作って（`scripts/make-inputs.sh`）exe に埋める。
//!
//! 1 行 1 件の JSON（`inputs/homophones-40.jsonl`）。`prompt_ids` と `candidate_ids` は任意で、モデルの変換
//! （lane/npu-adr の契約）がトークン列を別に作るときに入れる。あれば llama.cpp のトークン列と比べる。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 埋めた入力
pub const EMBEDDED: &str = include_str!("../inputs/homophones-40.jsonl");
/// 入力の元の評価セット（報告に出す名前）
pub const SET_NAME: &str = "homophones-ext";

/// 選んだ 40 件（文脈の文字数で並べ、等間隔に取った。`scripts/make-inputs.sh`）。順番は文脈の短い順
pub const SELECTED_IDS: [&str; 40] = [
    "ext-gram-131",
    "ext-gram-155",
    "ext-idou-1-07",
    "ext-kouka-1-01",
    "ext-shinkou-3-01",
    "ext-gram-227",
    "ext-idou-1-01",
    "ext-kaihou-1-02",
    "ext-katei-2-05",
    "ext-kousei-4-03",
    "ext-osameru-1-03",
    "ext-seisaku-3-01",
    "ext-souzou-1-01",
    "ext-youi-1-08",
    "ext-gram-250",
    "ext-katei-3-09",
    "ext-neg-62",
    "ext-gram-102",
    "ext-gram-248",
    "ext-kansei-1-10",
    "ext-neg-126",
    "ext-shoukai-2-04",
    "ext-hakaru-1-05",
    "ext-kikou-2-10",
    "ext-neg-94",
    "ext-gram-127",
    "ext-kanshin-1-06",
    "ext-neg-69",
    "ext-gram-55",
    "ext-neg-110",
    "ext-seikaku-1-08",
    "ext-katei-3-05",
    "ext-neg-82",
    "ext-neg-103",
    "ext-gram-99",
    "ext-gram-36",
    "ext-neg-71",
    "ext-kikai-2-10",
    "ext-ijou-2-10",
    "ext-hinshi-1-09",
];

/// 1 件の入力
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub context: String,
    pub reading: String,
    pub expected: String,
    /// LM に渡すプロンプト（`ikari_lm::build_prompt` の結果）
    pub prompt: String,
    /// LM に渡す候補（並べ替えの対象）
    pub candidates: Vec<String>,
    /// 別に作ったトークン列（BOS を含む）。無ければ llama.cpp の語彙で作る
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ids: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ids: Option<Vec<Vec<i64>>>,
    /// 終端のトークン（`prompt_ids` を入れるときに一緒に入れる）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eos_id: Option<i64>,
}

/// JSON の行を読む。空の行は飛ばす。候補の無い件、`candidate_ids` の数が候補と合わない件は誤り
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
        if let Some(ids) = &item.candidate_ids
            && ids.len() != item.candidates.len()
        {
            return Err(format!(
                "入力の {} 行目（{}）: candidate_ids の数（{}）が候補の数（{}）と違う",
                n + 1,
                item.id,
                ids.len(),
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

/// 記録の 1 行（`npu-probe-record-host` が書く）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub prompt: String,
    pub candidates: Vec<String>,
}

/// 評価セットの 1 行（使う列だけ）
struct SetRow {
    context: String,
    reading: String,
    expected: String,
}

/// 評価セット（TSV。`#` の行と見出しの行は飛ばす）を id で引ける形にする
fn read_set(tsv: &str) -> BTreeMap<String, SetRow> {
    const ID: usize = 0;
    const CONTEXT: usize = 4;
    const READING: usize = 5;
    const EXPECTED: usize = 6;
    let mut rows = BTreeMap::new();
    for line in tsv.lines() {
        if line.starts_with('#') || line.starts_with("id\t") {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() <= EXPECTED {
            continue;
        }
        rows.insert(
            cols[ID].to_owned(),
            SetRow {
                context: cols[CONTEXT].to_owned(),
                reading: cols[READING].to_owned(),
                expected: cols[EXPECTED].to_owned(),
            },
        );
    }
    rows
}

/// 記録と評価セットから、入力の JSON の行を作る。各 id で、候補のいちばん多い頼み（読み全体の変換）を使う
/// （暖機の頼みや、文節の頼みを避ける）。候補の数が同じなら先のもの
pub fn make_inputs(records: &str, set_tsv: &str) -> Result<String, String> {
    let set = read_set(set_tsv);
    let mut best: BTreeMap<String, Record> = BTreeMap::new();
    for (n, line) in records.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let r: Record =
            serde_json::from_str(line).map_err(|e| format!("記録の {} 行目: {e}", n + 1))?;
        let better = best
            .get(&r.id)
            .is_none_or(|b| r.candidates.len() > b.candidates.len());
        if better {
            best.insert(r.id.clone(), r);
        }
    }
    let mut out = String::new();
    for id in SELECTED_IDS {
        let row = set
            .get(id)
            .ok_or_else(|| format!("評価セットに {id} が無い"))?;
        let r = best
            .get(id)
            .ok_or_else(|| format!("{id} の記録が無い（LM に頼みが届いていない）"))?;
        let item = Item {
            id: id.to_owned(),
            context: row.context.clone(),
            reading: row.reading.clone(),
            expected: row.expected.clone(),
            prompt: r.prompt.clone(),
            candidates: r.candidates.clone(),
            prompt_ids: None,
            candidate_ids: None,
            eos_id: None,
        };
        out.push_str(&serde_json::to_string(&item).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, cands: &[&str]) -> String {
        serde_json::to_string(&Record {
            id: id.into(),
            prompt: format!("p-{id}"),
            candidates: cands.iter().map(|s| s.to_string()).collect(),
        })
        .unwrap()
    }

    fn full_set() -> String {
        let mut tsv =
            String::from("# comment\nid\tkind\ttags\tprofile\tcontext\treading\texpected\n");
        for id in SELECTED_IDS {
            tsv.push_str(&format!("{id}\tk\tt\t\tctx-{id}\tよみ\t読み\n"));
        }
        tsv
    }

    #[test]
    fn embedded_inputs_cover_the_selected_ids() {
        let items = parse(EMBEDDED).expect("埋めた入力が読めない");
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, SELECTED_IDS);
    }

    #[test]
    fn make_inputs_takes_the_request_with_most_candidates() {
        let mut records = String::new();
        for id in SELECTED_IDS {
            records.push_str(&line(id, &["a"]));
            records.push('\n');
            records.push_str(&line(id, &["b", "c"]));
            records.push('\n');
            records.push_str(&line(id, &["d", "e"]));
            records.push('\n');
        }
        let out = make_inputs(&records, &full_set()).unwrap();
        let items = parse(&out).unwrap();
        assert_eq!(items.len(), SELECTED_IDS.len());
        assert_eq!(items[0].candidates, ["b", "c"]);
        assert_eq!(items[0].context, format!("ctx-{}", SELECTED_IDS[0]));
        assert_eq!(items[0].prompt, format!("p-{}", SELECTED_IDS[0]));
    }

    #[test]
    fn make_inputs_fails_on_missing_record() {
        let err = make_inputs("", &full_set()).unwrap_err();
        assert!(err.contains(SELECTED_IDS[0]), "{err}");
    }

    #[test]
    fn parse_rejects_mismatched_candidate_ids() {
        let item = Item {
            id: "x".into(),
            context: String::new(),
            reading: String::new(),
            expected: String::new(),
            prompt: "p".into(),
            candidates: vec!["a".into(), "b".into()],
            prompt_ids: Some(vec![1]),
            candidate_ids: Some(vec![vec![2]]),
            eos_id: Some(3),
        };
        let text = serde_json::to_string(&item).unwrap();
        assert!(parse(&text).unwrap_err().contains("candidate_ids"));
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse("\n\n").is_err());
    }
}
