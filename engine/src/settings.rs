//! `settings.tsv` のうち、エンジンが読むキー（docs/phase2-contract.md §3）。
//!
//! 形式は `キー<TAB>値`。`#` で始まる行と空行は無視する。未知のキーと不正な値は無視して既定値を使う
//! （入力を止めない）。ファイルがなければ既定値。

use std::path::Path;
use std::time::Duration;

pub const SETTINGS_FILE_NAME: &str = "settings.tsv";

const COMMENT_PREFIX: char = '#';
const SEPARATOR: char = '\t';
const KEY_LM: &str = "engine.lm";
const KEY_IDLE_UNLOAD_SECS: &str = "engine.idle_unload_secs";
const KEY_LM_DEADLINE_MS: &str = "engine.lm_deadline_ms";
const KEY_LM_GPU: &str = "engine.lm_gpu";
const KEY_CONVERT_WAIT_MS: &str = "engine.convert_wait_ms";
const KEY_PACK_IT: &str = "dict.pack_it";
const KEY_POSTAL_JAPANESE: &str = "postal.japanese";
const KEY_POSTAL_ENGLISH: &str = "postal.english";
const KEY_POSTAL_TE_PREFIX: &str = "postal.te_prefix";
const KEY_UNICODE_NOTATION: &str = "unicode.notation";
const KEY_DATE_WEEKDAY_DATES: &str = "date.weekday_dates";
const VALUE_ON: &str = "on";
const VALUE_OFF: &str = "off";

/// `postal.japanese` の既定（住所 ↔ 郵便番号。ADR 0032）
pub const DEFAULT_POSTAL_JAPANESE: bool = true;
/// `postal.english` の既定（英語のモードで郵便番号の候補。殻が使う。ADR 0032 §9）
pub const DEFAULT_POSTAL_ENGLISH: bool = false;
/// `postal.te_prefix` の既定（`て` + 郵便番号 → 〒つき。ADR 0032 §8）
pub const DEFAULT_POSTAL_TE_PREFIX: bool = false;
/// `unicode.notation` の既定（1 文字のかな → `U+3042`）
pub const DEFAULT_UNICODE_NOTATION: bool = false;
/// `date.weekday_dates` の既定（曜日の読みに直近の日付の候補を足す。#265）
pub const DEFAULT_DATE_WEEKDAY_DATES: bool = false;

/// `engine.lm_gpu` の既定（GPU があれば使う。なければ CPU。2026-11-14 ユーザー決定。ADR 0059 §2.5）
pub const DEFAULT_LM_GPU: bool = true;

/// `engine.lm_deadline_ms` の既定（先行計算で LM を待つ締め切り。`LmBudget.max` の既定と同じ）
pub const DEFAULT_LM_DEADLINE_MS: u32 = 300;
/// `engine.lm_deadline_ms` の範囲（暫定）。範囲外は丸める
pub const LM_DEADLINE_MS_RANGE: (u32, u32) = (50, 2000);
/// `engine.convert_wait_ms` の既定（Space で先行計算を待つ上限）
pub const DEFAULT_CONVERT_WAIT_MS: u32 = 30;
/// `engine.convert_wait_ms` の範囲（暫定。0 = 待たない）。範囲外は丸める
pub const CONVERT_WAIT_MS_RANGE: (u32, u32) = (0, 200);

fn parse_switch(value: &str) -> Option<bool> {
    match value {
        VALUE_ON => Some(true),
        VALUE_OFF => Some(false),
        _ => None,
    }
}

/// 設定ファイルに書かれていたものだけを持つ（書かれていなければ None）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `engine.lm`
    pub lm: Option<bool>,
    /// `engine.idle_unload_secs`（0 = 解放しない）
    pub idle_unload_secs: Option<u32>,
    /// `engine.lm_gpu`（Vulkan のビルドだけで効く。読み取りは `lm_gpu_or_default`）
    pub lm_gpu: Option<bool>,
    /// `engine.lm_deadline_ms`（書かれた値のまま。丸めは `lm_deadline_or_default`）
    pub lm_deadline_ms: Option<u32>,
    /// `engine.convert_wait_ms`（書かれた値のまま。丸めは `convert_wait_or_default`）
    pub convert_wait_ms: Option<u32>,
    /// `dict.pack_it`（IT の語彙集。既定は on。ADR 0012 B20-4）
    pub pack_it: Option<bool>,
    /// `postal.japanese`（既定は on。ADR 0032）
    pub postal_japanese: Option<bool>,
    /// `postal.english`（既定は off。ADR 0032）
    pub postal_english: Option<bool>,
    /// `postal.te_prefix`（既定は off。ADR 0032）
    pub postal_te_prefix: Option<bool>,
    /// `unicode.notation`（既定は off）
    pub unicode_notation: Option<bool>,
    /// `date.weekday_dates`（既定は off）
    pub date_weekday_dates: Option<bool>,
}

