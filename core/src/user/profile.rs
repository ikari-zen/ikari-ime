//! 属性プロファイル（ADR 0002 §9）。プリセットの種語と、利用者の重み。
//!
//! - プリセット: `core/assets/profiles/<domain>.tsv`（バイナリに埋め込む）。1行目が
//!   `#ikari-domain<TAB>id=<id><TAB>name=<表示名>`、以降は種語の lemma（1行1語。`#` の行は注釈）。
//! - 利用者の設定: `<user_dir>/profile.tsv`。`#ikari-profile<TAB>schema=1` の後に `domain<TAB>weight`
//!   （`often` = 1.0、`sometimes` = 0.5）を 1 行ずつ。

use std::fs;
use std::path::Path;

use super::util::{
    FIELD_SEPARATOR, check_schema, has_cell_breaker, parse_error, parse_header, read_optional,
    write_atomic,
};
use super::{UserDataError, UserDir};

pub const PROFILE_SCHEMA: u32 = 1;
const SETTINGS_HEADER_NAME: &str = "#ikari-profile";
const DOMAIN_HEADER_NAME: &str = "#ikari-domain";
const COMMENT_PREFIX: char = '#';
const WEIGHT_OFTEN: &str = "often";
const WEIGHT_SOMETIMES: &str = "sometimes";
/// これ以上なら `often` として保存する
const OFTEN_THRESHOLD: f32 = 0.75;
pub const PROFILE_SECTION_NAME: &str = "profile";

/// 同梱のプリセット（フェーズ1。種語は下書きで、ユーザーの確認待ち）
const PRESET_SOURCES: [(&str, &str); 4] = [
    ("it", include_str!("../../assets/profiles/it.tsv")),
    ("infra", include_str!("../../assets/profiles/infra.tsv")),
    ("novel", include_str!("../../assets/profiles/novel.tsv")),
    (
        "business",
        include_str!("../../assets/profiles/business.tsv"),
    ),
];

/// `often` = 1.0、`sometimes` = 0.5
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileWeight(pub f32);

