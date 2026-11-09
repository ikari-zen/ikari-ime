//! manifest.json の型と読み書き（ADR 0058 §5.1。土台は ADR 0033 §4.2）。
//!
//! - `files` は入れる成果物（展開後の大きさと sha256）。`sources` は元データの出所で、入れる対象ではない。
//! - 知らない `schema` と知らない `kind` は拒む。それ以外の知らないフィールドは読み飛ばす
//!   （同じ schema の中で、古いアプリが新しい任意の欄を無視できるように。`builder`・`sources` は中身を見ない）。
//! - `signature` は 0.2 では常に空（§6.2）。欄だけ持つ。

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ResourceError;
use crate::sha256::is_sha256_hex;

pub const MANIFEST_FILE_NAME: &str = "manifest.json";
/// manifest 自体の版
pub const MANIFEST_SCHEMA: u32 = 1;

/// 部品の種類（ADR 0058 §5.1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Dict,
    Table,
    Model,
}

impl ComponentKind {
    const NAMES: [&'static str; 3] = ["dict", "table", "model"];
}

/// 束の出どころ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// 利用者の手元で作った（`cargo xtask data export`）
    Export,
    /// 作者の Release（ADR 0033）
    Release,
    /// 開発者の `install-data.sh`
    Local,
}

const CREATED_LEN: usize = "2026-11-10T03:00:00Z".len();

/// 作った日時。UTC の `YYYY-MM-DDTHH:MM:SSZ` だけを受ける。この形なら文字列の順が時刻の順になる（版の比較に使う）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Created(String);

impl Created {
    pub fn parse(s: &str) -> Result<Self, String> {
        let b = s.as_bytes();
        let shape_ok = b.len() == CREATED_LEN
            && b.iter().enumerate().all(|(i, &c)| match i {
                4 | 7 => c == b'-',
                10 => c == b'T',
                13 | 16 => c == b':',
                19 => c == b'Z',
                _ => c.is_ascii_digit(),
            });
        if !shape_ok {
            return Err(format!(
                "created は YYYY-MM-DDTHH:MM:SSZ（UTC）で書きます: {s:?}"
            ));
        }
        Ok(Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Created {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::parse(&s)
    }
}

impl From<Created> for String {
    fn from(c: Created) -> String {
        c.0
    }
}

/// 入れるファイル 1 つ。大きさと sha256 は展開後のもの
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    /// 小文字 16 進 64 文字
    pub sha256: String,
}

/// 部品 1 つ（辞書の一式、郵便番号の表など）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub kind: ComponentKind,
    /// 形式の版（辞書は dict-format の `FORMAT_VERSION`、表とモデルは 1）
    pub format: u32,
    pub files: Vec<FileEntry>,
    /// 作ったコードの版（`BUILD_INFO` と同じ考え）。中身は見ない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<serde_json::Value>,
    /// 元データの出所（URL、版、sha256）。入れる対象ではない。中身は見ない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<serde_json::Value>,
}

/// 同じ部品の版の比べ方の結果（ADR 0058 §6.3）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// 今の版に載っていない、または今の版より新しい（入れる）
    Newer,
    /// ファイルの sha256 がすべて同じ（何もしない）
    Same,
    /// 今の版より古い（拒む）
    Older,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub created: Created,
    pub origin: Origin,
    /// Release のタグ（`origin = release` のとき）
    #[serde(default)]
    pub release: Option<String>,
    /// 部品の名前（`dict`、`postal`、`emoji`、`model`）から部品へ
    pub components: BTreeMap<String, Component>,
    /// 署名の欄。0.2 では空（ADR 0058 §6.2）
    #[serde(default)]
    pub signature: Option<String>,
}

impl Manifest {
    /// 新しい manifest（schema は今の版、署名は空）
    pub fn new(created: Created, origin: Origin, components: BTreeMap<String, Component>) -> Self {
        Self {
            schema: MANIFEST_SCHEMA,
            created,
            origin,
            release: None,
            components,
            signature: None,
        }
    }

