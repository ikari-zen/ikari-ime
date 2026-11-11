//! 画面に出す固定のデータ。代表的な 6 種類（真偽、整数と範囲、列挙、文字列、キー割り当て、Bundle ID の一覧）。
//! 実際の一覧は `shell-core/src/settings/specs.rs`（W4-s1）の `SettingSpec` から作る。ここは固定。
//! キーと範囲は specs.rs の実際のもの（`paging.size` は 1..=9 など）に合わせてある。
//! 変更は標準出力に `CHANGE <key>\t<value>` で出す（値は `settings.tsv` の書き方）。

#![allow(dead_code)] // 2 つの exe がそれぞれ一部だけ使う

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Enum {
        choices: &'static [&'static str],
    },
    /// 自由な文字列（日本語が打てるかを見る入力欄。実際の設定のキーではなく、画面の部品の確認用）
    Text,
    KeyChord,
    BundleIdList,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: Kind,
    /// 初期値。Bool は "true"/"false"、BundleIdList は ',' 区切り、キー割り当ては "none" か "ctrl+j" など
    pub initial: &'static str,
}

pub fn fixed_items() -> Vec<Item> {
    vec![
        Item {
            key: "paging.wrap",
            label: "候補をぐるっと回す",
            description: "最後の候補の次に最初へ戻る",
            kind: Kind::Bool,
            initial: "false",
        },
        Item {
            key: "paging.size",
            label: "1 ページの候補の数",
            description: "1 から 9",
            kind: Kind::Int { min: 1, max: 9 },
            initial: "3",
        },
        Item {
            key: "input.method",
            label: "入力方式",
            description: "ローマ字、またはかな",
            kind: Kind::Enum {
                choices: &["romaji", "kana"],
            },
            initial: "romaji",
        },
        Item {
            key: "spike.note",
            label: "メモ（日本語の入力欄の確認用）",
            description: "ここで日本語の IME が使えるか確かめる",
            kind: Kind::Text,
            initial: "",
        },
        Item {
            key: "keymap.convert",
            label: "キー割り当て: 変換",
            description: "押したキーが入る。Backspace で none（外す）",
            kind: Kind::KeyChord,
            initial: "none",
        },
        Item {
            key: "emoji.shortcode_exclude",
            label: "絵文字のショートコードを使わないアプリ",
            description: "Bundle ID の一覧（Windows での識別子は未決）",
            kind: Kind::BundleIdList,
            initial: "com.example.terminal,com.example.editor",
        },
    ]
}

/// 変更を出す（両方式で同じ書式）
pub fn report_change(key: &str, value: &str) {
    println!("CHANGE {key}\t{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_kinds_present() {
        let items = fixed_items();
        assert_eq!(items.len(), 6);
        assert!(items.iter().any(|i| matches!(i.kind, Kind::BundleIdList)));
    }
}