impl ProfileWeight {
    pub const OFTEN: ProfileWeight = ProfileWeight(1.0);
    pub const SOMETIMES: ProfileWeight = ProfileWeight(0.5);

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            WEIGHT_OFTEN => Some(Self::OFTEN),
            WEIGHT_SOMETIMES => Some(Self::SOMETIMES),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        if self.0 >= OFTEN_THRESHOLD {
            WEIGHT_OFTEN
        } else {
            WEIGHT_SOMETIMES
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProfileDomain {
    /// `it` など
    pub id: String,
    /// 表示名（zenz のプロンプトにも使う）
    pub name: String,
    pub weight: ProfileWeight,
    /// 種語の lemma
    pub seeds: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub domains: Vec<ProfileDomain>,
}

impl Profile {
    pub fn new(domains: Vec<ProfileDomain>) -> Self {
        Self { domains }
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// `<user_dir>/profile.tsv` を読み、同梱のプリセットと結びつける。ファイルがない、または分野が 0 なら None。
    pub fn load(dir: &UserDir) -> Result<Option<Profile>, UserDataError> {
        let path = dir.profile_tsv();
        let Some(text) = read_optional(&path)? else {
            return Ok(None);
        };
        let settings = Self::parse_settings_in(&text, &path)?;
        let profile = Self::from_settings(&settings)?;
        Ok((!profile.is_empty()).then_some(profile))
    }

    /// 同梱のプリセットの一覧（`it` / `infra` / `novel` / `business`）。重みは `often`。
    pub fn presets() -> Vec<ProfileDomain> {
        PRESET_SOURCES
            .iter()
            .map(|(id, src)| parse_preset(id, src))
            .collect()
    }

    pub fn preset(id: &str) -> Option<ProfileDomain> {
        PRESET_SOURCES
            .iter()
            .find(|(preset_id, _)| *preset_id == id)
            .map(|(preset_id, src)| parse_preset(preset_id, src))
    }

    /// `(分野の id, 重み)` の設定から、プリセットの種語を結びつけて作る。未知の分野はエラー。
    pub fn from_settings(settings: &[(String, ProfileWeight)]) -> Result<Profile, UserDataError> {
        let mut domains = Vec::with_capacity(settings.len());
        for (id, weight) in settings {
            let mut d = Self::preset(id).ok_or_else(|| {
                UserDataError::Invalid(format!(
                    "未知の分野です: {id}（{}）",
                    PRESET_SOURCES.map(|(i, _)| i).join(" / ")
                ))
            })?;
            d.weight = *weight;
            domains.push(d);
        }
        Ok(Profile { domains })
    }

    pub fn settings(&self) -> Vec<(String, ProfileWeight)> {
        self.domains
            .iter()
            .map(|d| (d.id.clone(), d.weight))
            .collect()
    }

    /// `profile.tsv` の中身を検証して `(分野, 重み)` にする。
    pub fn parse_settings(text: &str) -> Result<Vec<(String, ProfileWeight)>, UserDataError> {
        Self::parse_settings_in(text, Path::new(PROFILE_SECTION_NAME))
    }

    fn parse_settings_in(
        text: &str,
        file: &Path,
    ) -> Result<Vec<(String, ProfileWeight)>, UserDataError> {
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| parse_error(file, 1, "空のファイルです"))?;
        let kv = parse_header(header, SETTINGS_HEADER_NAME).ok_or_else(|| {
            parse_error(
                file,
                1,
                format!("ヘッダが `{SETTINGS_HEADER_NAME}` ではありません"),
            )
        })?;
        check_schema(file, &kv, PROFILE_SCHEMA)?;
        let mut out: Vec<(String, ProfileWeight)> = Vec::new();
        for (i, line) in lines.enumerate() {
            let n = i + 2;
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            let [id, weight] = f[..] else {
                return Err(parse_error(file, n, "`分野<TAB>重み` の2列が必要です"));
            };
            if PRESET_SOURCES.iter().all(|(preset, _)| *preset != id) {
                return Err(parse_error(file, n, format!("未知の分野です: {id}")));
            }
            let weight = ProfileWeight::parse(weight).ok_or_else(|| {
                parse_error(
                    file,
                    n,
                    format!("重みは {WEIGHT_OFTEN} か {WEIGHT_SOMETIMES} です: {weight}"),
                )
            })?;
            if out.iter().any(|(d, _)| d == id) {
                return Err(parse_error(file, n, format!("分野が重複しています: {id}")));
            }
            out.push((id.to_string(), weight));
        }
        Ok(out)
    }

    /// `profile.tsv` の中身にする。
    pub fn serialize_settings(settings: &[(String, ProfileWeight)]) -> String {
        let mut out = format!("{SETTINGS_HEADER_NAME}\tschema={PROFILE_SCHEMA}\n");
        for (id, weight) in settings {
            out.push_str(&format!("{id}\t{}\n", weight.as_str()));
        }
        out
    }

    /// `<user_dir>/profile.tsv` に保存する（一時ファイル + rename）。
    pub fn save(&self, dir: &UserDir) -> Result<(), UserDataError> {
        write_atomic(
            &dir.profile_tsv(),
            &Self::serialize_settings(&self.settings()),
        )
    }

    /// `<user_dir>/profile.tsv` を消す。なければ何もしない。
    pub fn clear(dir: &UserDir) -> Result<(), UserDataError> {
        match fs::remove_file(dir.profile_tsv()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// 同梱のプリセットは壊れていないはずなので、壊れていたら id を表示名にして種語だけ読む（パニックしない）。
fn parse_preset(id: &str, src: &str) -> ProfileDomain {
    let mut name = id.to_string();
    let mut seeds = Vec::new();
    for (i, line) in src.lines().enumerate() {
        if i == 0
            && let Some(kv) = parse_header(line, DOMAIN_HEADER_NAME)
        {
            if let Some((_, n)) = kv.iter().find(|(k, _)| *k == "name") {
                name = (*n).to_string();
            }
            continue;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with(COMMENT_PREFIX) || has_cell_breaker(line) {
            continue;
        }
        seeds.push(line.to_string());
    }
    ProfileDomain {
        id: id.to_string(),
        name,
        weight: ProfileWeight::OFTEN,
        seeds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_the_four_domains_with_enough_unique_seeds() {
        let presets = Profile::presets();
        let ids: Vec<&str> = presets.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["it", "infra", "novel", "business"]);
        for d in &presets {
            assert!(!d.name.is_empty() && d.name != d.id, "{}", d.id);
            assert!(
                (50..=200).contains(&d.seeds.len()),
                "{}: {} seeds",
                d.id,
                d.seeds.len()
            );
            let mut sorted = d.seeds.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), d.seeds.len(), "{} has duplicate seeds", d.id);
            assert_eq!(d.weight, ProfileWeight::OFTEN);
        }
    }

    #[test]
    fn preset_files_are_marked_as_drafts() {
        for (id, src) in PRESET_SOURCES {
            assert!(src.starts_with("#ikari-domain\tid="), "{id}");
            assert!(src.contains("下書き・ユーザー確認待ち"), "{id}");
            assert!(src.contains(&format!("id={id}\t")), "{id}");
        }
    }

    #[test]
    fn load_returns_none_without_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Profile::load(&UserDir::new(dir.path())).unwrap().is_none());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ud = UserDir::new(dir.path());
        let p = Profile::from_settings(&[
            ("it".to_string(), ProfileWeight::OFTEN),
            ("novel".to_string(), ProfileWeight::SOMETIMES),
        ])
        .unwrap();
        p.save(&ud).unwrap();
        let loaded = Profile::load(&ud).unwrap().unwrap();
        assert_eq!(loaded.settings(), p.settings());
        assert!(loaded.domains[0].seeds.len() >= 50);
        Profile::clear(&ud).unwrap();
        assert!(Profile::load(&ud).unwrap().is_none());
        Profile::clear(&ud).unwrap();
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let h = "#ikari-profile\tschema=1\n";
        for text in [
            "",
            "#ikari-profile\n",
            "#ikari-profile\tschema=2\n",
            "#other\tschema=1\n",
            &format!("{h}it\tmaybe\n"),
            &format!("{h}golf\toften\n"),
            &format!("{h}it\toften\nit\tsometimes\n"),
            &format!("{h}it\n"),
            &format!("{h}it\toften\textra\n"),
        ] {
            assert!(Profile::parse_settings(text).is_err(), "{text:?}");
        }
        assert!(matches!(
            Profile::parse_settings("#ikari-profile\tschema=2\n"),
            Err(UserDataError::Schema { .. })
        ));
        assert_eq!(Profile::parse_settings(h).unwrap(), vec![]);
    }

    #[test]
    fn unknown_domain_cannot_build_a_profile() {
        assert!(Profile::from_settings(&[("golf".to_string(), ProfileWeight::OFTEN)]).is_err());
    }

    #[test]
    fn weight_text_roundtrip() {
        assert_eq!(ProfileWeight::parse("often"), Some(ProfileWeight::OFTEN));
        assert_eq!(ProfileWeight::OFTEN.as_str(), "often");
        assert_eq!(ProfileWeight::SOMETIMES.as_str(), "sometimes");
        assert_eq!(ProfileWeight::parse("x"), None);
    }
}
