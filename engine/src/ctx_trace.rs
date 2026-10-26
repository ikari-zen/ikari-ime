//! 文脈の番号（`context_rev`）が進んだ理由のログ用の記録（ADR 0021 L0、L2。調査用）。
//!
//! 殻は周辺テキストを取り直して前回と違うときだけ番号を進める。エンジンは番号を作らず受け取るだけなので、
//! 同じセッションの前の要求と見比べて、left / right / available のどれが変わったかを出す。
//! 出すのは変わったかどうかのフラグと文字数の差だけ。文字列そのもの、ハッシュは出さない（NFR-19）。
//! 直前の周辺テキストはメモリに持つが、ログにもディスクにも書かない。secure では何も持たない（NFR-18）。

use std::collections::HashMap;
use std::time::Instant;

use ikari_proto::v1 as pb;

/// セッションごとに最後に見た周辺テキストと番号
struct Seen {
    rev: u64,
    left: String,
    right: String,
    available: bool,
}

/// 最後の確定の時刻と文字数（番号のずれが確定の反映の遅れかを見分ける。#249）
struct LastCommit {
    at: Instant,
    chars: usize,
}

#[derive(Default)]
pub struct ContextTrace {
    seen: HashMap<u64, Seen>,
    commits: HashMap<u64, LastCommit>,
}

fn char_delta(new: &str, old: &str) -> i64 {
    let count = |s: &str| i64::try_from(s.chars().count()).unwrap_or(i64::MAX);
    count(new) - count(old)
}

/// LM が見る left の末尾（`zenz.context_chars` の既定）。ここが同じなら、番号を進めなくても LM の入力は変わらない
const LM_TAIL_CHARS: usize = 40;

fn tail(s: &str) -> &str {
    let skip = s.chars().count().saturating_sub(LM_TAIL_CHARS);
    s.char_indices().nth(skip).map_or("", |(i, _)| &s[i..])
}

/// 番号が進んだときのログの注記（固定の語と数だけ）。
/// `req` は番号を進めた要求の種類（prefetch は 1 文字目、convert は Space）、`reading` はその読みの文字数、
/// `lext` は left が前回の続きに伸びただけか、`tail` は left の末尾 40 字が違うか
fn change_note(
    prev: &Seen,
    left: &str,
    right: &str,
    available: bool,
    rev: u64,
    req: &str,
    reading: usize,
) -> String {
    format!(
        "rev={rev} prev_rev={} left={} right={} avail={} dleft={:+} dright={:+} req={req} reading={reading} lext={} tail={}",
        prev.rev,
        u8::from(prev.left != left),
        u8::from(prev.right != right),
        u8::from(prev.available != available),
        char_delta(left, &prev.left),
        char_delta(right, &prev.right),
        u8::from(prev.left != left && left.starts_with(prev.left.as_str())),
        u8::from(tail(&prev.left) != tail(left)),
    )
}

impl ContextTrace {
    /// 確定の時刻と文字数だけを覚える（文字列は持たない）。secure では呼ばない
    pub fn note_commit(&mut self, session: u64, chars: usize) {
        self.commits.insert(
            session,
            LastCommit {
                at: Instant::now(),
                chars,
            },
        );
    }

    /// 要求の周辺テキストを記録する。番号が前回と違うときだけ、`ctx_change` の行の注記を返す
    pub fn observe(
        &mut self,
        session: u64,
        rev: u64,
        surrounding: Option<&pb::SurroundingText>,
        req: &str,
        reading: usize,
    ) -> Option<String> {
        let (left, right, available) = match surrounding {
            Some(s) => (s.left.as_str(), s.right.as_str(), s.available),
            None => ("", "", false),
        };
        let note = match self.seen.get(&session) {
            Some(prev) if prev.rev != rev => {
                let mut note = change_note(prev, left, right, available, rev, req, reading);
                // 確定のあとなら、その経過 ms と文字数を足す
                if let Some(c) = self.commits.get(&session) {
                    note.push_str(&format!(
                        " since_commit_ms={} commit_chars={}",
                        c.at.elapsed().as_millis(),
                        c.chars
                    ));
                }
                Some(note)
            }
            _ => None,
        };
        if note.is_some() || !self.seen.contains_key(&session) {
            self.seen.insert(
                session,
                Seen {
                    rev,
                    left: left.to_owned(),
                    right: right.to_owned(),
                    available,
                },
            );
        }
        note
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    pub fn remove_session(&mut self, session: u64) {
        self.seen.remove(&session);
        self.commits.remove(&session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(left: &str, right: &str, available: bool) -> pb::SurroundingText {
        pb::SurroundingText {
            left: left.into(),
            right: right.into(),
            available,
        }
    }

    #[test]
    fn first_sight_and_same_rev_are_silent() {
        let mut t = ContextTrace::default();
        assert_eq!(
            t.observe(1, 1, Some(&st("あ", "", true)), "prefetch", 0),
            None
        );
        assert_eq!(
            t.observe(1, 1, Some(&st("あ", "", true)), "prefetch", 0),
            None
        );
    }

    #[test]
    fn reports_flags_and_char_deltas_only() {
        let mut t = ContextTrace::default();
        t.observe(1, 1, Some(&st("あ", "い", true)), "prefetch", 0);
        let note = t
            .observe(1, 2, Some(&st("あうえ", "い", true)), "prefetch", 0)
            .unwrap();
        assert_eq!(
            note,
            "rev=2 prev_rev=1 left=1 right=0 avail=0 dleft=+2 dright=+0 req=prefetch reading=0 lext=1 tail=1"
        );
        let note = t.observe(1, 3, None, "prefetch", 0).unwrap();
        assert_eq!(
            note,
            "rev=3 prev_rev=2 left=1 right=1 avail=1 dleft=-3 dright=-1 req=prefetch reading=0 lext=0 tail=1"
        );
    }

    #[test]
    fn sessions_are_separate_and_removable() {
        let mut t = ContextTrace::default();
        t.observe(1, 1, None, "prefetch", 0);
        assert_eq!(t.observe(2, 5, None, "prefetch", 0), None);
        t.remove_session(1);
        assert_eq!(t.observe(1, 9, None, "prefetch", 0), None);
    }
}
