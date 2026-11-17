//! アプリの版の確認の形と比べ方（ADR 0076 §6、§18.2）。**契約（第 4 弾）。中身はレーン D-2d**。
//!
//! `ikari-data version-check`（読むのはタグ）と、設定画面の核（Windows の `windows/settings`、macOS の `shell-ffi`。
//! 比べて表示を選ぶ）が共有する。OS に依存しない。
//!
//! - 受けるタグは `v<数>.<数>.<数>` だけ。ほかの形（データのタグ、`v` のない形、`-rc` などの付いた形）は受けない
//!   （`bad-reply`。確認できない側に倒す）。
//! - 各数字は [`MAX_VERSION_PART_DIGITS`] 桁まで（比べるときの桁あふれを避ける）。
//! - 表示は 3 つだけ（[`VersionVerdict`]）。取った版が今より古くても、古い版を勧めない。

/// 契約のレーンの仮置き。実装のレーンで計測して PR に書き、ユーザーに確かめる。
/// 版の各数字の桁の上限（ADR 0076 §18.2。今の `fetch_report::is_version` の値と同じ 9 にした。u32 に収まる）
pub const MAX_VERSION_PART_DIGITS: usize = 9;

/// 版の確認で受けるタグの接頭辞
pub const TAG_PREFIX: char = 'v';

/// アプリの版（`数.数.数`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AppVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl AppVersion {
    /// `数.数.数`（`fetch_report` の `latest` の行の形。`CARGO_PKG_VERSION` もこの形）を読む。
    /// 契約のスタブ: 今は常に None（D-2d）
    pub fn parse(text: &str) -> Option<AppVersion> {
        let _ = text;
        None
    }

    /// タグ `v数.数.数` を読む。ほかの形は None。契約のスタブ: 今は常に None（D-2d）
    pub fn parse_tag(tag: &str) -> Option<AppVersion> {
        let _ = tag;
        None
    }
}

/// 設定画面に出す 3 つ（ADR 0076 §6）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionVerdict {
    /// 新しい版があります（その版）
    Newer(AppVersion),
    /// 最新です
    UpToDate,
    /// この版のほうが新しい（開発版）。取った版が古くても、古い版は勧めない
    Ahead,
}

/// 自分の版（`CARGO_PKG_VERSION`）と、取った版を比べる。自分の版が読めなければ None（表示は「確かめられない」）。
/// 契約のスタブ: 今は常に None（D-2d）
pub fn verdict(current: &str, latest: AppVersion) -> Option<VersionVerdict> {
    let _ = (current, latest);
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "D-2d: v数.数.数 だけを受け、v なし・-rc つき・データのタグ（webwords-…）・4 つの数は受けない"]
    fn tags_other_than_v_major_minor_patch_are_refused() {
        unimplemented!("D-2d")
    }

    #[test]
    #[ignore = "D-2d: MAX_VERSION_PART_DIGITS を超える桁の数は受けない（桁あふれしない）"]
    fn too_many_digits_are_refused() {
        unimplemented!("D-2d")
    }

    #[test]
    #[ignore = "D-2d: 新しい・同じ・古い の 3 つ。古い版は Ahead で、勧めない"]
    fn verdicts_never_recommend_an_older_version() {
        unimplemented!("D-2d")
    }
}