impl Settings {
    /// LM を GPU で動かすか（既定は真。GPU が使えない機械では、真でも CPU に落ちる）
    pub fn lm_gpu_or_default(&self) -> bool {
        self.lm_gpu.unwrap_or(DEFAULT_LM_GPU)
    }

    /// 先行計算で LM を待てる締め切り。範囲外は範囲に丸める
    pub fn lm_deadline_or_default(&self) -> Duration {
        let (lo, hi) = LM_DEADLINE_MS_RANGE;
        let ms = self.lm_deadline_ms.unwrap_or(DEFAULT_LM_DEADLINE_MS);
        Duration::from_millis(u64::from(ms.clamp(lo, hi)))
    }

    /// Space で先行計算を待つ上限。範囲外は範囲に丸める
    pub fn convert_wait_or_default(&self) -> Duration {
        let (lo, hi) = CONVERT_WAIT_MS_RANGE;
        let ms = self.convert_wait_ms.unwrap_or(DEFAULT_CONVERT_WAIT_MS);
        Duration::from_millis(u64::from(ms.clamp(lo, hi)))
    }

    pub fn postal_japanese_or_default(&self) -> bool {
        self.postal_japanese.unwrap_or(DEFAULT_POSTAL_JAPANESE)
    }

    pub fn postal_english_or_default(&self) -> bool {
        self.postal_english.unwrap_or(DEFAULT_POSTAL_ENGLISH)
    }

    pub fn postal_te_prefix_or_default(&self) -> bool {
        self.postal_te_prefix.unwrap_or(DEFAULT_POSTAL_TE_PREFIX)
    }

    pub fn unicode_notation_or_default(&self) -> bool {
        self.unicode_notation.unwrap_or(DEFAULT_UNICODE_NOTATION)
    }

    pub fn date_weekday_dates_or_default(&self) -> bool {
        self.date_weekday_dates
            .unwrap_or(DEFAULT_DATE_WEEKDAY_DATES)
    }
}

pub fn parse(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(COMMENT_PREFIX) {
            continue;
        }
        let Some((key, value)) = line.split_once(SEPARATOR) else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            KEY_LM => match value {
                VALUE_ON => settings.lm = Some(true),
                VALUE_OFF => settings.lm = Some(false),
                _ => {}
            },
            KEY_LM_GPU => settings.lm_gpu = parse_switch(value),
            KEY_PACK_IT => match value {
                VALUE_ON => settings.pack_it = Some(true),
                VALUE_OFF => settings.pack_it = Some(false),
                _ => {}
            },
            KEY_POSTAL_JAPANESE => settings.postal_japanese = parse_switch(value),
            KEY_POSTAL_ENGLISH => settings.postal_english = parse_switch(value),
            KEY_POSTAL_TE_PREFIX => settings.postal_te_prefix = parse_switch(value),
            KEY_UNICODE_NOTATION => settings.unicode_notation = parse_switch(value),
            KEY_DATE_WEEKDAY_DATES => settings.date_weekday_dates = parse_switch(value),
            KEY_IDLE_UNLOAD_SECS => {
                if let Ok(secs) = value.parse::<u32>() {
                    settings.idle_unload_secs = Some(secs);
                }
            }
            KEY_LM_DEADLINE_MS => settings.lm_deadline_ms = value.parse().ok(),
            KEY_CONVERT_WAIT_MS => settings.convert_wait_ms = value.parse().ok(),
            _ => {}
        }
    }
    settings
}

