//! 英語のモードへの郵便番号・Unicode 表記の取り付け（今の `EnglishPostal.swift` の `extension EnglishMode`）。
//! 状態機械そのものは `crate::postal`。

use super::{EnglishMode, EnglishShortcodeEnvironment, EnglishTextOutcome};
use crate::postal::{PostalKinds, PostalLookupRequest, PostalStep};
use crate::types::KeyInput;

impl EnglishMode {
    /// 郵便番号か `u+` の語をためている（変換中の文字列か候補を出している）。
    pub fn is_postal_active(&self) -> bool {
        self.postal.is_active()
    }

    /// 打鍵を処理する。関係なければ `None`（ショートコードの口に流す）。
    pub(super) fn handle_postal(
        &mut self,
        input: &KeyInput,
        environment: &EnglishShortcodeEnvironment<'_>,
    ) -> Option<EnglishTextOutcome> {
        if self.shortcode.is_active() {
            return None;
        }
        let kinds = PostalKinds {
            te: environment.settings.postal_english,
            unicode: environment.settings.unicode_notation,
        };
        // ためている種類のスイッチが切れた。ためた文字を確定して、このキーはアプリに渡す
        if let Some(active) = self.postal.active_kind()
            && !kinds.contains(active)
        {
            let mut step = self.postal.flush()?;
            step.pass_key = true;
            return Some(self.postal_outcome(step, Some(input)));
        }
        if kinds.is_empty() {
            return None;
        }
        let can_enter = !environment.secure
            && !environment.is_repeat
            && !environment
                .settings
                .emoji_shortcode_exclude
                .iter()
                .any(|id| id == environment.client_bundle_id)
            && Self::is_word_start(self.last_character());
        let step = self.postal.handle(input, can_enter, kinds)?;
        Some(self.postal_outcome(step, Some(input)))
    }

    /// 住所の返事を受ける。状態が違えば捨てる（`None`）。
    pub fn postal_lookup_arrived(
        &mut self,
        request: &PostalLookupRequest,
        addresses: &[String],
    ) -> Option<EnglishTextOutcome> {
        let step = self.postal.lookup_arrived(request, addresses)?;
        Some(EnglishTextOutcome {
            actions: step.actions,
            pass_key: false,
            flushed: Vec::new(),
            lookup: None,
        })
    }

    /// モードの切り替え、secure、`deactivateServer`、クリックのとき、ためた文字をそのまま確定する。ためていなければ `None`。
    pub fn flush_postal(&mut self) -> Option<EnglishTextOutcome> {
        let step = self.postal.flush()?;
        Some(self.postal_outcome(step, None))
    }

