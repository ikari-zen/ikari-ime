//! 診断ログの許可リスト（ADR 0034 §2.2、§4.2）。NFR-19 の最後の守り。
//!
//! 殻から届く `DiagEvent` の kind・キー・タグの値は、ここにあるものだけを書く。タグの値は一覧の外の
//! 文字列が入る経路をなくす（ユーザーの文字を構造的に書けなくする）。`validate` が確かめて、
//! 行の注記（`k=v k=v`）にする。`panel` `ctx` `via` の値の名前は殻の取り方に合わせて D4b で確定した（ADR 0034 §4.2）。

use super::pb;

/// 値（キーと語）の最大バイト数。超えたら捨てる（ADR 0034 §2.2）
pub(crate) const MAX_VALUE_BYTES: usize = 32;

/// タグのキーと、取れる値の一覧
pub(crate) struct TagSpec {
    pub key: &'static str,
    pub values: &'static [&'static str],
}

/// 事象の種類ごとの許可
pub(crate) struct KindSpec {
    pub kind: &'static str,
    pub num_keys: &'static [&'static str],
    pub tags: &'static [TagSpec],
}

impl KindSpec {
    #[cfg(test)]
    pub(crate) fn allows_num(&self, key: &str) -> bool {
        self.num_keys.contains(&key)
    }

    /// `key` のタグが `value` を取ってよいか
    #[cfg(test)]
    pub(crate) fn allows_tag(&self, key: &str, value: &str) -> bool {
        self.tags
            .iter()
            .any(|t| t.key == key && t.values.contains(&value))
    }
}

pub(crate) const KIND_SH_EPISODE: &str = "sh_episode";
pub(crate) const KIND_SH_MODE: &str = "sh_mode";
pub(crate) const KIND_SH_ENGINE: &str = "sh_engine";

const MODES: &[&str] = &["romaji", "kana", "shortcode_en", "shortcode_ja"];
const STARTS: &[&str] = &["typing", "reconvert"];
const ENDS: &[&str] = &[
    "commit_cand",
    "commit_segments",
    "commit_kana",
    "esc_clear",
    "bs_empty",
    "commit_composition",
    "deactivate",
    "mode_switch",
    "click",
    "secure",
    "shortcode_literal",
    "shortcode_pick",
];
const PREVS: &[&str] = &[
    "composing",
    "converting",
    "segments",
    "ascii_run",
    "shortcode",
    "shortcode_choice",
];
const PICK_KINDS: &[&str] = &[
    "dict",
    "hiragana",
    "katakana",
    "rewriter",
    "literal",
    "fallback",
    "emoji",
    "small_letter",
];
// 殻の `AnchorSource`（クライアントの firstRect、アクセシビリティ、同じアプリの最後の正常値、マウスのある画面の中央下）と 1 対 1
const PANELS: &[&str] = &[
    "client",
    "accessibility",
    "last_known",
    "mouse_screen",
    "none",
];
// 権限の拒否は殻から見えない（IMK は「取れなかった」としか返さない）ので `denied` はない
const CTXS: &[&str] = &["ok", "none"];
const LANGS: &[&str] = &["ja", "en"];
// `key` = ikari のキー割り当て（`mode.*`）、`os` = macOS 経由（入力メニュー、英数・かなキーなど。通知の時点では区別できない）
const VIAS: &[&str] = &["key", "os"];

pub(crate) const SCHEMA: &[KindSpec] = &[
    KindSpec {
        kind: KIND_SH_EPISODE,
        num_keys: &[
            "keys",
            "keys_to_conv",
            "rlen",
            "dur_ms",
            "pick_ms",
            "spaces",
            "cand_moves",
            "pages",
            "pick",
            "digit_pick",
            "segs",
            "seg_focus",
            "seg_resize",
            "seg_changed",
            "esc_back",
            "bs",
            "wait_us",
            "fallback",
        ],
        tags: &[
            TagSpec {
                key: "mode",
                values: MODES,
            },
            TagSpec {
                key: "start",
                values: STARTS,
            },
            TagSpec {
                key: "end",
                values: ENDS,
            },
            TagSpec {
                key: "prev",
                values: PREVS,
            },
            TagSpec {
                key: "pick_kind",
                values: PICK_KINDS,
            },
            TagSpec {
                key: "panel",
                values: PANELS,
            },
            TagSpec {
                key: "ctx",
                values: CTXS,
            },
        ],
    },
    KindSpec {
        kind: KIND_SH_MODE,
        num_keys: &[],
        tags: &[
            TagSpec {
                key: "from",
                values: LANGS,
            },
            TagSpec {
                key: "to",
                values: LANGS,
            },
            TagSpec {
                key: "via",
                values: VIAS,
            },
        ],
    },
    KindSpec {
        kind: KIND_SH_ENGINE,
        num_keys: &[
            "timeout",
            "interrupted",
            "invalidated",
            "fallback_convs",
            "breaker_open",
            "reconnect",
            "resync",
        ],
        tags: &[],
    },
];

