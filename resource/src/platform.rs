//! この OS の製品で、自由指定のモデルを出すか（エンジンと画面の出し分けが同じ値を読む）。
//!
//! 「取っても読めないものを出さない」ので、エンジンが読まない構成では出さない側に倒す。

/// macOS の製品で、自由指定のモデルを出すか。檻役の実装がマージされ、実機の確認が済むまでは出さない
/// （既定の判断はリードの確認待ち）
pub const MACOS_USER_MODELS_RELEASED: bool = false;

/// この OS の製品で、自由指定のモデルを出すか（コンパイル時の値）。
/// `cage_only`: 檻役だけを使う構成か（macOS の製品）。Windows は段 2 があるので出す。ほかは段 2 が無いので出さない
pub const fn user_models_offered_on_platform(cage_only: bool) -> bool {
    if cfg!(windows) {
        true
    } else if cage_only {
        MACOS_USER_MODELS_RELEASED
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 檻役だけの構成は、解放の値に従う。段 2 の無い構成（Windows 以外）は出さない
    #[test]
    fn follows_release_flag_and_defaults_to_hidden() {
        if cfg!(windows) {
            assert!(user_models_offered_on_platform(false));
            assert!(user_models_offered_on_platform(true));
        } else {
            assert!(!user_models_offered_on_platform(false));
            assert_eq!(
                user_models_offered_on_platform(true),
                MACOS_USER_MODELS_RELEASED
            );
        }
    }
}
