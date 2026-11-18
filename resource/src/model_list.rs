//! 入っている自由指定のモデルの一覧（`models/MANIFEST.json`。ADR 0060 §6、§7.2。D-4）。
//!
//! - 書くのはエンジンの取り込みだけ（inbox の束を `models/` に移すとき）。読むのもエンジン。
//! - **この一覧は信用しない**（利用者が書ける場所のファイル。D-4 の条件 2・3）。読みには上限を置き、知っている欄だけを、
//!   形を確かめてから受ける。1 件でも形が合わなければ、一覧ごと壊れたものとして扱う（呼び出し側は既定のモデルに戻す）。
//! - 檻の段（プリセットか）は、この一覧から決めない。決めるのは、数えた sha256 がアプリに埋めた表にあるかだけ。
//! - ここに書く sha256 は、取り込みのときに数えた値。起動のたびに照合するのは**破損の検知**のため。同じ利用者なら
//!   一覧ごと書き換えられるので、改ざんの守りには数えない（D-4 の条件 7）。
//! - 名前（ID、ファイル名）は sha256 から作る（`user-<先頭 12 桁>`）。一覧の文字列から、パスを作らない。

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::json;

use crate::error::ResourceError;
use crate::manifest::{Component, ComponentKind, Created, FileEntry, Manifest, Origin};
use crate::sha256::is_sha256_hex;

/// 一覧のファイルの名前（`models/` の下）
pub const MODEL_LIST_FILE_NAME: &str = "MANIFEST.json";
/// 自由指定のモデルの ID の頭（`user-<sha256 の先頭 12 桁>`。`ikari-data` の束の名前と同じ）
pub const USER_MODEL_PREFIX: &str = "user-";
/// 自由指定のモデルの ID に使う sha256 の桁数
pub const USER_MODEL_HEX_DIGITS: usize = 12;
/// モデルのファイルの拡張子
pub const MODEL_FILE_EXTENSION: &str = ".gguf";
/// 自由指定のモデルの大きさの上限（バイト）。ADR 0060 §4 の値は未定なので、仮に 4GiB（計測なし。未定の値に合わせて直す）
pub const MODEL_MAX_SIZE: u64 = 4 * 1024 * 1024 * 1024;
/// 表示名の長さの上限（文字）。`ikari-data --name` と同じ
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;
/// 形式の語の長さの上限（バイト）
const MAX_FORMAT_WORD_LEN: usize = 32;
/// 一覧のモデルの形式の版（manifest の `format`）
const MODEL_FORMAT_VERSION: u32 = 1;
/// manifest の `profile` の欄
const PROFILE_FORMAT: &str = "format";
const PROFILE_NAME: &str = "name";

/// 自由指定のモデルの ID（`user-` と 16 進 12 桁）。設定 `lm.model` の値と、`models/` のファイル名に使う
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserModelId([u8; USER_MODEL_HEX_DIGITS]);

impl UserModelId {
    /// `user-` と小文字 16 進 12 桁だけを受ける
    pub fn parse(s: &str) -> Option<UserModelId> {
        let hex = s.strip_prefix(USER_MODEL_PREFIX)?;
        let bytes: [u8; USER_MODEL_HEX_DIGITS] = hex.as_bytes().try_into().ok()?;
        bytes
            .iter()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            .then_some(UserModelId(bytes))
    }

    /// sha256（小文字 16 進 64 文字）の先頭から作る
    pub fn from_sha256(sha256: &str) -> Option<UserModelId> {
        if !is_sha256_hex(sha256) {
            return None;
        }
        Self::parse(&format!(
            "{USER_MODEL_PREFIX}{}",
            &sha256[..USER_MODEL_HEX_DIGITS]
        ))
    }

    /// 16 進の部分（作るときに確かめてあるので、常に ASCII）
    fn hex(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }

    /// `models/` の中のファイル名（`user-<12 桁>.gguf`）
    pub fn file_name(&self) -> String {
        format!("{self}{MODEL_FILE_EXTENSION}")
    }
}

