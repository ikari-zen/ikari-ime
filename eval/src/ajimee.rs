//! AJIMEE-Bench の読み込み（ADR 0002 §15.1 `--set-format ajimee`）。
//!
//! データ（`JWTD_v2/v1/evaluation_items.json`、CC BY-SA 3.0）はリポジトリに入れない。
//! `eval/scripts/fetch_ajimee.sh` で `data/raw/ajimee/` に取得する。
//!
//! 1件は次の形。入力はカタカナなので、`normalize_reading` でひらがなにして読みにする。`expected_output` の各要素が別解。
//! ```json
//! {"index":"12","context_text":"きっかけで、漫画の仕事","input":"ノイライガクルヨウニ",
//!  "expected_output":["の依頼が来るように"], "original_text":"…", "splitted_input_for_limited_input_length":[]}
//! ```

use ikari_core::user::normalize_reading;
use serde_json::Value;

use crate::{ALT_SEPARATOR, EvalCase, EvalSetError, split_of_id, validate_expected};

/// 左文脈のあるケースの種別
pub const KIND_WITH_CONTEXT: &str = "ajimee-文脈";
/// 左文脈のないケースの種別
pub const KIND_WITHOUT_CONTEXT: &str = "ajimee";
pub const TAG_AJIMEE: &str = "ajimee";
const ID_PREFIX: &str = "ajimee-";
const INDEX_ERROR: &str = "`index` must be a string or a non-negative integer";

fn err(item: usize, message: impl Into<String>) -> EvalSetError {
    // line には配列の中の位置（1始まり）を入れる
    EvalSetError {
        line: item,
        message: message.into(),
    }
}

