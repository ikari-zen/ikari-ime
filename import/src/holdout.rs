//! 評価用の 2 割（ADR 0002 §11.4）。
//!
//! 評価用の文は **表記のハッシュで決める**: `FNV64(表記) % 5 == 0`。決定的なので、評価用の文を
//! IME の内部にコピーする必要がない（NFR-19, NFR-26）。評価（`ikari-eval --ingest-holdout`）は、
//! 取り込み元の台帳（`ingest_sources.tsv`）に残ったパスから取り込みファイルを読み直し、
//! 同じ規則で取り出す。

use std::path::Path;

use ikari_core::user::{IngestSources, UserDir, fnv1a64};
use ikari_core::{CONTEXT_WINDOW_CHARS, UserDataError};

use crate::align::{AlignedToken, TokenKind, align};
use crate::io::{ReadError, read_ingest_file};
use crate::pipeline::Env;
use crate::schema::Parsed;

/// 評価用にする割合の分母（5 つに 1 つ）
pub const HOLDOUT_MODULUS: u64 = 5;

/// この文を評価用にするか。表記だけで決まる。
pub fn is_holdout(surface: &str) -> bool {
    fnv1a64(surface.as_bytes()).is_multiple_of(HOLDOUT_MODULUS)
}

/// 評価用の 1 文
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoldoutSentence {
    pub source_id: u32,
    pub doc: u32,
    /// 同じ文書の、この文より前の文を連結したもの（末尾の `CONTEXT_WINDOW_CHARS` 文字まで）。
    /// 原文なので、評価の間だけメモリに持ち、保存しない。
    pub left_text: String,
    pub surface: String,
    pub reading: String,
    pub tokens: Vec<AlignedToken>,
}

/// 取り込みファイルから評価用の文を取り出す。辞書で読みを再現できない文と、未知語を使う文は除く。
pub fn holdout_sentences(parsed: &Parsed, env: &Env<'_>, source_id: u32) -> Vec<HoldoutSentence> {
    let mut out = Vec::new();
    let mut left = String::new();
    let mut current_doc = None;
    for s in &parsed.sentences {
        if current_doc != Some(s.doc) {
            current_doc = Some(s.doc);
            left.clear();
        }
        if is_holdout(&s.surface)
            && let Some(a) = align(env.dict, env.grammar, &s.surface, &s.reading)
            && a.tokens.iter().all(|t| t.kind != TokenKind::Unknown)
        {
            out.push(HoldoutSentence {
                source_id,
                doc: s.doc,
                left_text: tail_chars(&left, CONTEXT_WINDOW_CHARS),
                surface: s.surface.clone(),
                reading: s.reading.clone(),
                tokens: a.tokens,
            });
        }
        left.push_str(&s.surface);
        // 無限に伸びないよう、窓の 2 倍を超えたら前を捨てる
        if left.chars().count() > CONTEXT_WINDOW_CHARS * 2 {
            left = tail_chars(&left, CONTEXT_WINDOW_CHARS);
        }
    }
    out
}

fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// ユーザーデータの台帳を使った読み出しの結果
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HoldoutSet {
    pub sentences: Vec<HoldoutSentence>,
    /// 参照先がなくなった取り込み元（NFR-27: 知らせるだけ）
    pub missing: Vec<String>,
    /// 取り込み後に内容が変わったファイル（ハッシュが台帳と違う）
    pub changed: Vec<String>,
}

/// `<user_dir>` の台帳にある取り込み元すべてから、評価用の文を読み出す。
pub fn load_holdout(dir: &UserDir, env: &Env<'_>) -> Result<HoldoutSet, UserDataError> {
    let ledger = IngestSources::open(&dir.ingest_sources_tsv())?;
    let mut set = HoldoutSet::default();
    for src in ledger.list() {
        match read_ingest_file(Path::new(&src.path)) {
            Ok(file) => {
                if file.fnv64 != src.fnv64 {
                    set.changed.push(src.path.clone());
                }
                set.sentences
                    .extend(holdout_sentences(&file.parsed, env, src.id));
            }
            Err(ReadError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                set.missing.push(src.path.clone());
            }
            Err(e) => {
                return Err(UserDataError::Invalid(format!("{}: {e}", src.path)));
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holdout_is_deterministic_and_about_a_fifth() {
        let hits = (0..2000)
            .filter(|i| is_holdout(&format!("これは{i}番目の文です。")))
            .count();
        assert!((300..=500).contains(&hits), "{hits}");
        assert_eq!(is_holdout("同じ文"), is_holdout("同じ文"));
        // 値そのものを固定する（規則が変わると、過去に取り込んだ評価用が入れ替わってしまうため）
        assert!(is_holdout("文1"));
        assert!(!is_holdout("同じ文"));
    }
}
