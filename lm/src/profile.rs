//! モデルの記述（profile。ADR 0060 §3）。モデルごとの差は、ここにだけ書く。
//!
//! - [`PromptFormat`]: プロンプトの形式（旧 `ModelKind`）。新しい形式は、enum の値と `build_prompt_for` の腕を
//!   1 つずつ足して加える。条件（プロフィール、話題、右文脈）を使うかは [`PromptFormat::uses_conditions`] に聞く。
//! - [`ModelProfile`]: ID、形式、`alpha`、上限（[`LmLimits`]）、アダプタの元モデル。
//! - 特殊トークン、BOS、終端、トークナイザの上書きは持たない（今の汎用の処理で足りている。ADR 0060 §3）。
//!
//! 優先の順: profile の値 < `--ranker-config` の `zenz.*`（開発と評価だけ。製品は `zenz.*` を読まない）。

/// プロンプトの形式。採点の仕組み（プロンプトの続きとして候補を出す確率）は同じ
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PromptFormat {
    /// zenz v3: `[EE03 プロフィール][EE04 話題] [EE02 左文脈] EE00 読み EE01`。条件は空なら省く
    #[default]
    ZenzV3,
    /// kanakanji-edge: `EE02 左文脈 EE00 読み EE01`。EE02 は文脈が空でも置く。プロフィールと話題は学習していない
    KanakanjiEdge,
}

impl PromptFormat {
    pub const ZENZ_V3: &'static str = "zenz-v3";
    pub const KANAKANJI_EDGE: &'static str = "kanakanji-edge";
    /// 利用者が選べる形式（設定画面の一覧の順。ADR 0060 §4）
    pub const ALL: [PromptFormat; 2] = [PromptFormat::ZenzV3, PromptFormat::KanakanjiEdge];

    /// 形式の語（manifest の `profile.format`、`ikari-data --format`、`zenz.model_kind` と同じ語）
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            Self::ZENZ_V3 => Some(Self::ZenzV3),
            Self::KANAKANJI_EDGE => Some(Self::KanakanjiEdge),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZenzV3 => Self::ZENZ_V3,
            Self::KanakanjiEdge => Self::KANAKANJI_EDGE,
        }
    }

    /// プロフィール、話題を、プロンプトに入れるか（学習していない形式には入れない）
    pub fn uses_conditions(self) -> bool {
        match self {
            Self::ZenzV3 => true,
            Self::KanakanjiEdge => false,
        }
    }

    /// 自由指定のモデルが `alpha` と上限を借りる、その形式の基準のプリセット（ADR 0060 §4）。
    /// 基準のプリセットが無い形式は zenz-v3.2 small から借りる（「未調整」と出す）
    pub fn base_profile(self) -> &'static ModelProfile {
        match self {
            Self::ZenzV3 | Self::KanakanjiEdge => &ZENZ_V3_2_SMALL,
        }
    }
}

/// モデルごとの上限（ADR 0060 §3）。KV とバッチはモデルに依らない資源の上限なので、ここに持たない（`llama.rs`）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LmLimits {
    /// プロンプトのトークン数の上限
    pub max_prompt_tokens: usize,
    /// 候補 1 つあたりのトークン数の上限（EOS を含む）
    pub max_candidate_tokens: usize,
    /// プロンプトに入れる左文脈の末尾の文字数
    pub context_chars: usize,
    /// プロンプトに入れる右文脈の先頭の文字数（0 = 渡さない。#270）
    pub right_context_chars: usize,
}

/// 今の値（zenz-v3.2 small で使ってきたもの。値は変えていない）
pub const DEFAULT_LIMITS: LmLimits = LmLimits {
    max_prompt_tokens: 512,
    max_candidate_tokens: 96,
    context_chars: 40,
    // 測定で 10 字を採用（0 字より悪化 0 件。ADR 0015 付録 C。#270）
    right_context_chars: 10,
};

/// モデルの記述
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelProfile {
    /// 設定 `lm.model` の ID（プリセットは `ikari_resource::MODEL_PRESETS` の ID と同じ）
    pub id: &'static str,
    pub format: PromptFormat,
    /// 加点は `alpha * (NLL - min NLL) * 1000`。`max_total` は全モデルで共通（ADR 0060 §2）
    pub alpha: f32,
    pub limits: LmLimits,
    /// アダプタの元モデルの ID（NFR-35・37。今は書くだけ）
    pub adapter_base: Option<&'static str>,
}

/// zenz-v3.2 small（今の既定。値は今までの `ZenzConfig` の既定と同じ）
pub const ZENZ_V3_2_SMALL: ModelProfile = ModelProfile {
    id: "zenz-small",
    format: PromptFormat::ZenzV3,
    alpha: 1.0,
    limits: DEFAULT_LIMITS,
    adapter_base: None,
};

/// アプリに埋めた profile（プリセットと同じ順。ADR 0060 §5。足すときは評価してユーザーに聞く）
pub const MODEL_PROFILES: &[ModelProfile] = &[ZENZ_V3_2_SMALL];

/// ID から profile を引く
pub fn profile_by_id(id: &str) -> Option<&'static ModelProfile> {
    MODEL_PROFILES.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_words_round_trip_and_unknown_words_are_rejected() {
        for f in PromptFormat::ALL {
            assert_eq!(PromptFormat::parse(f.as_str()), Some(f));
        }
        assert_eq!(PromptFormat::parse("gpt"), None);
        assert_eq!(PromptFormat::default(), PromptFormat::ZenzV3);
    }

    #[test]
    fn only_zenz_uses_conditions() {
        assert!(PromptFormat::ZenzV3.uses_conditions());
        assert!(!PromptFormat::KanakanjiEdge.uses_conditions());
    }

    #[test]
    fn the_default_profile_is_zenz_small_with_the_old_values() {
        let p = profile_by_id("zenz-small").unwrap();
        assert_eq!(p.format, PromptFormat::ZenzV3);
        assert_eq!(p.alpha, 1.0);
        assert_eq!(p.limits, DEFAULT_LIMITS);
        assert!(profile_by_id("nope").is_none());
        for f in PromptFormat::ALL {
            assert!(profile_by_id(f.base_profile().id).is_some());
        }
    }
}