    fn postal_outcome(&mut self, step: PostalStep, key: Option<&KeyInput>) -> EnglishTextOutcome {
        let mut flushed = Vec::new();
        if let Some(committed) = &step.committed
            && let Some(overflow) = self.append_committed(committed)
        {
            flushed.push(overflow);
        }
        if step.pass_key
            && let Some(key) = key
            && let Some(recorded) = self.record(key)
        {
            flushed.push(recorded);
        }
        EnglishTextOutcome {
            actions: step.actions,
            pass_key: step.pass_key,
            flushed,
            lookup: step.lookup,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::types::{KeyKind, SettingsSnapshot};

    const CODE: &str = "1000001";
    const CHIYODA: &str = "東京都千代田区千代田";

    struct Env {
        settings: SettingsSnapshot,
        bundle_id: String,
        secure: bool,
        is_repeat: bool,
    }

    impl Env {
        fn new(postal: bool) -> Self {
            let mut settings = default_settings();
            settings.postal_english = postal;
            Self {
                settings,
                bundle_id: "com.apple.TextEdit".into(),
                secure: false,
                is_repeat: false,
            }
        }

        fn unicode(postal: bool, unicode: bool) -> Self {
            let mut env = Self::new(postal);
            env.settings.unicode_notation = unicode;
            env
        }

        fn view(&self) -> EnglishShortcodeEnvironment<'_> {
            EnglishShortcodeEnvironment {
                settings: &self.settings,
                client_bundle_id: &self.bundle_id,
                secure: self.secure,
                is_repeat: self.is_repeat,
            }
        }
    }

    fn type_into(mode: &mut EnglishMode, text: &str, env: &Env) -> Option<EnglishTextOutcome> {
        let mut last = None;
        for c in text.chars() {
            last = Some(mode.handle_text(&ch(&c.to_string()), &env.view(), 0));
        }
        last
    }

    #[test]
    fn setting_defaults_to_off_and_off_changes_nothing() {
        assert!(!default_settings().postal_english);
        let mut mode = EnglishMode::new();
        let outcome = type_into(&mut mode, "te1000001", &Env::new(false)).unwrap();
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
        assert_eq!(outcome.lookup, None);
    }

    #[test]
    fn opens_when_on() {
        let mut mode = EnglishMode::new();
        let outcome = type_into(&mut mode, "te1000001", &Env::new(true)).unwrap();
        assert!(!outcome.pass_key);
        assert_eq!(outcome.lookup.map(|l| l.code), Some(CODE.to_string()));
        assert!(mode.is_postal_active());
    }

    #[test]
    fn excluded_app_secure_and_repeat_do_not_enter() {
        let mut excluded = Env::new(true);
        excluded.settings.emoji_shortcode_exclude = vec!["com.example.excluded".into()];
        excluded.bundle_id = "com.example.excluded".into();
        let mut secure = Env::new(true);
        secure.secure = true;
        let mut repeating = Env::new(true);
        repeating.is_repeat = true;
        for env in [excluded, secure, repeating] {
            let mut mode = EnglishMode::new();
            let outcome = mode.handle_text(&ch("t"), &env.view(), 0);
            assert!(outcome.pass_key);
            assert!(!mode.is_postal_active());
        }
    }

    #[test]
    fn only_at_word_start() {
        let mut mode = EnglishMode::new();
        let env = Env::new(true);
        type_into(&mut mode, "abc", &env);
        let outcome = type_into(&mut mode, "te1000001", &env).unwrap();
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
        // 空白のあとは働く
        mode.handle_text(&key(KeyKind::Space), &env.view(), 0);
        type_into(&mut mode, "te1000001", &env);
        assert!(mode.is_postal_active());
    }

    #[test]
    fn committed_text_goes_to_the_context_pool() {
        let mut mode = EnglishMode::new();
        let env = Env::new(true);
        type_into(&mut mode, "te1000001", &env);
        let escape = mode.handle_text(&key(KeyKind::Escape), &env.view(), 0);
        assert!(!escape.pass_key);
        assert_eq!(mode.pool().last_character(), Some('1'));
    }

    #[test]
    fn turning_the_setting_off_while_typing_commits_and_passes_the_key() {
        let mut mode = EnglishMode::new();
        type_into(&mut mode, "te10", &Env::new(true));
        let outcome = mode.handle_text(&ch("0"), &Env::new(false).view(), 0);
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
        // ためた文字と今のキーは、たまりに入る
        assert_eq!(mode.pool().text(), "te100");
    }

    #[test]
    fn lookup_arrived_through_the_mode_updates_the_candidates() {
        let mut mode = EnglishMode::new();
        let request = type_into(&mut mode, "te1000001", &Env::new(true))
            .unwrap()
            .lookup
            .unwrap();
        let outcome = mode.postal_lookup_arrived(&request, &[CHIYODA.to_string()]);
        assert!(!outcome.unwrap().actions.is_empty());
        mode.flush_postal();
        assert_eq!(
            mode.postal_lookup_arrived(&request, &[CHIYODA.to_string()]),
            None
        );
    }

    // Unicode 表記（ADR 0032 §9.1。issue #28）

    #[test]
    fn unicode_setting_defaults_to_off() {
        assert!(!default_settings().unicode_notation);
        let mut mode = EnglishMode::new();
        let outcome = type_into(&mut mode, "u+3042", &Env::unicode(false, false)).unwrap();
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
    }

    #[test]
    fn unicode_opens_when_on_and_needs_no_postal_setting() {
        let mut mode = EnglishMode::new();
        let outcome = type_into(&mut mode, "u+3042", &Env::unicode(false, true)).unwrap();
        assert!(!outcome.pass_key);
        assert_eq!(outcome.lookup, None);
        assert!(mode.is_postal_active());
    }

    #[test]
    fn postal_stays_off_when_only_unicode_is_on() {
        let mut mode = EnglishMode::new();
        let outcome = type_into(&mut mode, "te1000001", &Env::unicode(false, true)).unwrap();
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
    }

    #[test]
    fn unicode_turning_the_setting_off_while_typing_commits_and_passes_the_key() {
        let mut mode = EnglishMode::new();
        type_into(&mut mode, "u+30", &Env::unicode(false, true));
        let outcome = mode.handle_text(&ch("4"), &Env::unicode(false, false).view(), 0);
        assert!(outcome.pass_key);
        assert!(!mode.is_postal_active());
    }
}
