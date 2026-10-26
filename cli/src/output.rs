//! 候補の整形（ADR 0001 §5）。`&[Candidate]` から text / tsv の文字列を作る純粋関数。

use ikari_core::{Candidate, CandidateOrigin, Segment};

/// 品詞IDから品詞名を引く関数（grammar.pos_name を包んだもの）。
pub type PosNameFn<'a> = &'a dyn Fn(u16) -> Option<String>;

const UNKNOWN_POS_NAME: &str = "?";
const SEGMENT_JOINER: &str = " + ";
const READING_LABEL: &str = "読み";
const CONTEXT_LABEL: &str = "文脈";

/// tsv の origin 列。Rewriter は `rewriter:<name>:<format>`（学習が使う書式IDまで見えるように）。
fn origin_name(origin: CandidateOrigin) -> String {
    match origin {
        CandidateOrigin::Dictionary => "dictionary".to_string(),
        CandidateOrigin::Hiragana => "hiragana".to_string(),
        CandidateOrigin::Katakana => "katakana".to_string(),
        CandidateOrigin::Rewriter { name, format } => format!("rewriter:{name}:{format}"),
        // non_exhaustive: 今後増える origin はここに足す
        _ => "other".to_string(),
    }
}

fn pos_name_or_unknown(pos_name: PosNameFn<'_>, pos_id: u16) -> String {
    pos_name(pos_id).unwrap_or_else(|| UNKNOWN_POS_NAME.to_string())
}

/// TSV のセルに入れられない文字（タブ、改行）を空白にする。
fn sanitize_cell(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn text_segment(seg: &Segment, pos_name: PosNameFn<'_>) -> String {
    format!(
        "[{}/{} {}]",
        seg.surface,
        seg.reading,
        pos_name_or_unknown(pos_name, seg.pos_id)
    )
}

fn tsv_segment(seg: &Segment, pos_name: PosNameFn<'_>) -> String {
    format!(
        "{}/{}/{}",
        seg.surface,
        seg.reading,
        pos_name_or_unknown(pos_name, seg.pos_id)
    )
}

/// text 形式。番号はページ内の 1 始まり（番号ラベル）。`page_size` ごとに `-- page N --` を入れる。
pub fn render_text(
    reading: &str,
    context: &str,
    candidates: &[Candidate],
    page_size: usize,
    show_segments: bool,
    pos_name: PosNameFn<'_>,
) -> String {
    let page_size = page_size.max(1);
    let mut out = String::new();
    out.push_str(&format!("{READING_LABEL}: {reading}\n"));
    if !context.is_empty() {
        out.push_str(&format!("{CONTEXT_LABEL}: {context}\n"));
    }
    for (i, cand) in candidates.iter().enumerate() {
        if i > 0 && i % page_size == 0 {
            out.push_str(&format!("-- page {} --\n", i / page_size + 1));
        }
        let label = i % page_size + 1;
        out.push_str(&format!("{label:>2}  {}", cand.surface));
        if show_segments && !cand.segments.is_empty() {
            let segs: Vec<String> = cand
                .segments
                .iter()
                .map(|s| text_segment(s, pos_name))
                .collect();
            out.push_str("  ");
            out.push_str(&segs.join(SEGMENT_JOINER));
        }
        out.push('\n');
    }
    out
}

/// tsv 形式（ヘッダなし）。rank は全体での 1 始まりの順位。
/// 列: rank, surface, score, base_cost, origin, segments
pub fn render_tsv(candidates: &[Candidate], pos_name: PosNameFn<'_>) -> String {
    let mut out = String::new();
    for (i, cand) in candidates.iter().enumerate() {
        let segs: Vec<String> = cand
            .segments
            .iter()
            .map(|s| sanitize_cell(&tsv_segment(s, pos_name)))
            .collect();
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            i + 1,
            sanitize_cell(&cand.surface),
            cand.score,
            cand.base_cost,
            origin_name(cand.origin),
            segs.join(SEGMENT_JOINER)
        ));
    }
    out
}

