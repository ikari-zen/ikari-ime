//! `matrix.def` の厳格な読み込み（ADR 0001 §3.3 の検証）。
//!
//! 先頭行 `A B`（A = 前の語の右ID数、B = 次の語の左ID数）の後に、`a b cost` の行がちょうど `A*B` 行ある。
//! 各行で `a < A`、`b < B`、(a, b) の重複なし。ダウンロードしたファイルは信用しない入力として扱う。

use std::path::Path;

use crate::{BuildError, open_reader, read_line_at};

/// 読み込んだ連接表。
#[derive(Debug)]
pub struct Matrix {
    pub prev_right_size: u16,
    pub next_left_size: u16,
    /// `costs[next_left * prev_right_size + prev_right]`（grammar.bin と同じ並び）
    pub costs: Vec<i16>,
}

fn input_error(path: &Path, line: usize, message: impl Into<String>) -> BuildError {
    BuildError::Input {
        file: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

pub fn read_matrix(path: &Path) -> Result<Matrix, BuildError> {
    let mut reader = open_reader(path)?;
    let mut line = String::new();

    // 先頭行
    if read_line_at(&mut reader, &mut line, path, 1)? == 0 {
        return Err(input_error(
            path,
            1,
            "empty file: expected `<prev_right_size> <next_left_size>`",
        ));
    }
    let mut fields = line.split_ascii_whitespace();
    let (Some(a), Some(b), None) = (fields.next(), fields.next(), fields.next()) else {
        return Err(input_error(
            path,
            1,
            "header must be `<prev_right_size> <next_left_size>`",
        ));
    };
    let parse_size = |s: &str| -> Result<u16, BuildError> {
        s.parse::<u16>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| input_error(path, 1, format!("invalid matrix size `{s}` (1..=65535)")))
    };
    let prev_right_size = parse_size(a)?;
    let next_left_size = parse_size(b)?;

    let prev = usize::from(prev_right_size);
    let next = usize::from(next_left_size);
    let total = prev * next;
    let mut costs = vec![0i16; total];
    let mut seen = vec![0u64; total.div_ceil(64)]; // 重複検出のビットマップ
    let mut rows = 0usize;

    let mut lineno = 1usize;
    loop {
        lineno += 1;
        if read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
            break;
        }
        let mut fields = line.split_ascii_whitespace();
        let (Some(a), Some(b), Some(c), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(input_error(
                path,
                lineno,
                "expected `<right_id> <left_id> <cost>`",
            ));
        };
        let (Ok(a), Ok(b), Ok(c)) = (a.parse::<u16>(), b.parse::<u16>(), c.parse::<i16>()) else {
            return Err(input_error(
                path,
                lineno,
                "ids must be 0..=65535 and cost must fit in i16",
            ));
        };
        if a >= prev_right_size {
            return Err(input_error(
                path,
                lineno,
                format!("right id {a} is out of range (prev_right_size={prev_right_size})"),
            ));
        }
        if b >= next_left_size {
            return Err(input_error(
                path,
                lineno,
                format!("left id {b} is out of range (next_left_size={next_left_size})"),
            ));
        }
        rows += 1;
        if rows > total {
            return Err(input_error(
                path,
                lineno,
                format!("too many rows: {total} expected ({prev_right_size} x {next_left_size})"),
            ));
        }
        let index = usize::from(b) * prev + usize::from(a);
        let (word, bit) = (index / 64, 1u64 << (index % 64));
        if seen[word] & bit != 0 {
            return Err(input_error(
                path,
                lineno,
                format!("duplicate pair ({a}, {b})"),
            ));
        }
        seen[word] |= bit;
        costs[index] = c;
    }
    if rows != total {
        return Err(input_error(
            path,
            lineno - 1,
            format!(
                "too few rows: {rows} found, {total} expected ({prev_right_size} x {next_left_size})"
            ),
        ));
    }
    Ok(Matrix {
        prev_right_size,
        next_left_size,
        costs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &tempfile::TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("matrix.def");
        fs::write(&path, content).unwrap();
        path
    }

    /// 2 x 3（prev_right_size=2, next_left_size=3）。転置していれば値が入れ替わる。
    const OK_2X3: &str = "2 3\n0 0 1\n1 0 2\n0 1 3\n1 1 4\n0 2 5\n1 2 6\n";

    #[test]
    fn non_square_matrix_is_not_transposed() {
        let dir = tempfile::tempdir().unwrap();
        let m = read_matrix(&write(&dir, OK_2X3)).unwrap();
        assert_eq!((m.prev_right_size, m.next_left_size), (2, 3));
        // costs[next_left * 2 + prev_right]
        assert_eq!(m.costs, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn rows_may_come_in_any_order() {
        let dir = tempfile::tempdir().unwrap();
        let m = read_matrix(&write(
            &dir,
            "2 3\n1 2 6\n0 0 1\n1 0 2\n0 1 3\n1 1 4\n0 2 5\n",
        ))
        .unwrap();
        assert_eq!(m.costs, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn negative_and_extreme_costs() {
        let dir = tempfile::tempdir().unwrap();
        let m = read_matrix(&write(&dir, "1 2\n0 0 -32768\n0 1 32767\n")).unwrap();
        assert_eq!(m.costs, vec![-32768, 32767]);
    }

    fn assert_err(content: &str, needle: &str) {
        let dir = tempfile::tempdir().unwrap();
        let err = read_matrix(&write(&dir, content)).unwrap_err().to_string();
        assert!(err.contains(needle), "`{err}` should contain `{needle}`");
    }

    #[test]
    fn too_few_rows() {
        assert_err("2 3\n0 0 1\n1 0 2\n0 1 3\n1 1 4\n0 2 5\n", "too few rows");
        assert_err("2 3\n", "too few rows");
    }

    #[test]
    fn too_many_rows() {
        assert_err(&format!("{OK_2X3}0 0 9\n"), "too many rows");
    }

    #[test]
    fn duplicate_pair_with_missing_pair() {
        // 行数は 6 で合っているが (0,0) が重複し (1,2) が欠けている
        assert_err(
            "2 3\n0 0 1\n1 0 2\n0 1 3\n1 1 4\n0 2 5\n0 0 6\n",
            "duplicate pair (0, 0)",
        );
    }

    #[test]
    fn ids_out_of_range() {
        // 転置した表（3 x 2 として書かれたもの）を 2 x 3 の見出しで読むと範囲外になる
        assert_err("2 3\n2 0 1\n", "right id 2 is out of range");
        assert_err("2 3\n0 3 1\n", "left id 3 is out of range");
    }

    #[test]
    fn malformed_lines() {
        assert_err("", "empty file");
        assert_err("2\n", "header");
        assert_err("2 3 4\n", "header");
        assert_err("0 3\n", "invalid matrix size");
        assert_err("70000 3\n", "invalid matrix size");
        assert_err("1 1\n0 0\n", "expected");
        assert_err("1 1\n0 0 1 2\n", "expected");
        assert_err("1 1\n0 0 x\n", "cost must fit");
        assert_err("1 1\n0 0 40000\n", "cost must fit");
        assert_err("1 1\n-1 0 3\n", "ids must be");
        assert_err("1 1\n0 0 1\n\n", "expected");
    }

    #[test]
    fn error_reports_line_number() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_matrix(&write(&dir, "2 3\n0 0 1\n0 5 2\n")).unwrap_err();
        match err {
            BuildError::Input { line, .. } => assert_eq!(line, 3),
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn missing_file_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_matrix(&dir.path().join("nope.def")).unwrap_err();
        assert!(matches!(err, BuildError::Io(_)));
    }
}
