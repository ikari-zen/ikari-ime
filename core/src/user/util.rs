//! ユーザーデータの共通部品（ハッシュ、TSV の補助、原子的な書き込み、日付）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ikari_dict_format::Fnv1a64;

use super::UserDataError;

const SECONDS_PER_DAY: u64 = 86_400;
/// 書き込み途中のファイルの拡張子（rename で置き換える）
pub const TMP_SUFFIX: &str = "tmp";
pub const FIELD_SEPARATOR: char = '\t';

/// FNV-1a 64。ソルト付きの特徴量、取り込み元のハッシュ、評価用の選別に使う（暗号用ではない）。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = Fnv1a64::new();
    h.update(bytes);
    h.finish()
}

/// 続きから計算する（`fnv1a64_continue(fnv1a64(a), b) == fnv1a64(a ++ b)`）。
pub fn fnv1a64_continue(state: u64, bytes: &[u8]) -> u64 {
    let mut h = Fnv1a64::from_state(state);
    h.update(bytes);
    h.finish()
}

/// 読みの正規化（カタカナをひらがなにする。`ikari_dict_format::reading::normalize_key` と同じ）。
pub fn normalize_reading(s: &str) -> String {
    ikari_dict_format::reading::normalize_key(s)
}

/// 現在の UNIX 日（1970-01-01 からの日数）。
pub fn today_unix_day() -> u32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    u32::try_from(secs / SECONDS_PER_DAY).unwrap_or(u32::MAX)
}

/// TSV のセルに入れられない文字（タブ、改行）を含むか。
pub fn has_cell_breaker(s: &str) -> bool {
    s.contains(['\t', '\n', '\r'])
}

pub fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(TMP_SUFFIX);
    path.with_file_name(name)
}

/// 一時ファイルに書いてから rename で置き換える。親ディレクトリがなければ作る。
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), UserDataError> {
    let tmp = prepare_tmp(path, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `write_atomic` の前半。一時ファイルを書いて、そのパスを返す。
pub fn prepare_tmp(path: &Path, contents: &str) -> Result<PathBuf, UserDataError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(tmp)
}

/// ファイルを読む。なければ None。
pub fn read_optional(path: &Path) -> Result<Option<String>, UserDataError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `#name<TAB>key=value<TAB>...` 形式のヘッダ行を解析する。`name` が違えば None。
pub fn parse_header<'a>(line: &'a str, name: &str) -> Option<Vec<(&'a str, &'a str)>> {
    let mut parts = line.trim_end_matches(['\r', '\n']).split(FIELD_SEPARATOR);
    if parts.next()? != name {
        return None;
    }
    let mut kv = Vec::new();
    for p in parts {
        let (k, v) = p.split_once('=')?;
        kv.push((k, v));
    }
    Some(kv)
}

/// ヘッダの `schema=<n>` を検査する。`n` が `expected` と違えば `Schema` エラー。
pub fn check_schema(file: &Path, kv: &[(&str, &str)], expected: u32) -> Result<(), UserDataError> {
    let found = kv
        .iter()
        .find(|(k, _)| *k == "schema")
        .and_then(|(_, v)| v.parse::<u32>().ok())
        .ok_or_else(|| UserDataError::Parse {
            file: file.to_path_buf(),
            line: 1,
            message: "ヘッダに schema=<番号> がありません".to_string(),
        })?;
    if found != expected {
        return Err(UserDataError::Schema { found, expected });
    }
    Ok(())
}

pub fn parse_error(file: &Path, line: usize, message: impl Into<String>) -> UserDataError {
    UserDataError::Parse {
        file: file.to_path_buf(),
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_reference_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            fnv1a64_continue(fnv1a64(b"foo"), b"bar"),
            fnv1a64(b"foobar")
        );
    }

    /// 独立に計算した値の表。実装を `ikari_dict_format::Fnv1a64` に寄せる前後で結果が変わらないことの固定。
    #[test]
    fn fnv_known_value_table() {
        let table: [(&[u8], u64); 5] = [
            (b"", 0xcbf2_9ce4_8422_2325),
            (b"a", 0xaf63_dc4c_8601_ec8c),
            (b"foobar", 0x8594_4171_f739_67e8),
            (b"ikari", 0xbe65_cab9_4edf_8281),
            ("日本語".as_bytes(), 0xee9e_e2b5_c854_ef87),
        ];
        for (input, want) in table {
            assert_eq!(fnv1a64(input), want, "{input:?}");
        }
        // 続きからの計算も同じ値になる
        let (head, tail) = b"foobar".split_at(3);
        assert_eq!(fnv1a64_continue(fnv1a64(head), tail), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn header_parsing() {
        let kv = parse_header("#x\tschema=1\tsalt=ab", "#x").unwrap();
        assert_eq!(kv, vec![("schema", "1"), ("salt", "ab")]);
        assert!(parse_header("#y\tschema=1", "#x").is_none());
        assert!(parse_header("#x\tbroken", "#x").is_none());
    }

    #[test]
    fn atomic_write_replaces_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a/b/c.tsv");
        write_atomic(&p, "one").unwrap();
        write_atomic(&p, "two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        assert!(!tmp_path(&p).exists());
        assert_eq!(read_optional(&dir.path().join("none")).unwrap(), None);
    }
}
