//! 行単位の差分（`diff` の通常の書式）。Windows には `diff` がないので、`eval snapshot --compare` の表示用に持つ。
//! 差分の取り方は LCS（最長共通部分列）。同じ長さの候補が複数あるときの選び方は GNU diff と違うことがある
//! （表示の先頭を人が読むためのもので、一致か差分ありかの判定には使わない）。

/// この数を超える比較（行数の積）は、LCS を取らずに「真ん中は全部変わった」とする（メモリを守る）。
const MAX_LCS_CELLS: usize = 16_000_000;

/// 1 つのまとまった差分。範囲は 1 始まり（a は元、b は先）。`a_len` / `b_len` が 0 なら、そちら側は追加または削除だけ。
#[derive(Debug, PartialEq, Eq)]
struct Hunk {
    a_start: usize,
    a_len: usize,
    b_start: usize,
    b_len: usize,
}

fn lines_of(text: &str) -> Vec<&str> {
    text.split_terminator('\n').collect()
}

fn hunks(a: &[&str], b: &[&str]) -> Vec<Hunk> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let am = &a[prefix..a.len() - suffix];
    let bm = &b[prefix..b.len() - suffix];
    if am.is_empty() && bm.is_empty() {
        return Vec::new();
    }
    if am.is_empty() || bm.is_empty() || am.len().saturating_mul(bm.len()) > MAX_LCS_CELLS {
        return vec![Hunk {
            a_start: prefix + 1,
            a_len: am.len(),
            b_start: prefix + 1,
            b_len: bm.len(),
        }];
    }
    // lcs[i][j] = am[i..] と bm[j..] の LCS の長さ
    let width = bm.len() + 1;
    let mut lcs = vec![0u32; (am.len() + 1) * width];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            lcs[i * width + j] = if am[i] == bm[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut open: Option<(usize, usize)> = None; // 差分の始まり (i, j)
    let mut close = |open: &mut Option<(usize, usize)>, i: usize, j: usize| {
        if let Some((si, sj)) = open.take() {
            out.push(Hunk {
                a_start: prefix + si + 1,
                a_len: i - si,
                b_start: prefix + sj + 1,
                b_len: j - sj,
            });
        }
    };
    while i < am.len() || j < bm.len() {
        if i < am.len() && j < bm.len() && am[i] == bm[j] {
            close(&mut open, i, j);
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert((i, j));
        if j >= bm.len() || (i < am.len() && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    close(&mut open, i, j);
    out
}

fn range(start: usize, len: usize) -> String {
    match len {
        0 | 1 => start.to_string(),
        _ => format!("{},{}", start, start + len - 1),
    }
}

/// 通常の書式の差分（`diff a b` の出力）。同じなら空文字列。
pub fn normal_diff(a: &str, b: &str) -> String {
    let (la, lb) = (lines_of(a), lines_of(b));
    let mut out = String::new();
    for hunk in hunks(&la, &lb) {
        // 追加と削除は、位置を「その直前の行」で言う（diff の書式）
        let left = if hunk.a_len == 0 {
            (hunk.a_start - 1).to_string()
        } else {
            range(hunk.a_start, hunk.a_len)
        };
        let right = if hunk.b_len == 0 {
            (hunk.b_start - 1).to_string()
        } else {
            range(hunk.b_start, hunk.b_len)
        };
        let kind = match (hunk.a_len, hunk.b_len) {
            (_, 0) => 'd',
            (0, _) => 'a',
            _ => 'c',
        };
        out.push_str(&format!("{left}{kind}{right}\n"));
        for line in &la[hunk.a_start - 1..hunk.a_start - 1 + hunk.a_len] {
            out.push_str(&format!("< {line}\n"));
        }
        if kind == 'c' {
            out.push_str("---\n");
        }
        for line in &lb[hunk.b_start - 1..hunk.b_start - 1 + hunk.b_len] {
            out.push_str(&format!("> {line}\n"));
        }
    }
    out
}

/// 差分の行数（`diff` の出力のうち `<` と `>` で始まる行の数）。
pub fn changed_lines(diff: &str) -> usize {
    diff.lines()
        .filter(|line| line.starts_with('<') || line.starts_with('>'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_empty() {
        assert_eq!(normal_diff("a\nb\n", "a\nb\n"), "");
    }

    #[test]
    fn change_add_delete_formats() {
        assert_eq!(
            normal_diff("a\nb\nc\n", "a\nX\nc\n"),
            "2c2\n< b\n---\n> X\n"
        );
        assert_eq!(normal_diff("a\nc\n", "a\nb\nc\n"), "1a2\n> b\n");
        assert_eq!(normal_diff("a\nb\nc\n", "a\nc\n"), "2d1\n< b\n");
        assert_eq!(
            normal_diff("a\nb\nc\nd\n", "a\nX\nY\nd\n"),
            "2,3c2,3\n< b\n< c\n---\n> X\n> Y\n"
        );
    }

    #[test]
    fn separate_hunks_and_count() {
        let diff = normal_diff("1\n2\n3\n4\n5\n", "1\nB\n3\n4\nE\nF\n");
        assert_eq!(diff, "2c2\n< 2\n---\n> B\n5c5,6\n< 5\n---\n> E\n> F\n");
        assert_eq!(changed_lines(&diff), 5);
    }

    #[test]
    fn empty_sides() {
        assert_eq!(normal_diff("", "a\n"), "0a1\n> a\n");
        assert_eq!(normal_diff("a\n", ""), "1d0\n< a\n");
    }
}