/// 読めなければ既定値。
pub fn load(data_dir: &Path) -> Settings {
    ikari_textfile::read_to_string(data_dir.join(SETTINGS_FILE_NAME))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_engine_keys() {
        let s = parse("# c\n\nengine.lm\toff\nengine.idle_unload_secs\t60\ninput.method\tkana\n");
        assert_eq!(
            s,
            Settings {
                lm: Some(false),
                idle_unload_secs: Some(60),
                pack_it: None,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn load_reads_lf_crlf_and_bom_files() {
        let dir =
            std::env::temp_dir().join(format!("ikari-engine-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (v, text) in
            ikari_textfile::testing::read_variants("engine.lm\toff\nengine.idle_unload_secs\t60\n")
        {
            std::fs::write(dir.join(SETTINGS_FILE_NAME), text).unwrap();
            let s = load(&dir);
            assert_eq!(s.lm, Some(false), "{v:?}");
            assert_eq!(s.idle_unload_secs, Some(60), "{v:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_pack_switch() {
        assert_eq!(parse("dict.pack_it\toff\n").pack_it, Some(false));
        assert_eq!(parse("dict.pack_it\ton\n").pack_it, Some(true));
        assert_eq!(parse("dict.pack_it\tmaybe\n").pack_it, None);
        assert_eq!(parse("").pack_it, None);
    }

    #[test]
    fn postal_keys_have_defaults_and_read_both_values() {
        let none = parse("");
        assert_eq!(none.postal_japanese, None);
        assert!(none.postal_japanese_or_default());
        assert!(!none.postal_english_or_default());
        assert!(!none.postal_te_prefix_or_default());
        let s = parse("postal.japanese\toff\npostal.english\ton\npostal.te_prefix\ton\n");
        assert_eq!(s.postal_japanese, Some(false));
        assert_eq!(s.postal_english, Some(true));
        assert_eq!(s.postal_te_prefix, Some(true));
        assert!(!s.postal_japanese_or_default());
        assert!(s.postal_english_or_default() && s.postal_te_prefix_or_default());
        // 不正な値は書かれていないものとして扱う（既定に戻る）
        let bad = parse("postal.japanese\tmaybe\npostal.te_prefix\t1\n");
        assert_eq!(bad, Settings::default());
    }

    #[test]
    fn unicode_notation_defaults_off_and_reads_both_values() {
        assert!(!parse("").unicode_notation_or_default());
        assert!(parse("unicode.notation\ton\n").unicode_notation_or_default());
        assert_eq!(
            parse("unicode.notation\toff\n").unicode_notation,
            Some(false)
        );
        assert_eq!(parse("unicode.notation\t1\n"), Settings::default());
    }

    #[test]
    fn date_weekday_dates_defaults_off_and_reads_both_values() {
        assert!(!parse("").date_weekday_dates_or_default());
        assert!(parse("date.weekday_dates\ton\n").date_weekday_dates_or_default());
        assert_eq!(
            parse("date.weekday_dates\toff\n").date_weekday_dates,
            Some(false)
        );
        assert_eq!(parse("date.weekday_dates\t1\n"), Settings::default());
    }

    #[test]
    fn lm_gpu_defaults_on_and_reads_both_values() {
        assert!(parse("").lm_gpu_or_default());
        assert!(!parse("engine.lm_gpu\toff\n").lm_gpu_or_default());
        assert!(parse("engine.lm_gpu\ton\n").lm_gpu_or_default());
        assert_eq!(parse("engine.lm_gpu\toff\n").lm_gpu, Some(false));
        // 書き方が違う値は無視して既定（真）
        assert_eq!(parse("engine.lm_gpu\t0\n"), Settings::default());
        assert!(parse("engine.lm_gpu\t0\n").lm_gpu_or_default());
    }

    #[test]
    fn bad_values_and_unknown_keys_are_ignored() {
        let s = parse("engine.lm\tmaybe\nengine.idle_unload_secs\t-1\nx\ty\nno tab line\n");
        assert_eq!(s, Settings::default());
        assert_eq!(
            parse("engine.idle_unload_secs\t0\n").idle_unload_secs,
            Some(0)
        );
        assert_eq!(parse("engine.lm\ton\r\n").lm, Some(true));
    }

    #[test]
    fn timeout_keys_default_to_the_current_constants() {
        let none = parse("");
        assert_eq!(none.lm_deadline_ms, None);
        assert_eq!(none.lm_deadline_or_default(), Duration::from_millis(300));
        assert_eq!(none.convert_wait_or_default(), Duration::from_millis(30));
    }

    #[test]
    fn timeout_keys_are_read_and_clamped() {
        let s = parse("engine.lm_deadline_ms\t500\nengine.convert_wait_ms\t0\n");
        assert_eq!(s.lm_deadline_or_default(), Duration::from_millis(500));
        assert_eq!(s.convert_wait_or_default(), Duration::ZERO);
        let low = parse("engine.lm_deadline_ms\t1\n");
        assert_eq!(low.lm_deadline_or_default(), Duration::from_millis(50));
        let high = parse("engine.lm_deadline_ms\t99999\nengine.convert_wait_ms\t99999\n");
        assert_eq!(high.lm_deadline_or_default(), Duration::from_millis(2000));
        assert_eq!(high.convert_wait_or_default(), Duration::from_millis(200));
    }

    #[test]
    fn bad_timeout_values_fall_back_to_the_default() {
        let s = parse("engine.lm_deadline_ms\t-1\nengine.convert_wait_ms\tabc\n");
        assert_eq!(s, Settings::default());
        assert_eq!(s.lm_deadline_or_default(), Duration::from_millis(300));
        assert_eq!(s.convert_wait_or_default(), Duration::from_millis(30));
    }

    #[test]
    fn missing_file_is_the_default() {
        assert_eq!(load(Path::new("/nonexistent/ikari")), Settings::default());
    }
}