    /// JSON から読む。知らない schema・kind は、壊れている（`ManifestMalformed`）と区別して拒む。
    /// 形（ファイル名の安全、sha256 の書式、重複）もここで検査する。
    pub fn from_json(text: &str) -> Result<Self, ResourceError> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| ResourceError::ManifestMalformed(e.to_string()))?;
        let schema = value
            .get("schema")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ResourceError::ManifestMalformed("schema がありません".into()))?;
        if schema != u64::from(MANIFEST_SCHEMA) {
            return Err(ResourceError::UnknownSchema {
                found: schema,
                supported: MANIFEST_SCHEMA,
            });
        }
        if let Some(components) = value
            .get("components")
            .and_then(serde_json::Value::as_object)
        {
            for (name, c) in components {
                let kind = c.get("kind").and_then(serde_json::Value::as_str);
                if let Some(kind) = kind
                    && !ComponentKind::NAMES.contains(&kind)
                {
                    return Err(ResourceError::UnknownKind {
                        component: name.clone(),
                        kind: kind.to_owned(),
                    });
                }
            }
        }
        let manifest: Self = serde_json::from_value(value)
            .map_err(|e| ResourceError::ManifestMalformed(e.to_string()))?;
        manifest.validate_shape()?;
        Ok(manifest)
    }

    pub fn to_json(&self) -> Result<String, ResourceError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| ResourceError::ManifestMalformed(e.to_string()))
    }

    /// ファイルから読む（なければ `ManifestMissing`）
    pub fn read(path: &Path) -> Result<Self, ResourceError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ResourceError::ManifestMissing);
            }
            Err(e) => return Err(ResourceError::io(path, e)),
        };
        Self::from_json(&text)
    }

    /// 一時ファイルに書いてから rename する（読む側が書きかけを見ない）
    pub fn write(&self, path: &Path) -> Result<(), ResourceError> {
        let json = self.to_json()?;
        crate::journal::write_atomic(path, json.as_bytes())
    }

    /// 束の形の検査: ファイル名が束の直下に収まる、sha256 の書式、重複なし、`manifest.json` を載せない
    fn validate_shape(&self) -> Result<(), ResourceError> {
        let mut seen = HashSet::new();
        for (cname, component) in &self.components {
            for f in &component.files {
                check_file_name(&f.name)?;
                if f.name == MANIFEST_FILE_NAME || !seen.insert(f.name.as_str()) {
                    return Err(ResourceError::ManifestMalformed(format!(
                        "{cname}: ファイル名が重複しています: {}",
                        f.name
                    )));
                }
                if !is_sha256_hex(&f.sha256) {
                    return Err(ResourceError::ManifestMalformed(format!(
                        "{}: sha256 は小文字 16 進 64 文字です",
                        f.name
                    )));
                }
            }
            if component.files.is_empty() {
                return Err(ResourceError::ManifestMalformed(format!(
                    "部品 {cname} にファイルがありません"
                )));
            }
        }
        Ok(())
    }

    /// 束の部品 `name` を、今入っている版 `installed`（`dict/MANIFEST.json` など）と比べる（§6.3）。
    /// sha256 がすべて同じなら `Same`（`created` が違っても同じ）。違うとき、`created` が古ければ `Older`。
    pub fn freshness_of(&self, name: &str, installed: &Manifest) -> Freshness {
        let (Some(mine), Some(theirs)) =
            (self.components.get(name), installed.components.get(name))
        else {
            return Freshness::Newer;
        };
        fn digests(c: &Component) -> Vec<(&str, &str)> {
            let mut v: Vec<(&str, &str)> = c
                .files
                .iter()
                .map(|f| (f.name.as_str(), f.sha256.as_str()))
                .collect();
            v.sort_unstable();
            v
        }
        if digests(mine) == digests(theirs) {
            Freshness::Same
        } else if self.created < installed.created {
            Freshness::Older
        } else {
            Freshness::Newer
        }
    }
}