impl fmt::Display for UserModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{USER_MODEL_PREFIX}{}", self.hex())
    }
}

impl fmt::Debug for UserModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserModelId({self})")
    }
}

/// 一覧の 1 件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserModelEntry {
    pub id: UserModelId,
    pub size: u64,
    /// 取り込みのときに数えた値（小文字 16 進 64 文字）
    pub sha256: String,
    /// 形式の語（`PromptFormat` の語。どの語を受けるかは読む側が渡す）
    pub format: String,
    /// 表示名（掃除して長さを切ったもの。表示にだけ使う）
    pub display_name: Option<String>,
}

impl UserModelEntry {
    /// `models/` の中のファイル名
    pub fn file_name(&self) -> String {
        self.id.file_name()
    }
}

/// 入っている自由指定のモデルの一覧
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelList {
    entries: BTreeMap<UserModelId, UserModelEntry>,
}

impl ModelList {
    /// `models/MANIFEST.json` を読む。無ければ空の一覧。壊れていれば誤り（`formats` は受ける形式の語）
    pub fn read(models_dir: &Path, formats: &[&str]) -> Result<ModelList, ResourceError> {
        match Manifest::read(&models_dir.join(MODEL_LIST_FILE_NAME)) {
            Ok(manifest) => Self::from_manifest(&manifest, formats),
            Err(ResourceError::ManifestMissing) => Ok(ModelList::default()),
            Err(e) => Err(e),
        }
    }

    /// manifest から作る。知っている欄だけを、形を確かめて受ける
    pub fn from_manifest(
        manifest: &Manifest,
        formats: &[&str],
    ) -> Result<ModelList, ResourceError> {
        let mut entries = BTreeMap::new();
        for (key, component) in &manifest.components {
            let entry = entry_from_component(key, component, formats)?;
            entries.insert(entry.id, entry);
        }
        Ok(ModelList { entries })
    }

