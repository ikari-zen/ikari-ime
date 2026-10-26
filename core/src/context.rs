//! 変換時の文脈（ADR 0001 §4.3, ADR 0002 §9.1）。

use std::sync::Arc;

use crate::user::Profile;

#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// カーソルより前の文脈（周辺テキストと確定履歴を合わせたもの）。末尾がカーソルの直前
    pub left_text: String,
    /// `left_text` が、ほかの入力欄（ほかのアプリ）から借りたものなら true（ADR 0015 D5、D8）。既定 false。
    ///
    /// 契約: true のとき `LearningRanker` は、学習の文脈の特徴量（Prev、PrevContent、Bag、Topic）を
    /// 空にする。引き当て（候補の順位づけ）も、確定のときの記録も、「文脈なし」として扱う
    /// （ADR 0013 の「記録のない文脈」）。話題、LM、連接、文書内キャッシュは、借りた左文脈を
    /// ふつうの左文脈と同じに使う。
    pub left_borrowed: bool,
    /// 補助（フェーズ0では使わない）
    pub right_text: String,
    /// 同じ入力欄・同じセッションの確定履歴（古い順）
    pub history: Vec<Commit>,
    /// パスワード欄。true なら Converter が文脈を空にしてから Ranker に渡す（NFR-18）
    pub secure: bool,
    /// 属性プロファイル。アプリごとの切り替えや eval のケースごとの指定もこの口で扱う
    pub profile: Option<Arc<Profile>>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_left_text(mut self, s: impl Into<String>) -> Self {
        self.left_text = s.into();
        self
    }

    pub fn with_right_text(mut self, s: impl Into<String>) -> Self {
        self.right_text = s.into();
        self
    }

    pub fn with_history(mut self, h: Vec<Commit>) -> Self {
        self.history = h;
        self
    }

    pub fn with_profile(mut self, profile: Option<Arc<Profile>>) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_left_borrowed(mut self, borrowed: bool) -> Self {
        self.left_borrowed = borrowed;
        self
    }

    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub reading: String,
    pub surface: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_is_not_borrowed_by_default() {
        assert!(!Context::new().left_borrowed);
        assert!(Context::new().with_left_borrowed(true).left_borrowed);
    }
}
