//! `scripts/lib.sh` の定数（`readonly NAME="値"` と `readonly NAME=(a b c)`）を読む。
//! シェルとの二重管理を避けるため、値は lib.sh を唯一の元にする（`$OTHER` の展開だけ対応）。

use std::collections::HashMap;
use std::path::Path;

use crate::XtaskError;

const LIB_SH: &str = "scripts/lib.sh";

#[derive(Debug, Default)]
pub struct LibConsts {
    scalars: HashMap<String, String>,
    arrays: HashMap<String, Vec<String>>,
}

/// 外来語の一覧の取得元（`scripts/lib.sh` の `WEBWORDS_LIST_*`）
#[allow(dead_code, reason = "レーン D（ADR 0055 §7）が fetch と dict で使う")]
#[derive(Debug, PartialEq, Eq)]
pub struct WebwordsList {
    /// data/raw/ からの相対パス（展開後）
    pub path: String,
    pub url: String,
    pub sha256: String,
}

impl LibConsts {
    pub fn load(root: &Path) -> Result<Self, XtaskError> {
        let path = root.join(LIB_SH);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| XtaskError(format!("{} を読めません: {e}", path.display())))?;
        Ok(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut consts = Self::default();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("readonly ") else {
                continue;
            };
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            if let Some(inner) = value.strip_prefix('(') {
                let Some(inner) = inner.split(')').next() else {
                    continue;
                };
                let items = inner
                    .split_whitespace()
                    .map(|item| consts.expand(item))
                    .collect();
                consts.arrays.insert(name.to_owned(), items);
            } else if let Some(quoted) = value.strip_prefix('"') {
                let Some(inner) = quoted.split('"').next() else {
                    continue;
                };
                let expanded = consts.expand(inner);
                consts.scalars.insert(name.to_owned(), expanded);
            }
        }
        consts
    }

    /// `$NAME` を既知の定数で置き換える。未知の名前はそのまま残す。
    fn expand(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(pos) = rest.find('$') {
            out.push_str(&rest[..pos]);
            let after = &rest[pos + 1..];
            let len = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            match self.scalars.get(&after[..len]) {
                Some(value) if len > 0 => out.push_str(value),
                _ => out.push_str(&rest[pos..pos + 1 + len]),
            }
            rest = &after[len..];
        }
        out.push_str(rest);
        out
    }

    pub fn get(&self, name: &str) -> Result<&str, XtaskError> {
        self.scalars
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| XtaskError(format!("{LIB_SH} に {name} がありません")))
    }

    /// 外来語の一覧（ADR 0055）の置き場所と取得元。URL か sha256 が空なら `None`（取得しない・辞書に組み込まない）。
    #[allow(dead_code, reason = "レーン D（ADR 0055 §7）が fetch と dict で使う")]
    pub fn webwords_list(&self) -> Result<Option<WebwordsList>, XtaskError> {
        let url = self.get("WEBWORDS_LIST_URL")?;
        let sha256 = self.get("WEBWORDS_LIST_SHA256")?;
        if url.is_empty() || sha256.is_empty() {
            return Ok(None);
        }
        Ok(Some(WebwordsList {
            path: self.get("WEBWORDS_LIST_PATH")?.to_owned(),
            url: url.to_owned(),
            sha256: sha256.to_owned(),
        }))
    }

    pub fn array(&self, name: &str) -> Result<&[String], XtaskError> {
        self.arrays
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| XtaskError(format!("{LIB_SH} に配列 {name} がありません")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars_arrays_and_expansion() {
        let c = LibConsts::parse(
            "readonly V=\"2026\"\nreadonly D=\"sudachi-$V\" # c\nreadonly F=(a.bin b.dict)\nx=1\n",
        );
        assert_eq!(c.get("D").unwrap(), "sudachi-2026");
        assert_eq!(c.array("F").unwrap(), ["a.bin", "b.dict"]);
        assert!(c.get("X").is_err());
    }

    #[test]
    fn webwords_list_is_none_until_url_and_sha256_are_set() {
        let empty = LibConsts::parse(
            "readonly WEBWORDS_LIST_PATH=\"webwords/w.tsv\"\nreadonly WEBWORDS_LIST_URL=\"\"\nreadonly WEBWORDS_LIST_SHA256=\"\"\n",
        );
        assert_eq!(empty.webwords_list().unwrap(), None);
        let set = LibConsts::parse(
            "readonly WEBWORDS_LIST_PATH=\"webwords/w.tsv\"\nreadonly WEBWORDS_LIST_URL=\"https://x/w.xz\"\nreadonly WEBWORDS_LIST_SHA256=\"ab\"\n",
        );
        assert_eq!(set.webwords_list().unwrap().unwrap().path, "webwords/w.tsv");
    }

    #[test]
    fn real_lib_sh_has_no_webwords_list_yet() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let c = LibConsts::load(&root).unwrap();
        assert_eq!(c.webwords_list().unwrap(), None);
    }

    #[test]
    fn real_lib_sh_has_what_xtask_needs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let c = LibConsts::load(&root).unwrap();
        assert_eq!(c.get("SUDACHI_DIR_NAME").unwrap(), "sudachi-20260723");
        assert_eq!(c.array("DICT_FILES").unwrap().len(), 3);
        assert!(c.get("ZENZ_MODEL_FILE").unwrap().ends_with(".gguf"));
    }
}
