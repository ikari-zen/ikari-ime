//! 管理の要求（設定アプリ → 殻 → エンジン。ADR 0005 §8.3）の組み立てと、入力欄の検証。
//! 今の `SettingsModel` の属性プロファイル（ProfileGet / ProfileSet）と `DictionaryRules`（DictionaryModel.swift）。
//! 送受信は Bridge（`shell-core::bridge`、L-B）の仕事で、ここは要求の中身だけを作る。
//!
//! 移していないもの: `DictionaryRules.filter`（検索語の畳み込みに、かな・全角半角・大文字小文字の正規化が要る。
//! `Cargo.toml` に Unicode の正規化の依存を足す判断が要るため、Swift に残す）。

use ikari_proto::v1;

/// 属性プロファイルの分野ごとの重み（入力メニューと同じ型）。
pub use crate::menu::ProfileWeight;

/// 属性プロファイルの 1 行（入力メニューの `ProfileDomain` と同じ型）。
pub type ProfileDomainRow = crate::menu::ProfileDomain;

/// 読めない重みは `Off`（`Unspecified` と未知の値）
pub fn profile_weight_from_proto(kind: i32) -> ProfileWeight {
    match v1::ProfileWeightKind::try_from(kind) {
        Ok(v1::ProfileWeightKind::ProfileWeightSometimes) => ProfileWeight::Sometimes,
        Ok(v1::ProfileWeightKind::ProfileWeightOften) => ProfileWeight::Often,
        _ => ProfileWeight::Off,
    }
}

pub fn profile_weight_to_proto(weight: ProfileWeight) -> i32 {
    match weight {
        ProfileWeight::Off => v1::ProfileWeightKind::ProfileWeightOff as i32,
        ProfileWeight::Sometimes => v1::ProfileWeightKind::ProfileWeightSometimes as i32,
        ProfileWeight::Often => v1::ProfileWeightKind::ProfileWeightOften as i32,
    }
}

pub fn profile_get_request() -> v1::ProfileGet {
    v1::ProfileGet {}
}

/// 分野ごとの重み。`ProfileSet` は全体を置き換えるので、全分野を送る（OFF は書かない）。
/// `id` の分野がない、または重みが同じなら `None`（送らない）
pub fn profile_set_request(
    rows: &[ProfileDomainRow],
    id: &str,
    weight: ProfileWeight,
) -> Option<v1::ProfileSet> {
    let index = rows.iter().position(|row| row.id == id)?;
    if rows[index].weight == weight {
        return None;
    }
    let domains = rows
        .iter()
        .enumerate()
        .map(|(i, row)| ProfileDomainRow {
            weight: if i == index { weight } else { row.weight },
            ..row.clone()
        })
        .filter(|row| row.weight != ProfileWeight::Off)
        .map(|row| v1::ProfileDomainState {
            id: row.id,
            name: row.name,
            weight: profile_weight_to_proto(row.weight),
        })
        .collect();
    Some(v1::ProfileSet { domains })
}

/// 返事の行から画面の行へ
pub fn profile_rows(reply: &v1::ProfileReply) -> Vec<ProfileDomainRow> {
    reply
        .domains
        .iter()
        .map(|state| ProfileDomainRow {
            id: state.id.clone(),
            name: state.name.clone(),
            weight: profile_weight_from_proto(state.weight),
        })
        .collect()
}

// ---------------------------------------------------------------- ユーザー辞書（DictionaryRules）

/// 追加の入力欄
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DictionaryDraft {
    pub reading: String,
    pub surface: String,
    /// 簡略品詞の名前（`PosOption.name`）
    pub pos_name: String,
    /// 活用型。`PosOption.needs_conj_type` のときだけ使う
    pub conj_type: String,
    pub comment: String,
}