/// 未確定のローマ字がある場合の stderr 向けメッセージ。なければ None。
pub fn pending_notice(pending: &str) -> Option<String> {
    if pending.is_empty() {
        None
    } else {
        Some(format!("未確定: {pending}（変換対象外）"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(surface: &str, reading: &str, pos_id: u16) -> Segment {
        Segment {
            reading: reading.to_string(),
            surface: surface.to_string(),
            word: None,
            left_id: 0,
            right_id: 0,
            pos_id,
        }
    }

    fn cand(surface: &str, cost: i32, origin: CandidateOrigin, segs: Vec<Segment>) -> Candidate {
        Candidate {
            surface: surface.to_string(),
            segments: segs,
            base_cost: cost,
            score: cost,
            origin,
        }
    }

    fn pos(id: u16) -> Option<String> {
        match id {
            1 => Some("名詞,普通名詞,一般,*,*,*".to_string()),
            2 => Some("動詞,非自立可能,*,*,サ行変格,終止形-一般".to_string()),
            _ => None,
        }
    }

    fn sample() -> Vec<Candidate> {
        vec![
            cand(
                "品詞",
                7392,
                CandidateOrigin::Dictionary,
                vec![seg("品詞", "ひんし", 1)],
            ),
            cand(
                "瀕死",
                7403,
                CandidateOrigin::Dictionary,
                vec![seg("瀕死", "ひんし", 1)],
            ),
            cand("ひんし", 9000, CandidateOrigin::Hiragana, vec![]),
        ]
    }

    #[test]
    fn text_basic() {
        let out = render_text("ひんし", "文脈です", &sample(), 9, false, &pos);
        assert_eq!(
            out,
            "読み: ひんし\n文脈: 文脈です\n 1  品詞\n 2  瀕死\n 3  ひんし\n"
        );
    }

    #[test]
    fn text_omits_empty_context() {
        let out = render_text("ひんし", "", &sample()[..1], 9, false, &pos);
        assert_eq!(out, "読み: ひんし\n 1  品詞\n");
    }

    #[test]
    fn text_with_segments() {
        let cands = vec![
            cand(
                "変換する",
                100,
                CandidateOrigin::Dictionary,
                vec![seg("変換", "へんかん", 1), seg("する", "する", 2)],
            ),
            cand(
                "x",
                200,
                CandidateOrigin::Dictionary,
                vec![seg("x", "x", 99)],
            ),
        ];
        let out = render_text("へんかんする", "", &cands, 9, true, &pos);
        let expected = [
            "読み: へんかんする",
            " 1  変換する  [変換/へんかん 名詞,普通名詞,一般,*,*,*] + [する/する 動詞,非自立可能,*,*,サ行変格,終止形-一般]",
            " 2  x  [x/x ?]",
            "",
        ]
        .join("\n");
        assert_eq!(out, expected);
    }

    #[test]
    fn text_page_boundaries_and_labels() {
        let cands: Vec<Candidate> = (0..5)
            .map(|i| cand(&format!("c{i}"), i, CandidateOrigin::Dictionary, vec![]))
            .collect();
        let out = render_text("r", "", &cands, 2, false, &pos);
        assert_eq!(
            out,
            "読み: r\n 1  c0\n 2  c1\n-- page 2 --\n 1  c2\n 2  c3\n-- page 3 --\n 1  c4\n"
        );
    }

    #[test]
    fn text_no_trailing_page_marker_on_exact_fit() {
        let cands: Vec<Candidate> = (0..2)
            .map(|i| cand(&format!("c{i}"), i, CandidateOrigin::Dictionary, vec![]))
            .collect();
        let out = render_text("r", "", &cands, 2, false, &pos);
        assert!(!out.contains("-- page"));
    }

    #[test]
    fn text_empty_candidates() {
        let out = render_text("zzz", "", &[], 9, false, &pos);
        assert_eq!(out, "読み: zzz\n");
    }

    #[test]
    fn tsv_basic() {
        let out = render_tsv(&sample(), &pos);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "1\t品詞\t7392\t7392\tdictionary\t品詞/ひんし/名詞,普通名詞,一般,*,*,*"
        );
        assert_eq!(lines[2], "3\tひんし\t9000\t9000\thiragana\t");
    }

    #[test]
    fn tsv_score_differs_from_base_cost_and_katakana() {
        let mut c = cand("ヒンシ", 100, CandidateOrigin::Katakana, vec![]);
        c.score = 42;
        let out = render_tsv(&[c], &pos);
        assert_eq!(out, "1\tヒンシ\t42\t100\tkatakana\t\n");
    }

    #[test]
    fn tsv_joins_segments_and_sanitizes() {
        let c = cand(
            "a\tb",
            1,
            CandidateOrigin::Dictionary,
            vec![seg("変換", "へんかん", 1), seg("する", "する", 2)],
        );
        let out = render_tsv(&[c], &pos);
        let cols: Vec<&str> = out.trim_end().split('\t').collect();
        assert_eq!(cols.len(), 6);
        assert_eq!(cols[1], "a b");
        assert_eq!(
            cols[5],
            "変換/へんかん/名詞,普通名詞,一般,*,*,* + する/する/動詞,非自立可能,*,*,サ行変格,終止形-一般"
        );
    }

    #[test]
    fn tsv_shows_rewriter_origin_with_name_and_format() {
        let c = cand(
            "2026年10月11日",
            1,
            CandidateOrigin::Rewriter {
                name: "date",
                format: 2,
            },
            vec![],
        );
        let out = render_tsv(&[c], &pos);
        assert_eq!(out, "1\t2026年10月11日\t1\t1\trewriter:date:2\t\n");
    }

    #[test]
    fn pending_notice_message() {
        assert_eq!(pending_notice(""), None);
        assert_eq!(
            pending_notice("k").as_deref(),
            Some("未確定: k（変換対象外）")
        );
    }
}
