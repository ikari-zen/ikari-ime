//! 版（通常版・オフライン版）と、画面に出してよい機能（ADR 0080 §7.2。契約。レーン 0）。
//!
//! 判断の置き場所: Rust（macOS と Windows で 1 つ）。Swift と C# は [`edition_capabilities`] の値で、
//! ボタンを出す・出さないを決めるだけ。
//!
//! 値の決め方（レーン 2 で埋める）: `cfg!(feature = "online")`、檻の段（自由指定は段 2 のある OS だけ。ADR 0072 Q1）、
//! `ikari-data` が同梱の場所にあるか。
//!
//! 今の中身はスタブ: 通常版の値（すべて出す）を返す。今の画面の挙動と同じ。

/// 版
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// 通常版（ボタンを押したときだけ、自由指定のモデルの取得と版の確認で通信する）
    Normal,
    /// オフライン版（通信のロジックを持たない。`ikari-data` も入れない）
    Offline,
}

/// 画面に出してよい機能（ADR 0080 §7.2）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// 版（「このソフトについて」に出す名前）
    pub edition: Edition,
    /// 「URL から取る」（自由指定のモデルの取得）
    pub url_model_fetch: bool,
    /// 「新しい版を確認」
    pub version_check: bool,
    /// 「ファイルから取り込む」（#246。両方の版で出す）
    pub file_import: bool,
}

/// このビルドの版と、出してよい機能。
///
/// スタブ: 通常版の値を返す（今の挙動）。
pub fn edition_capabilities() -> Capabilities {
    Capabilities {
        edition: Edition::Normal,
        url_model_fetch: true,
        version_check: true,
        file_import: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// スタブの挙動は今と同じ（通常版。取得と版の確認のボタンを出す）
    #[test]
    fn stub_is_the_normal_edition() {
        assert_eq!(
            edition_capabilities(),
            Capabilities {
                edition: Edition::Normal,
                url_model_fetch: true,
                version_check: true,
                file_import: true,
            }
        );
    }

    /// 殻のスレッドをまたいで渡せる（値の型）
    #[test]
    fn capabilities_are_plain_values() {
        fn assert_traits<T: Copy + Send + Sync + 'static>() {}
        assert_traits::<Capabilities>();
        assert_traits::<Edition>();
    }
}