/// 入力欄の問題。文言は UI（Swift）が決める
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftProblem {
    ReadingEmpty,
    SurfaceEmpty,
    PosNotChosen,
    ConjTypeNeeded,
    /// 辞書は TSV なので、タブと改行は入れられない（エンジンも拒む）
    ControlCharacters,
}

/// 簡略品詞と活用型をつなぐ文字（`動詞:五段-ラ行`。core の `VERB_TYPE_SEPARATOR`）
pub const CONJ_SEPARATOR: char = ':';

/// 入力欄を要求の項目にする。読みの正規化と品詞の検証はエンジンが行う（ここは明らかな入力漏れだけ弾く）
pub fn dictionary_entry_from_draft(
    draft: &DictionaryDraft,
    options: &[v1::PosOption],
) -> Result<v1::UserDictEntry, DraftProblem> {
    let reading = draft.reading.trim();
    let surface = draft.surface.trim();
    let comment = draft.comment.trim();
    if reading.is_empty() {
        return Err(DraftProblem::ReadingEmpty);
    }
    if surface.is_empty() {
        return Err(DraftProblem::SurfaceEmpty);
    }
    let option = options
        .iter()
        .find(|option| option.name == draft.pos_name)
        .ok_or(DraftProblem::PosNotChosen)?;
    if [reading, surface, comment]
        .iter()
        .any(|text| text.contains(['\t', '\n', '\r']))
    {
        return Err(DraftProblem::ControlCharacters);
    }
    let mut pos = option.name.clone();
    if option.needs_conj_type {
        if draft.conj_type.is_empty() {
            return Err(DraftProblem::ConjTypeNeeded);
        }
        pos.push(CONJ_SEPARATOR);
        pos.push_str(&draft.conj_type);
    }
    Ok(v1::UserDictEntry {
        reading: reading.to_string(),
        surface: surface.to_string(),
        pos,
        comment: comment.to_string(),
    })
}

/// 一覧に出す品詞。`動詞:五段-ラ行` は `動詞（五段-ラ行）`
pub fn pos_display(pos: &str) -> String {
    match pos.split_once(CONJ_SEPARATOR) {
        Some((head, conj)) => format!("{head}（{conj}）"),
        None => pos.to_string(),
    }
}