/// AJIMEE-Bench の JSON（項目の配列）を評価ケースにする。
/// 不正な項目があればエラー（どの項目かを `line` に入れる）。
pub fn load_ajimee(text: &str) -> Result<Vec<EvalCase>, EvalSetError> {
    let root: Value = serde_json::from_str(text).map_err(|e| err(0, format!("json error: {e}")))?;
    let items = root
        .as_array()
        .ok_or_else(|| err(0, "top level must be an array"))?;
    let mut cases = Vec::with_capacity(items.len());
    let mut seen = std::collections::HashSet::new();
    for (i, item) in items.iter().enumerate() {
        let n = i + 1;
        if !item.is_object() {
            return Err(err(n, "item must be an object"));
        }
        let index = match item.get("index") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(Value::Number(v)) => match v.as_f64() {
                Some(v) if v.fract() == 0.0 && v >= 0.0 => format!("{}", v as u64),
                _ => return Err(err(n, INDEX_ERROR)),
            },
            None => n.to_string(),
            _ => return Err(err(n, INDEX_ERROR)),
        };
        let id = format!("{ID_PREFIX}{index}");
        if !seen.insert(id.clone()) {
            return Err(err(n, format!("duplicate index `{index}`")));
        }
        let context = match item.get("context_text") {
            None | Some(Value::Null) => "",
            Some(Value::String(s)) => s.as_str(),
            _ => return Err(err(n, "`context_text` must be a string")),
        };
        let input = item
            .get("input")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(n, "`input` must be a non-empty string"))?;
        let expected = item
            .get("expected_output")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| err(n, "`expected_output` must be a non-empty array"))?;
        let mut alts = Vec::with_capacity(expected.len());
        for e in expected {
            let s = e
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| err(n, "`expected_output` entries must be non-empty strings"))?;
            if s.contains(ALT_SEPARATOR) {
                return Err(err(
                    n,
                    format!(
                        "expected `{s}` contains `{ALT_SEPARATOR}`, which is the alternative separator"
                    ),
                ));
            }
            if !alts.contains(&s) {
                alts.push(s);
            }
        }
        let expected = alts.join(&ALT_SEPARATOR.to_string());
        validate_expected(&expected).map_err(|m| err(n, m))?;
        cases.push(EvalCase {
            kind: if context.is_empty() {
                KIND_WITHOUT_CONTEXT
            } else {
                KIND_WITH_CONTEXT
            }
            .to_owned(),
            tags: vec![TAG_AJIMEE.to_owned()],
            profile: None,
            context: context.to_owned(),
            right: String::new(),
            field: None,
            history: String::new(),
            reading: normalize_reading(input),
            expected,
            note: String::new(),
            split: Some(split_of_id(&id)),
            id,
        });
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実データから切り出したサンプル（形式の確認用。データ本体はリポジトリに入れない）
    const SAMPLE: &str = r#"[
  {
    "index": "12",
    "context_text": "きっかけで、漫画の仕事",
    "input": "ノイライガクルヨウニ",
    "expected_output": ["の依頼が来るように"],
    "original_text": "これがきっかけで、漫画の仕事の依頼が来るようになった。",
    "splitted_input_for_limited_input_length": []
  },
  {
    "index": "1890",
    "context_text": "",
    "input": "カケイヲタスケルタメ、リョウシュノイエニホウコウシ",
    "expected_output": [
      "家計を助けるため、領主の家に奉公し",
      "家計を助ける為、領主の家に奉公し"
    ],
    "original_text": "家計を助けるため、領主の家に奉公している。",
    "splitted_input_for_limited_input_length": ["カケイヲタスケルタメ、", "リョウシュノイエニホウコウシ"]
  }
]"#;

    #[test]
    fn loads_sample_items() {
        let cases = load_ajimee(SAMPLE).unwrap();
        assert_eq!(cases.len(), 2);
        let a = &cases[0];
        assert_eq!(a.id, "ajimee-12");
        assert_eq!(a.kind, KIND_WITH_CONTEXT);
        assert_eq!(a.context, "きっかけで、漫画の仕事");
        assert_eq!(a.reading, "のいらいがくるように");
        assert_eq!(a.expected, "の依頼が来るように");
        assert_eq!(a.tags, [TAG_AJIMEE]);
        assert_eq!(a.split, Some(split_of_id("ajimee-12")));
        let b = &cases[1];
        assert_eq!(b.kind, KIND_WITHOUT_CONTEXT);
        assert_eq!(b.context, "");
        assert_eq!(
            b.reading,
            "かけいをたすけるため、りょうしゅのいえにほうこうし"
        );
        // 別解は `|` でつなぐ
        assert_eq!(
            b.alternatives(),
            [
                "家計を助けるため、領主の家に奉公し",
                "家計を助ける為、領主の家に奉公し"
            ]
        );
    }

    #[test]
    fn reading_conversion_keeps_long_vowel_marks_and_symbols() {
        assert_eq!(
            normalize_reading("ラーメン、ＡＢＣ１２３（ヴ）"),
            "らーめん、ＡＢＣ１２３（ゔ）"
        );
        assert_eq!(normalize_reading(""), "");
        // ヷヺ などの範囲外はそのまま
        assert_eq!(normalize_reading("ヷ"), "ヷ");
    }

    #[test]
    fn missing_optional_fields_are_tolerated() {
        let cases = load_ajimee(r#"[{"input":"ア","expected_output":["亜"]}]"#).unwrap();
        assert_eq!(cases[0].id, "ajimee-1");
        assert_eq!(cases[0].context, "");
        let cases = load_ajimee(
            r#"[{"index":7,"context_text":null,"input":"ア","expected_output":["亜"]}]"#,
        )
        .unwrap();
        assert_eq!(cases[0].id, "ajimee-7");
    }

    #[test]
    fn duplicate_expected_outputs_are_collapsed() {
        let cases = load_ajimee(r#"[{"input":"ア","expected_output":["亜","亜","阿"]}]"#).unwrap();
        assert_eq!(cases[0].expected, "亜|阿");
    }

    #[test]
    fn rejects_bad_data() {
        let bad = [
            ("", 0),
            ("{}", 0),
            ("[1]", 1),
            (r#"[{"input":"ア"}]"#, 1),
            (r#"[{"input":"","expected_output":["a"]}]"#, 1),
            (r#"[{"input":"ア","expected_output":[]}]"#, 1),
            (r#"[{"input":"ア","expected_output":[""]}]"#, 1),
            (r#"[{"input":"ア","expected_output":[1]}]"#, 1),
            (r#"[{"input":"ア","expected_output":["a|b"]}]"#, 1),
            (
                r#"[{"input":"ア","context_text":3,"expected_output":["a"]}]"#,
                1,
            ),
            (r#"[{"index":"","input":"ア","expected_output":["a"]}]"#, 1),
            (r#"[{"index":-1,"input":"ア","expected_output":["a"]}]"#, 1),
            (
                r#"[{"index":"1","input":"ア","expected_output":["a"]},{"index":"1","input":"イ","expected_output":["b"]}]"#,
                2,
            ),
        ];
        for (text, line) in bad {
            let e = load_ajimee(text).expect_err(text);
            assert_eq!(e.line, line, "{text}: {e}");
        }
    }

    #[test]
    fn deeply_nested_input_is_an_error_not_a_panic() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert_eq!(load_ajimee(&deep).expect_err("too deep").line, 0);
    }

    /// `eval/scripts/fetch_ajimee.sh` で取得した実データがあれば、読み込めることを確かめる。
    /// なければ何もしない（データはリポジトリに入れない）。
    #[test]
    fn loads_real_data_when_fetched() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../data/raw/ajimee/evaluation_items.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("skip: {} is not fetched", path.display());
            return;
        };
        let cases = load_ajimee(&text).expect("real AJIMEE data must load");
        assert_eq!(cases.len(), 200);
        assert_eq!(
            cases.iter().filter(|c| c.kind == KIND_WITH_CONTEXT).count(),
            100,
            "100 items have a left context"
        );
        assert!(
            cases
                .iter()
                .all(|c| !c.reading.is_empty() && !c.expected.is_empty())
        );
        // 読みはひらがな化されている（カタカナが残らない）
        assert!(cases.iter().all(|c| {
            c.reading
                .chars()
                .all(|ch| !('\u{30A1}'..='\u{30F6}').contains(&ch))
        }));
        // 別解があるものも読める
        assert!(cases.iter().any(|c| c.alternatives().len() > 1));
    }
}