    pub fn get(&self, id: UserModelId) -> Option<&UserModelEntry> {
        self.entries.get(&id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &UserModelEntry> {
        self.entries.values()
    }

    /// 足すか、同じ ID のものを置き換える
    pub fn upsert(&mut self, entry: UserModelEntry) {
        self.entries.insert(entry.id, entry);
    }

    /// `models/MANIFEST.json` に書く（一時ファイルに書いてから置き換える）
    pub fn write(&self, models_dir: &Path, created: Created) -> Result<(), ResourceError> {
        self.to_manifest(created)
            .write(&models_dir.join(MODEL_LIST_FILE_NAME))
    }

    fn to_manifest(&self, created: Created) -> Manifest {
        let components = self
            .entries
            .values()
            .map(|e| {
                let mut profile = serde_json::Map::new();
                profile.insert(PROFILE_FORMAT.into(), json!(e.format));
                if let Some(name) = &e.display_name {
                    profile.insert(PROFILE_NAME.into(), json!(name));
                }
                let component = Component {
                    kind: ComponentKind::Model,
                    format: MODEL_FORMAT_VERSION,
                    files: vec![FileEntry {
                        name: e.file_name(),
                        size: e.size,
                        sha256: e.sha256.clone(),
                    }],
                    builder: None,
                    sources: None,
                    profile: Some(serde_json::Value::Object(profile)),
                };
                (e.id.to_string(), component)
            })
            .collect();
        Manifest::new(created, Origin::User, components)
    }
}

fn malformed(key: &str, why: &str) -> ResourceError {
    ResourceError::ManifestMalformed(format!("{key}: {why}"))
}

fn entry_from_component(
    key: &str,
    component: &Component,
    formats: &[&str],
) -> Result<UserModelEntry, ResourceError> {
    let entry = UserModelEntry::from_component(component, formats)?;
    if key != entry.id.to_string() {
        return Err(malformed(key, "名前が sha256 と合いません"));
    }
    Ok(entry)
}

impl UserModelEntry {
    /// モデルの部品 1 つ（一覧の 1 件か、取り込む束の `model`）から作る。ID は sha256 から作り、ファイル名がそれに
    /// 合うものだけを受ける。形式は `formats` の語だけ。表示名は掃除する
    pub fn from_component(
        component: &Component,
        formats: &[&str],
    ) -> Result<UserModelEntry, ResourceError> {
        from_component(component, formats)
    }
}

fn from_component(
    component: &Component,
    formats: &[&str],
) -> Result<UserModelEntry, ResourceError> {
    let key = "model";
    if component.kind != ComponentKind::Model || component.format != MODEL_FORMAT_VERSION {
        return Err(malformed(key, "モデルの部品ではありません"));
    }
    let [file] = component.files.as_slice() else {
        return Err(malformed(key, "ファイルは 1 つです"));
    };
    // ID は sha256 から作り、ファイル名がそれに合うかを見る（一覧の文字列からパスを作らない）
    let id = UserModelId::from_sha256(&file.sha256).ok_or_else(|| malformed(key, "sha256"))?;
    if file.name != id.file_name() {
        return Err(malformed(key, "名前が sha256 と合いません"));
    }
    if file.size > MODEL_MAX_SIZE {
        return Err(malformed(key, "大きさの上限を超えています"));
    }
    let format = profile_format(component)
        .filter(|f| formats.contains(f))
        .ok_or_else(|| malformed(key, "知らない形式です"))?;
    Ok(UserModelEntry {
        id,
        size: file.size,
        sha256: file.sha256.clone(),
        format: format.to_owned(),
        display_name: profile_display_name(component),
    })
}

/// manifest の `profile.format`（形式の語の形のものだけ）
pub fn profile_format(component: &Component) -> Option<&str> {
    let word = component.profile.as_ref()?.get(PROFILE_FORMAT)?.as_str()?;
    let shaped = !word.is_empty()
        && word.len() <= MAX_FORMAT_WORD_LEN
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    shaped.then_some(word)
}

/// manifest の `profile.name` を掃除したもの（空になれば None）
fn profile_display_name(component: &Component) -> Option<String> {
    let raw = component.profile.as_ref()?.get(PROFILE_NAME)?.as_str()?;
    let name = sanitize_display_name(raw);
    (!name.is_empty()).then_some(name)
}

/// 表示を変える見えない文字か: 向きを変える文字（候補窓の掃除と同じ 12 個）、行と段落の区切り、幅のない文字、
/// タグの文字（FP-5。表示名は利用者や取得先が決める文字なので、候補より広く落とす）
fn is_invisible_or_reordering(c: char) -> bool {
    matches!(c,
        '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}'
        | '\u{200E}'
        | '\u{200F}'
        | '\u{061C}'
        | '\u{2028}'
        | '\u{2029}'
        | '\u{200B}'..='\u{200D}'
        | '\u{2060}'
        | '\u{FEFF}'
        | '\u{E0000}'..='\u{E007F}')
}

/// 表示名にする: 制御文字と、表示を変える見えない文字を落とし、前後の空白を除き、長さを切る
pub fn sanitize_display_name(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|&c| !c.is_control() && !is_invisible_or_reordering(c))
        .collect();
    kept.trim().chars().take(MAX_DISPLAY_NAME_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const FORMATS: &[&str] = &["zenz-v3", "kanakanji-edge"];

    fn entry() -> UserModelEntry {
        UserModelEntry {
            id: UserModelId::from_sha256(SHA).unwrap(),
            size: 3,
            sha256: SHA.into(),
            format: "kanakanji-edge".into(),
            display_name: Some("my model".into()),
        }
    }

    fn created() -> Created {
        Created::parse("2026-11-18T00:00:00Z").unwrap()
    }

    #[test]
    fn ids_are_user_and_twelve_lowercase_hex_digits() {
        let id = UserModelId::parse("user-ba7816bf8f01").unwrap();
        assert_eq!(id.to_string(), "user-ba7816bf8f01");
        assert_eq!(id.file_name(), "user-ba7816bf8f01.gguf");
        assert_eq!(UserModelId::from_sha256(SHA), Some(id));
        for bad in [
            "",
            "user-",
            "user-BA7816BF8F01",
            "user-ba7816bf8f0",
            "user-ba7816bf8f011",
            "user-ba7816bf8f0g",
            "zenz-small",
            "../user-ba7816bf8f01",
            "user-ba7816bf8f01.gguf",
        ] {
            assert_eq!(UserModelId::parse(bad), None, "{bad:?}");
        }
        assert_eq!(UserModelId::from_sha256("abc"), None);
    }

    #[test]
    fn the_list_round_trips_and_a_missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ModelList::read(dir.path(), FORMATS).unwrap(),
            ModelList::default()
        );
        let mut list = ModelList::default();
        list.upsert(entry());
        list.write(dir.path(), created()).unwrap();
        let read = ModelList::read(dir.path(), FORMATS).unwrap();
        assert_eq!(read, list);
        assert_eq!(read.get(entry().id), Some(&entry()));
    }

