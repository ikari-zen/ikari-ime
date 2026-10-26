//! 同梱のプロンプト（NFR-21, NFR-22, NFR-28）。
//!
//! プロンプトの版は本文とヘッダの両方に入っていて、LLM に出力ファイルのヘッダへ書かせる。
//! 版を上げるときは新しいファイル（`ingest-v2.md`）を足し、`schema::KNOWN_PROMPT_VERSIONS` に加える。

/// プロンプトの版（出力ファイルの `prompt_version` に書かせる値）
pub const PROMPT_VERSION: &str = "ingest-prompt-v1";

/// プロンプト本文（Markdown）
pub const INGEST_PROMPT: &str = include_str!("../prompts/ingest-v1.md");

/// 業務文書の注意（NFR-28）。プロンプトの冒頭と、CLI の画面の両方で出す。
pub const CLOUD_LLM_WARNING: &str = "注意: 業務文書、顧客情報、機密情報、個人情報を含む文章を、クラウドの LLM に入力しないでください。\
入力してよいのは、あなた自身が書いた、外部に出してよい文章だけです。";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{KNOWN_PROMPT_VERSIONS, parse_jsonl};

    #[test]
    fn prompt_carries_its_version_and_the_business_document_warning() {
        assert!(INGEST_PROMPT.contains(PROMPT_VERSION));
        assert!(KNOWN_PROMPT_VERSIONS.contains(&PROMPT_VERSION));
        assert!(INGEST_PROMPT.contains("業務文書"));
        assert!(INGEST_PROMPT.contains("クラウドの LLM"));
        // 注意は本文の冒頭（取り込む文章より前、最初の見出しの直後）にある
        let warn_at = INGEST_PROMPT.find("業務文書").unwrap();
        let role_at = INGEST_PROMPT.find("あなたの役割").unwrap();
        assert!(warn_at < role_at);
        assert!(CLOUD_LLM_WARNING.contains("業務文書"));
    }

    #[test]
    fn prompt_asks_to_exclude_code_inline_code_and_urls() {
        for needle in ["コードブロック", "インラインコード", "URL"] {
            assert!(INGEST_PROMPT.contains(needle), "{needle}");
        }
    }

    #[test]
    fn the_example_in_the_prompt_is_a_valid_file() {
        // プロンプト内の最後のコードブロック（例）をそのまま読み込める
        let blocks: Vec<&str> = INGEST_PROMPT.split("```").collect();
        let example = blocks
            .iter()
            .rev()
            .find(|b| b.contains("\"type\":\"sentence\""))
            .expect("example block");
        let parsed = parse_jsonl(example.as_bytes()).unwrap();
        assert_eq!(parsed.prompt_version, PROMPT_VERSION);
        assert_eq!(parsed.drops.total(), 0);
        assert_eq!(parsed.sentences.len(), 2);
    }
}