/// 表示順: 読み、表記の順
pub fn sort_dictionary_entries(entries: &mut [v1::UserDictEntry]) {
    entries.sort_by(|a, b| (&a.reading, &a.surface).cmp(&(&b.reading, &b.surface)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<ProfileDomainRow> {
        [
            ("it", "IT", ProfileWeight::Often),
            ("law", "法律", ProfileWeight::Off),
            ("med", "医学", ProfileWeight::Sometimes),
        ]
        .into_iter()
        .map(|(id, name, weight)| ProfileDomainRow {
            id: id.into(),
            name: name.into(),
            weight,
        })
        .collect()
    }

    #[test]
    fn profile_set_sends_every_domain_except_off() {
        let request = profile_set_request(&rows(), "law", ProfileWeight::Sometimes).unwrap();
        let ids: Vec<&str> = request.domains.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["it", "law", "med"]);
        assert_eq!(
            request.domains[1].weight,
            v1::ProfileWeightKind::ProfileWeightSometimes as i32
        );
        // OFF にした分野は送らない
        let request = profile_set_request(&rows(), "it", ProfileWeight::Off).unwrap();
        let ids: Vec<&str> = request.domains.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["med"]);
    }

    #[test]
    fn profile_set_is_skipped_for_unknown_or_unchanged() {
        assert!(profile_set_request(&rows(), "nope", ProfileWeight::Often).is_none());
        assert!(profile_set_request(&rows(), "it", ProfileWeight::Often).is_none());
    }

    #[test]
    fn profile_weights_round_trip_and_unknown_is_off() {
        for weight in [
            ProfileWeight::Off,
            ProfileWeight::Sometimes,
            ProfileWeight::Often,
        ] {
            assert_eq!(
                profile_weight_from_proto(profile_weight_to_proto(weight)),
                weight
            );
        }
        assert_eq!(profile_weight_from_proto(0), ProfileWeight::Off);
        assert_eq!(profile_weight_from_proto(99), ProfileWeight::Off);
        let reply = v1::ProfileReply {
            domains: vec![v1::ProfileDomainState {
                id: "it".into(),
                name: "IT".into(),
                weight: v1::ProfileWeightKind::ProfileWeightOften as i32,
            }],
        };
        assert_eq!(profile_rows(&reply)[0].weight, ProfileWeight::Often);
    }

    fn options() -> Vec<v1::PosOption> {
        vec![
            v1::PosOption {
                name: "名詞".into(),
                needs_conj_type: false,
            },
            v1::PosOption {
                name: "動詞".into(),
                needs_conj_type: true,
            },
        ]
    }

    fn draft(reading: &str, surface: &str, pos: &str) -> DictionaryDraft {
        DictionaryDraft {
            reading: reading.into(),
            surface: surface.into(),
            pos_name: pos.into(),
            ..Default::default()
        }
    }

    #[test]
    fn draft_becomes_an_entry_with_trimmed_fields() {
        let mut d = draft(" ひんし ", "品詞\u{3000}", "名詞");
        d.comment = " メモ\n".into();
        let entry = dictionary_entry_from_draft(&d, &options()).unwrap();
        assert_eq!(
            (
                entry.reading.as_str(),
                entry.surface.as_str(),
                entry.pos.as_str(),
                entry.comment.as_str()
            ),
            ("ひんし", "品詞", "名詞", "メモ")
        );
    }

    #[test]
    fn draft_problems_are_reported_in_a_fixed_order() {
        let o = options();
        let problem = |d: &DictionaryDraft| dictionary_entry_from_draft(d, &o).unwrap_err();
        assert_eq!(
            problem(&draft(" ", "a", "名詞")),
            DraftProblem::ReadingEmpty
        );
        assert_eq!(
            problem(&draft("あ", "", "名詞")),
            DraftProblem::SurfaceEmpty
        );
        assert_eq!(problem(&draft("あ", "a", "")), DraftProblem::PosNotChosen);
        assert_eq!(
            problem(&draft("あ", "a", "形容詞")),
            DraftProblem::PosNotChosen
        );
        assert_eq!(
            problem(&draft("あ", "a\tb", "名詞")),
            DraftProblem::ControlCharacters
        );
        let mut with_comment = draft("あ", "a", "名詞");
        with_comment.comment = "x\ry".into();
        assert_eq!(problem(&with_comment), DraftProblem::ControlCharacters);
        assert_eq!(
            problem(&draft("あ", "a", "動詞")),
            DraftProblem::ConjTypeNeeded
        );
    }

    #[test]
    fn verb_entries_join_the_conjugation_type() {
        let mut d = draft("はしる", "走る", "動詞");
        d.conj_type = "五段-ラ行".into();
        let entry = dictionary_entry_from_draft(&d, &options()).unwrap();
        assert_eq!(entry.pos, "動詞:五段-ラ行");
        assert_eq!(pos_display(&entry.pos), "動詞（五段-ラ行）");
        assert_eq!(pos_display("名詞"), "名詞");
    }

    #[test]
    fn entries_sort_by_reading_then_surface() {
        let entry = |reading: &str, surface: &str| v1::UserDictEntry {
            reading: reading.into(),
            surface: surface.into(),
            ..Default::default()
        };
        let mut entries = vec![entry("い", "b"), entry("あ", "z"), entry("い", "a")];
        sort_dictionary_entries(&mut entries);
        let order: Vec<(&str, &str)> = entries
            .iter()
            .map(|e| (e.reading.as_str(), e.surface.as_str()))
            .collect();
        assert_eq!(order, vec![("あ", "z"), ("い", "a"), ("い", "b")]);
    }
}
