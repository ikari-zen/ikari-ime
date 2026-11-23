//! この OS の製品で、自由指定のモデルを出すか（エンジンと画面の出し分けが同じ値を読む）。
//!
//! 「取っても読めないものを出さない」ので、エンジンが読まない構成では出さない側に倒す。

/// macOS の製品で、自由指定のモデルを出すか。檻役の実装がマージされ、実機の確認が済むまでは出さない
/// （既定の判断はリードの確認待ち）
pub const MACOS_USER_MODELS_RELEASED: bool = false;

/// この OS の製品で、自由指定のモデルを出すか（コンパイル時の値。エンジンと画面の出し分けは、この 1 か所を読む）。
/// Windows は段 2 があるので出す。macOS（Apple の OS）は檻役の解放（[`MACOS_USER_MODELS_RELEASED`]）に従う。
/// ほかは段 2 が無いので出さない。開発用のビルド（テスト、`dev-knobs`）でも値は変えない（画面と食い違わせない）
pub const USER_MODELS_OFFERED: bool = if cfg!(windows) {
    true
} else if cfg!(target_vendor = "apple") {
    MACOS_USER_MODELS_RELEASED
} else {
    false
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows は出す。macOS は解放の値に従う（既定は出さない）。ほかは出さない
    #[test]
    fn follows_release_flag_and_defaults_to_hidden() {
        const {
            assert!(
                USER_MODELS_OFFERED
                    == (cfg!(windows)
                        || (cfg!(target_vendor = "apple") && MACOS_USER_MODELS_RELEASED))
            )
        };
        if !cfg!(windows) {
            // 解放前の macOS と、段 2 の無い OS は出さない
            const {
                assert!(
                    USER_MODELS_OFFERED == MACOS_USER_MODELS_RELEASED
                        || !cfg!(target_vendor = "apple")
                )
            };
        }
    }
}