/// kind の許可。なければ None（行ごと捨てて `diag_reject` を数える）
pub(crate) fn spec_for(kind: &str) -> Option<&'static KindSpec> {
    SCHEMA.iter().find(|s| s.kind == kind)
}

/// 1 件に入れてよい数とタグの合計（殻の不具合で巨大な行を作らないため）
pub(crate) const MAX_ITEMS: usize = 64;

/// 許可リストを通らなかった理由（固定の語。ログにはこの語だけを書く）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectReason {
    Kind,
    TooMany,
    TooLong,
    NumKey,
    TagKey,
    TagValue,
    DupKey,
    /// セッションがない（殻の `session_id` がエンジンに知られていない。`validate` では出ない）
    NoSession,
}

impl RejectReason {
    pub(crate) fn as_word(self) -> &'static str {
        match self {
            RejectReason::Kind => "kind",
            RejectReason::TooMany => "too_many",
            RejectReason::TooLong => "too_long",
            RejectReason::NumKey => "num_key",
            RejectReason::TagKey => "tag_key",
            RejectReason::TagValue => "tag_value",
            RejectReason::DupKey => "dup_key",
            RejectReason::NoSession => "no_session",
        }
    }
}

/// 通った事象。`kind` は許可リストの静的な語、`note` は `k=v k=v`（数、タグの順。送られた順）
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Accepted {
    pub kind: &'static str,
    pub note: String,
}

/// 通らなかった事象。`kind` は許可リストにある kind のときだけ入る（送られた文字列は書かない）
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rejected {
    pub reason: RejectReason,
    pub kind: Option<&'static str>,
}

/// kind、キー、タグの値、長さを許可リストで確かめる。どれか 1 つでも外れたら行ごと捨てる。
/// 戻りの `note` に入る文字列は、すべて許可リストの定数か数（送られた文字列は写さない）。
pub(crate) fn validate(event: &pb::DiagEvent) -> Result<Accepted, Rejected> {
    let reject = |reason, kind| Err(Rejected { reason, kind });
    if event.kind.len() > MAX_VALUE_BYTES {
        return reject(RejectReason::TooLong, None);
    }
    let Some(spec) = spec_for(&event.kind) else {
        return reject(RejectReason::Kind, None);
    };
    let kind = Some(spec.kind);
    if event.nums.len() + event.tags.len() > MAX_ITEMS {
        return reject(RejectReason::TooMany, kind);
    }
    let mut seen: Vec<&'static str> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    for n in &event.nums {
        if n.key.len() > MAX_VALUE_BYTES {
            return reject(RejectReason::TooLong, kind);
        }
        let Some(key) = spec.num_keys.iter().find(|k| **k == n.key) else {
            return reject(RejectReason::NumKey, kind);
        };
        if seen.contains(key) {
            return reject(RejectReason::DupKey, kind);
        }
        seen.push(key);
        parts.push(format!("{key}={}", n.value));
    }
    for t in &event.tags {
        if t.key.len() > MAX_VALUE_BYTES || t.value.len() > MAX_VALUE_BYTES {
            return reject(RejectReason::TooLong, kind);
        }
        let Some(tag) = spec.tags.iter().find(|s| s.key == t.key) else {
            return reject(RejectReason::TagKey, kind);
        };
        let Some(value) = tag.values.iter().find(|v| **v == t.value) else {
            return reject(RejectReason::TagValue, kind);
        };
        if seen.contains(&tag.key) {
            return reject(RejectReason::DupKey, kind);
        }
        seen.push(tag.key);
        parts.push(format!("{}={value}", tag.key));
    }
    Ok(Accepted {
        kind: spec.kind,
        note: parts.join(" "),
    })
}

