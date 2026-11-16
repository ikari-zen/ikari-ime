//! TIP が入った先のプロセスの種類と、その判定（ADR 0067 §5）。**OS に依存しない**（Mac でテストする）。
//!
//! TIP は有効になるとき（`ActivateEx`）に 1 回、自分のプロセスのトークンから事実（`ProcessFacts`）を取り、
//! `classify` で種類を決める。事実を取るのは COM の層（`com::process_facts`。`cfg(windows)`）で、ここは判断だけを持つ。
//! 種類が `ProcessKind::Normal` 以外なら「素通し」にする: `Shell` を作らない（設定ファイルを読む前に決める）、
//! エンジンにつながない、設定・ログ・計測の印を読まない・書かない、何も起動しない、候補窓に送らない、
//! キーはすべてアプリに渡す。素通しの組み込みは `com` の層が行う。

/// 整合性のレベル（トークンの必須ラベルの SID の最後の部分。`SECURITY_MANDATORY_*_RID`）
pub mod integrity {
    /// 信頼しない（`SECURITY_MANDATORY_UNTRUSTED_RID`）
    pub const UNTRUSTED: u32 = 0x0000;
    /// 低い（`SECURITY_MANDATORY_LOW_RID`）
    pub const LOW: u32 = 0x1000;
    /// 中（`SECURITY_MANDATORY_MEDIUM_RID`）。ふつうのデスクトップのアプリ
    pub const MEDIUM: u32 = 0x2000;
    /// 中より少し高い（`SECURITY_MANDATORY_MEDIUM_PLUS_RID`）。中の帯として扱う
    pub const MEDIUM_PLUS: u32 = 0x2100;
    /// 高い（`SECURITY_MANDATORY_HIGH_RID`）。管理者として実行
    pub const HIGH: u32 = 0x3000;
    /// SYSTEM（`SECURITY_MANDATORY_SYSTEM_RID`）。サービス、サインイン画面など
    pub const SYSTEM: u32 = 0x4000;
}

/// OS から取った、自分のプロセスの事実。取れなかったものは `None`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessFacts {
    /// 整合性のレベル（`integrity` の値。`GetTokenInformation(TokenIntegrityLevel)`）
    pub integrity_rid: Option<u32>,
    /// AppContainer の中か（`GetTokenInformation(TokenIsAppContainer)`）
    pub app_container: Option<bool>,
}

impl ProcessFacts {
    /// 何も取れなかった（判定できない。素通しに倒れる）
    pub const UNKNOWN: Self = Self {
        integrity_rid: None,
        app_container: None,
    };
}

/// TIP が入った先のプロセスの種類（ADR 0067 §5 の表の 4 行）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessKind {
    /// ふつうのデスクトップのアプリ（整合性が中、AppContainer でない）。通常どおり動く
    Normal,
    /// 高い権限（整合性が高以上。管理者として実行、サービス、サインイン画面や UAC の画面）。素通し
    Elevated,
    /// ストアのアプリ（AppContainer）、または整合性が中より低い。1.0.0 は素通し
    Sandboxed,
    /// トークンを読めず、判定できない。素通し（安全な側）
    Unknown,
}

impl ProcessKind {
    /// 素通しにするか（`Normal` 以外はすべて素通し）
    pub fn is_passthrough(self) -> bool {
        self != Self::Normal
    }

    /// ログに出す固定の文
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Elevated => "elevated",
            Self::Sandboxed => "sandboxed",
            Self::Unknown => "unknown",
        }
    }
}

/// 事実から種類を決める。
///
/// 順: 整合性が取れなければ `Unknown`。高以上なら `Elevated`（AppContainer かどうかに依らない）。
/// AppContainer なら `Sandboxed`。AppContainer かどうかが取れなければ `Unknown`。中より低ければ `Sandboxed`。
/// 残り（中の帯。`MEDIUM` 以上 `HIGH` 未満）が `Normal`。
pub fn classify(facts: &ProcessFacts) -> ProcessKind {
    let Some(rid) = facts.integrity_rid else {
        return ProcessKind::Unknown;
    };
    if rid >= integrity::HIGH {
        return ProcessKind::Elevated;
    }
    match facts.app_container {
        None => ProcessKind::Unknown,
        Some(true) => ProcessKind::Sandboxed,
        Some(false) if rid < integrity::MEDIUM => ProcessKind::Sandboxed,
        Some(false) => ProcessKind::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(rid: Option<u32>, app_container: Option<bool>) -> ProcessFacts {
        ProcessFacts {
            integrity_rid: rid,
            app_container,
        }
    }

    #[test]
    fn table() {
        use ProcessKind::*;
        use integrity::*;
        let cases: &[(&str, ProcessFacts, ProcessKind)] = &[
            ("desktop app", facts(Some(MEDIUM), Some(false)), Normal),
            ("medium plus", facts(Some(MEDIUM_PLUS), Some(false)), Normal),
            (
                "run as administrator",
                facts(Some(HIGH), Some(false)),
                Elevated,
            ),
            ("system", facts(Some(SYSTEM), Some(false)), Elevated),
            (
                "above system",
                facts(Some(SYSTEM + 0x1000), Some(false)),
                Elevated,
            ),
            ("high, container unknown", facts(Some(HIGH), None), Elevated),
            ("store app", facts(Some(LOW), Some(true)), Sandboxed),
            (
                "container at medium",
                facts(Some(MEDIUM), Some(true)),
                Sandboxed,
            ),
            ("low integrity", facts(Some(LOW), Some(false)), Sandboxed),
            ("untrusted", facts(Some(UNTRUSTED), Some(false)), Sandboxed),
            (
                "just below medium",
                facts(Some(MEDIUM - 1), Some(false)),
                Sandboxed,
            ),
            (
                "just below high",
                facts(Some(HIGH - 1), Some(false)),
                Normal,
            ),
            ("no integrity", facts(None, Some(false)), Unknown),
            (
                "medium, container unknown",
                facts(Some(MEDIUM), None),
                Unknown,
            ),
            ("nothing", ProcessFacts::UNKNOWN, Unknown),
        ];
        for (name, f, expected) in cases {
            assert_eq!(classify(f), *expected, "{name}");
        }
    }

    #[test]
    fn only_normal_is_not_passthrough() {
        use ProcessKind::*;
        assert!(!Normal.is_passthrough());
        for kind in [Elevated, Sandboxed, Unknown] {
            assert!(kind.is_passthrough(), "{kind:?}");
        }
    }

    #[test]
    fn texts_are_distinct() {
        use ProcessKind::*;
        let all = [Normal, Elevated, Sandboxed, Unknown];
        let texts: std::collections::HashSet<_> = all.iter().map(|k| k.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