    fn list_json_with(edit: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut list = ModelList::default();
        list.upsert(entry());
        let mut v: serde_json::Value =
            serde_json::from_str(&list.to_manifest(created()).to_json().unwrap()).unwrap();
        edit(&mut v);
        v.to_string()
    }

    /// 一覧の JSON を書き換える手
    type Edit<'a> = Box<dyn FnOnce(&mut serde_json::Value) + 'a>;

    fn parse(json: &str) -> Result<ModelList, ResourceError> {
        ModelList::from_manifest(&Manifest::from_json(json)?, FORMATS)
    }

    #[test]
    fn entries_that_do_not_match_their_sha256_or_format_are_refused() {
        let key = "user-ba7816bf8f01";
        let cases: Vec<Edit<'_>> = vec![
            // 欄の名前が sha256 と合わない
            Box::new(|v| {
                let c = v["components"][key].take();
                v["components"]["user-000000000000"] = c;
            }),
            // ファイル名が ID と合わない
            Box::new(|v| v["components"][key]["files"][0]["name"] = json!("other.gguf")),
            // 知らない形式
            Box::new(|v| v["components"][key]["profile"]["format"] = json!("gpt")),
            // 形式が無い
            Box::new(|v| v["components"][key]["profile"] = json!(null)),
            // ファイルが 2 つ
            Box::new(|v| {
                let f = v["components"][key]["files"][0].clone();
                let mut g = f.clone();
                g["name"] = json!("x.gguf");
                v["components"][key]["files"] = json!([f, g]);
            }),
            // モデルでない部品
            Box::new(|v| v["components"][key]["kind"] = json!("table")),
            // 大きさの上限
            Box::new(|v| v["components"][key]["files"][0]["size"] = json!(MODEL_MAX_SIZE + 1)),
        ];
        for (i, edit) in cases.into_iter().enumerate() {
            let json = list_json_with(edit);
            assert!(parse(&json).is_err(), "case {i}: {json}");
        }
        // 「プリセット」と書いてあっても、一覧の 1 件として読むだけ（段は sha256 の表で決める。読む側のテスト）
        let json = list_json_with(|v| v["origin"] = json!("preset"));
        assert!(parse(&json).is_ok());
    }

    #[test]
    fn display_names_are_cleaned_and_cut() {
        assert_eq!(sanitize_display_name("  my\tmodel\n "), "mymodel");
        assert_eq!(
            sanitize_display_name("a\u{202E}b\u{2028}c\u{200B}d\u{FEFF}e\u{E0041}f"),
            "abcdef"
        );
        assert_eq!(
            sanitize_display_name(&"あ".repeat(MAX_DISPLAY_NAME_CHARS + 5))
                .chars()
                .count(),
            MAX_DISPLAY_NAME_CHARS
        );
        let json = list_json_with(|v| {
            v["components"]["user-ba7816bf8f01"]["profile"]["name"] = json!("\u{202E}\u{200B}")
        });
        assert_eq!(
            parse(&json).unwrap().entries().next().unwrap().display_name,
            None
        );
    }
}