#[cfg(test)]
mod tests {
    fn event(kind: &str, nums: &[(&str, i64)], tags: &[(&str, &str)]) -> pb::DiagEvent {
        pb::DiagEvent {
            session_id: 1,
            kind: kind.into(),
            nums: nums
                .iter()
                .map(|(k, v)| pb::DiagNum {
                    key: (*k).into(),
                    value: *v,
                })
                .collect(),
            tags: tags
                .iter()
                .map(|(k, v)| pb::DiagTag {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
        }
    }

    fn reason(e: &pb::DiagEvent) -> RejectReason {
        validate(e).unwrap_err().reason
    }

    #[test]
    fn validate_accepts_listed_values() {
        let ok = validate(&event(
            "sh_episode",
            &[("keys", 9), ("bs", -1)],
            &[("mode", "romaji"), ("end", "commit_cand")],
        ))
        .unwrap();
        assert_eq!(ok.kind, "sh_episode");
        assert_eq!(ok.note, "keys=9 bs=-1 mode=romaji end=commit_cand");
        // 値のない事象も通る
        assert_eq!(validate(&event("sh_engine", &[], &[])).unwrap().note, "");
    }

    #[test]
    fn validate_rejects_with_a_reason_word() {
        assert_eq!(reason(&event("convert", &[], &[])), RejectReason::Kind);
        assert_eq!(reason(&event("", &[], &[])), RejectReason::Kind);
        assert_eq!(
            reason(&event("sh_episode", &[("reading", 1)], &[])),
            RejectReason::NumKey
        );
        assert_eq!(
            reason(&event("sh_episode", &[], &[("reading", "romaji")])),
            RejectReason::TagKey
        );
        assert_eq!(
            reason(&event("sh_episode", &[], &[("mode", "こんにちは")])),
            RejectReason::TagValue
        );
        assert_eq!(
            reason(&event("sh_episode", &[("keys", 1), ("keys", 2)], &[])),
            RejectReason::DupKey
        );
        let long = "a".repeat(MAX_VALUE_BYTES + 1);
        assert_eq!(
            reason(&event("sh_episode", &[], &[("mode", &long)])),
            RejectReason::TooLong
        );
        assert_eq!(reason(&event(&long, &[], &[])), RejectReason::TooLong);
        let many: Vec<(&str, i64)> = vec![("keys", 1); MAX_ITEMS + 1];
        assert_eq!(
            reason(&event("sh_episode", &many, &[])),
            RejectReason::TooMany
        );
    }

    #[test]
    fn rejected_never_carries_the_sent_text() {
        let r = validate(&event("secret-kind", &[], &[])).unwrap_err();
        assert_eq!(r.kind, None);
        let r = validate(&event("sh_mode", &[], &[("via", "あ")])).unwrap_err();
        assert_eq!(r.kind, Some("sh_mode"));
    }

    use super::*;
    use std::collections::HashSet;

    fn is_word(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_VALUE_BYTES
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    #[test]
    fn kinds_are_unique_and_found() {
        let kinds: HashSet<_> = SCHEMA.iter().map(|s| s.kind).collect();
        assert_eq!(kinds.len(), SCHEMA.len());
        for k in [KIND_SH_EPISODE, KIND_SH_MODE, KIND_SH_ENGINE] {
            assert!(spec_for(k).is_some(), "{k}");
        }
        assert!(spec_for("convert").is_none());
        assert!(spec_for("").is_none());
    }

    #[test]
    fn keys_and_values_are_short_ascii_words() {
        for s in SCHEMA {
            assert!(is_word(s.kind), "{}", s.kind);
            for k in s.num_keys {
                assert!(is_word(k), "{k}");
            }
            for t in s.tags {
                assert!(is_word(t.key), "{}", t.key);
                assert!(!t.values.is_empty(), "{} has no values", t.key);
                for v in t.values {
                    assert!(is_word(v), "{v}");
                }
            }
        }
    }

    #[test]
    fn no_duplicates_within_a_kind() {
        for s in SCHEMA {
            let mut keys = HashSet::new();
            for k in s.num_keys {
                assert!(keys.insert(*k), "{}: dup {k}", s.kind);
            }
            for t in s.tags {
                // 数のキーとタグのキーも重ならない
                assert!(keys.insert(t.key), "{}: dup {}", s.kind, t.key);
                let vals: HashSet<_> = t.values.iter().collect();
                assert_eq!(vals.len(), t.values.len(), "{}.{}", s.kind, t.key);
            }
        }
    }

    #[test]
    fn allows_only_listed_values() {
        let ep = spec_for(KIND_SH_EPISODE).unwrap();
        assert!(ep.allows_num("keys"));
        assert!(!ep.allows_num("mode")); // タグのキーは数としては不可
        assert!(!ep.allows_num("reading"));
        assert!(ep.allows_tag("end", "commit_cand"));
        assert!(!ep.allows_tag("end", "こんにちは"));
        assert!(!ep.allows_tag("mode", "commit_cand")); // 別のキーの値は不可
        assert!(!ep.allows_tag("keys", "1")); // 数のキーはタグとしては不可
    }
}
