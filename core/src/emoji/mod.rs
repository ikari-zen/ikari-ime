//! 絵文字の表（ショートコード `:eyes:` の名前 → 絵文字）。ADR 0027 §5.3、§9.3。
//!
//! 契約（B47 の契約コミット）: 型とシグネチャだけ。中身はレーン D が埋める。

use std::fmt;

/// `emoji.tsv` のファイル名（辞書と同じ `dict/` の下。ADR 0027 §9.3）
pub const EMOJI_TSV_FILE_NAME: &str = "emoji.tsv";

/// データ行の種類: ショートコード
const KIND_SHORTCODE: &str = "s";

/// `emoji.tsv` を読んだ表。名前の昇順（UTF-8 のバイト順）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmojiTable {
    shortcodes: Vec<(String, String)>,
}

/// `emoji.tsv` の読み込みの失敗
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmojiError {
    /// 行の形が違う（行番号は 1 始まり）
    BadLine { line: usize },
    /// 同じ名前が別の絵文字を指している
    ConflictingName { name: String },
}

impl fmt::Display for EmojiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmojiError::BadLine { line } => write!(f, "emoji.tsv: bad line {line}"),
            EmojiError::ConflictingName { name } => {
                write!(f, "emoji.tsv: conflicting name {name}")
            }
        }
    }
}

impl std::error::Error for EmojiError {}

impl EmojiTable {
    /// 空の表（`emoji.tsv` がないとき）
    pub fn empty() -> EmojiTable {
        EmojiTable::default()
    }

    /// `emoji.tsv` の本文から作る。仕様は下の `tests` のコメントを見る。
    pub fn from_tsv(text: &str) -> Result<EmojiTable, EmojiError> {
        let mut shortcodes: Vec<(String, String)> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = EmojiError::BadLine { line: i + 1 };
            let cols: Vec<&str> = line.split('\t').collect();
            let [kind, name, emoji] = cols[..] else {
                return Err(bad);
            };
            if name.is_empty() || emoji.is_empty() {
                return Err(bad);
            }
            if kind != KIND_SHORTCODE {
                continue;
            }
            shortcodes.push((name.to_owned(), emoji.to_owned()));
        }
        shortcodes.sort();
        shortcodes.dedup();
        if let Some(w) = shortcodes.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(EmojiError::ConflictingName {
                name: w[0].0.clone(),
            });
        }
        Ok(EmojiTable { shortcodes })
    }

    /// 名前の昇順（バイト順）の (名前, 絵文字)
    pub fn shortcodes(&self) -> &[(String, String)] {
        &self.shortcodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // レーン D が埋める。仕様は実データ（data/build/dict/emoji.tsv、B46 の出力。1978 行）の形から:
    //
    //   # ikari-ime emoji table v1
    //   # source<TAB>emoji-data<TAB><commit>
    //   # 列: 種類<TAB>キー<TAB>絵文字
    //   s<TAB>+1<TAB>👍        （名前は +1, -1, 100, 8ball のように数字・記号で始まるものが先頭に来る）
    //   s<TAB>eyes<TAB>👀
    //
    // - `#` で始まる行と空行は読み飛ばす（ヘッダー。実データでは 3 行）。
    // - データ行はタブ区切りの 3 列ちょうど。列の数が違う、名前か絵文字が空なら BadLine（行番号は 1 始まり）。
    // - 種類 `s`（ショートコード）だけを表に入れる（実データは 1975 行がすべて `s`）。
    //   未知の種類（段階2 の `t` など）は読み飛ばし、エラーにしない。
    // - 実データの名前は昇順（バイト順）だが、from_tsv は入力順を信用せず名前でソートして返す。
    // - 同じ名前で同じ絵文字の重複行は 1 つにする。同じ名前で別の絵文字は ConflictingName。
    //   （別名は別の行: `flag-jp` と `jp` は名前が違うので両方残る。同じ絵文字に複数の名前がありうる）
    // - 絵文字は ZWJ 列・異体字セレクタを含みうる。文字列のまま保つ（正規化しない）。
    // - 行末の `\r` は取り除く。空の入力は空の表（Ok）。

    fn pair(n: &str, e: &str) -> (String, String) {
        (n.to_owned(), e.to_owned())
    }

    #[test]
    fn parses_header_and_rows() {
        let t = EmojiTable::from_tsv(
            "# ikari-ime emoji table v1\n# source\temoji-data\tabc\n\ns\t+1\t👍\r\ns\teyes\t👀\n",
        )
        .unwrap();
        assert_eq!(t.shortcodes(), [pair("+1", "👍"), pair("eyes", "👀")]);
    }

    #[test]
    fn sorts_by_name_bytes() {
        let t = EmojiTable::from_tsv("s\tb\t2\ns\t100\t💯\ns\ta\t1\ns\ta\t1\n").unwrap();
        let names: Vec<&str> = t.shortcodes().iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["100", "a", "b"]);
        // 同じ絵文字の別名は両方残る
        let t = EmojiTable::from_tsv("s\tjp\t🇯🇵\ns\tflag-jp\t🇯🇵\n").unwrap();
        assert_eq!(t.shortcodes().len(), 2);
    }

    #[test]
    fn skips_unknown_kinds() {
        let t = EmojiTable::from_tsv("t\tねこ\t🐱\ns\tcat\t🐱\n").unwrap();
        assert_eq!(t.shortcodes(), [pair("cat", "🐱")]);
    }

    #[test]
    fn rejects_malformed_rows() {
        let bad = |text: &str, line: usize| {
            assert_eq!(
                EmojiTable::from_tsv(text),
                Err(EmojiError::BadLine { line }),
                "{text:?}"
            );
        };
        bad("# h\ns\tcat\n", 2);
        bad("s\tcat\t🐱\textra\n", 1);
        bad("s\t\t🐱\n", 1);
        bad("s\tcat\t\n", 1);
        bad("s\tcat\t🐱\n\ns\tdog\n", 3);
    }

    #[test]
    fn rejects_conflicting_names() {
        assert_eq!(
            EmojiTable::from_tsv("s\tcat\t🐱\ns\tcat\t🐈\n"),
            Err(EmojiError::ConflictingName { name: "cat".into() })
        );
    }

    #[test]
    fn empty_input_is_empty_table() {
        assert_eq!(EmojiTable::from_tsv(""), Ok(EmojiTable::empty()));
        assert_eq!(EmojiTable::from_tsv("# h\n\n"), Ok(EmojiTable::empty()));
    }

    #[test]
    fn keeps_zwj_and_variation_selectors() {
        let e = "👨\u{200d}👩\u{200d}👧";
        let v = "❤\u{fe0f}";
        let t = EmojiTable::from_tsv(&format!("s\tfamily\t{e}\ns\theart\t{v}\n")).unwrap();
        assert_eq!(t.shortcodes(), [pair("family", e), pair("heart", v)]);
    }

    #[test]
    fn reads_the_real_table_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../data/build/dict")
            .join(EMOJI_TSV_FILE_NAME);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return; // データがなければスキップ
        };
        let t = EmojiTable::from_tsv(&text).unwrap();
        assert!(t.shortcodes().len() > 1000);
        assert!(t.shortcodes().iter().any(|(n, e)| n == "eyes" && e == "👀"));
    }

    #[test]
    fn empty_table_has_no_shortcodes() {
        assert!(EmojiTable::empty().shortcodes().is_empty());
    }
}