/// 名前が束の直下の 1 ファイルを指すか。空、`.`、`..`、区切り（`/`、`\`）、絶対パス（`:` を含む Windows のドライブ）、NUL を拒む。
pub fn check_file_name(name: &str) -> Result<(), ResourceError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '\0'])
        || Path::new(name).is_absolute();
    if bad {
        Err(ResourceError::UnsafePath {
            name: name.to_owned(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn sample() -> Manifest {
        let mut components = BTreeMap::new();
        components.insert(
            "postal".to_owned(),
            Component {
                kind: ComponentKind::Table,
                format: 1,
                files: vec![FileEntry {
                    name: "postal.tsv".into(),
                    size: 3,
                    sha256: SHA_A.into(),
                }],
                builder: None,
                sources: Some(serde_json::json!({"abr": {"sha256": "x"}})),
            },
        );
        Manifest::new(
            Created::parse("2026-11-10T03:00:00Z").unwrap(),
            Origin::Export,
            components,
        )
    }

    #[test]
    fn roundtrip() {
        let m = sample();
        assert_eq!(Manifest::from_json(&m.to_json().unwrap()).unwrap(), m);
    }

    #[test]
    fn signature_field_is_kept_empty() {
        let json = sample().to_json().unwrap();
        assert!(json.contains("\"signature\": null"), "{json}");
    }

    #[test]
    fn unknown_schema_is_rejected_distinctly() {
        let json = sample()
            .to_json()
            .unwrap()
            .replace("\"schema\": 1", "\"schema\": 2");
        assert!(matches!(
            Manifest::from_json(&json),
            Err(ResourceError::UnknownSchema {
                found: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn unknown_kind_is_rejected_distinctly() {
        let json = sample()
            .to_json()
            .unwrap()
            .replace("\"table\"", "\"plugin\"");
        assert!(matches!(
            Manifest::from_json(&json),
            Err(ResourceError::UnknownKind { .. })
        ));
    }

    #[test]
    fn unknown_extra_fields_are_ignored() {
        let mut v: serde_json::Value = serde_json::from_str(&sample().to_json().unwrap()).unwrap();
        v["future"] = serde_json::json!({"x": 1});
        v["components"]["postal"]["note"] = serde_json::json!("hi");
        assert_eq!(Manifest::from_json(&v.to_string()).unwrap(), sample());
    }

    #[test]
    fn malformed_json_and_types_are_rejected() {
        assert!(matches!(
            Manifest::from_json("{"),
            Err(ResourceError::ManifestMalformed(_))
        ));
        assert!(matches!(
            Manifest::from_json("{}"),
            Err(ResourceError::ManifestMalformed(_))
        ));
        let json = sample()
            .to_json()
            .unwrap()
            .replace("2026-11-10T03:00:00Z", "yesterday");
        assert!(matches!(
            Manifest::from_json(&json),
            Err(ResourceError::ManifestMalformed(_))
        ));
        let json = sample().to_json().unwrap().replace(SHA_A, "ABC");
        assert!(matches!(
            Manifest::from_json(&json),
            Err(ResourceError::ManifestMalformed(_))
        ));
    }

    #[test]
    fn unsafe_names_are_rejected() {
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:x",
            "a\0b",
        ] {
            assert!(
                matches!(check_file_name(bad), Err(ResourceError::UnsafePath { .. })),
                "{bad:?}"
            );
        }
        assert!(check_file_name("system.dict").is_ok());
        let json = sample()
            .to_json()
            .unwrap()
            .replace("postal.tsv", "../postal.tsv");
        assert!(matches!(
            Manifest::from_json(&json),
            Err(ResourceError::UnsafePath { .. })
        ));
    }

    #[test]
    fn freshness() {
        let installed = sample();
        let mut same_digest_newer_date = sample();
        same_digest_newer_date.created = Created::parse("2026-12-01T00:00:00Z").unwrap();
        assert_eq!(
            same_digest_newer_date.freshness_of("postal", &installed),
            Freshness::Same
        );

        let mut changed = sample();
        changed.components.get_mut("postal").unwrap().files[0].sha256 = "b".repeat(64);
        assert_eq!(changed.freshness_of("postal", &installed), Freshness::Newer);

        let mut older = changed.clone();
        older.created = Created::parse("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(older.freshness_of("postal", &installed), Freshness::Older);
        assert_eq!(older.freshness_of("emoji", &installed), Freshness::Newer);
    }
}
