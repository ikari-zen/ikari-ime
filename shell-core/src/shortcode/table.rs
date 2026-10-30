//! 絵文字のショートコードの表（ADR 0027 §9.1。今の `ShortcodeTable.swift`）。

/// ショートコードの名前と絵文字の 1 組（`emoji.tsv` の `s` の行、`EmojiEntry` の写し）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeEntry {
    pub name: String,
    pub emoji: String,
}

impl ShortcodeEntry {
    pub fn new(name: impl Into<String>, emoji: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            emoji: emoji.into(),
        }
    }
}

/// 名前 → 絵文字。名前の昇順（UTF-8 のバイト順。エンジンの返事と同じ）の配列を二分探索する（ADR 0027 §8.2）。純粋。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShortcodeTable {
    /// 名前の昇順。エンジンが昇順で返す前提で、`new` は並べ直さない
    entries: Vec<ShortcodeEntry>,
}

impl ShortcodeTable {
    /// 空の表（データがない、まだ届いていない）。`ShortcodeMachine` は入らない。
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new(entries: Vec<ShortcodeEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[ShortcodeEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 名前が完全に一致する絵文字。名前は emoji-data の中で一意（ADR 0027 §3.5）。
    pub fn exact(&self, name: &str) -> Option<&str> {
        let index = self.lower_bound(name);
        self.entries
            .get(index)
            .filter(|entry| entry.name == name)
            .map(|entry| entry.emoji.as_str())
    }

    /// 名前の前方一致（`name` と同じ名前も含む）。並びは名前の短い順、同じ長さなら名前の順（ADR 0027 §3.5）。
    /// `name` が空なら全件ではなく空（`:` だけでは候補を出さない。§3.3）。
    pub fn prefix(&self, name: &str) -> Vec<&ShortcodeEntry> {
        if name.is_empty() {
            return Vec::new();
        }
        let mut matched: Vec<&ShortcodeEntry> = self.entries[self.lower_bound(name)..]
            .iter()
            .take_while(|entry| entry.name.starts_with(name))
            .collect();
        // 元の並び（名前の昇順）は同じ長さの中の順になる。長さと名前の両方で比べる
        matched.sort_by(|lhs, rhs| {
            lhs.name
                .len()
                .cmp(&rhs.name.len())
                .then_with(|| lhs.name.cmp(&rhs.name))
        });
        matched
    }

    /// `name` 以上の最初の位置（UTF-8 のバイト順の二分探索）。
    fn lower_bound(&self, name: &str) -> usize {
        self.entries
            .partition_point(|entry| entry.name.as_str() < name)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// テスト用の表（名前の昇順。UTF-8 のバイト順）
    pub fn sample_table() -> ShortcodeTable {
        ShortcodeTable::new(
            [
                ("+1", "👍"),
                ("100", "💯"),
                ("a", "🅰"),
                ("ab", "🆎"),
                ("abc", "🔤"),
                ("eye", "👁"),
                ("eyeglasses", "👓"),
                ("eyes", "👀"),
                ("smile", "😄"),
                ("smiley", "😃"),
                ("smirk", "😏"),
                ("tada", "🎉"),
            ]
            .into_iter()
            .map(|(name, emoji)| ShortcodeEntry::new(name, emoji))
            .collect(),
        )
    }

    fn names(entries: Vec<&ShortcodeEntry>) -> Vec<&str> {
        entries.into_iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn exact_finds_the_name_only() {
        let table = sample_table();
        assert_eq!(table.exact("eyes"), Some("👀"));
        assert_eq!(table.exact("+1"), Some("👍"));
        assert_eq!(table.exact("tada"), Some("🎉"));
        assert_eq!(table.exact("ey"), None);
        assert_eq!(table.exact(""), None);
        assert_eq!(table.exact("zzz"), None);
        assert_eq!(ShortcodeTable::empty().exact("eyes"), None);
    }

    #[test]
    fn prefix_is_ordered_by_length_then_name() {
        // 名前の短い順、同じ長さなら名前の順（§3.5）
        let table = sample_table();
        assert_eq!(names(table.prefix("ey")), ["eye", "eyes", "eyeglasses"]);
        assert_eq!(names(table.prefix("sm")), ["smile", "smirk", "smiley"]);
        assert_eq!(names(table.prefix("a")), ["a", "ab", "abc"]);
    }

    #[test]
    fn prefix_includes_the_exact_name_and_is_empty_for_no_match() {
        let table = sample_table();
        assert_eq!(names(table.prefix("eyes")), ["eyes"]);
        assert!(table.prefix("q").is_empty());
        assert!(table.prefix("eyesx").is_empty());
    }

    #[test]
    fn empty_name_gives_no_candidates() {
        let table = sample_table();
        assert!(table.prefix("").is_empty());
        assert!(ShortcodeTable::empty().prefix("a").is_empty());
        assert!(ShortcodeTable::empty().is_empty());
    }
}
