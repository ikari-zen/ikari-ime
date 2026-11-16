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
    /// 各数字は 1 桁以上 [`MAX_VERSION_PART_DIGITS`] 桁以下の 10 進数字だけ（符号、空白、`+` は受けない）。
    pub fn parse(text: &str) -> Option<AppVersion> {
        let mut parts = text.split('.');
        let major = parse_part(parts.next()?)?;
        let minor = parse_part(parts.next()?)?;
        let patch = parse_part(parts.next()?)?;
        // 4 つ目以降は受けない
        parts.next().is_none().then_some(AppVersion {
            major,
            minor,
            patch,
        })
    }

    /// タグ `v数.数.数` を読む。ほかの形（`v` なし、`-rc` つき、データのタグ）は None。
    pub fn parse_tag(tag: &str) -> Option<AppVersion> {
        AppVersion::parse(tag.strip_prefix(TAG_PREFIX)?)
    }
}

impl std::fmt::Display for AppVersion {
    /// `数.数.数`（`parse` が読み戻せる形）
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_part(part: &str) -> Option<u32> {
    if part.is_empty()
        || part.len() > MAX_VERSION_PART_DIGITS
        || !part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    part.parse().ok()
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
pub fn verdict(current: &str, latest: AppVersion) -> Option<VersionVerdict> {
    let current = AppVersion::parse(current)?;
    Some(match latest.cmp(&current) {
        std::cmp::Ordering::Greater => VersionVerdict::Newer(latest),
        std::cmp::Ordering::Equal => VersionVerdict::UpToDate,
        // 取った版が古くても、古い版は勧めない
        std::cmp::Ordering::Less => VersionVerdict::Ahead,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, patch: u32) -> AppVersion {
        AppVersion {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn tags_other_than_v_major_minor_patch_are_refused() {
        assert_eq!(AppVersion::parse_tag("v0.3.1"), Some(v(0, 3, 1)));
        assert_eq!(AppVersion::parse_tag("v10.20.30"), Some(v(10, 20, 30)));
        for bad in [
            "",
            "v",
            "0.3.1",
            "V0.3.1",
            "v0.3",
            "v0.3.1.4",
            "v0.3.1-rc1",
            "v0.3.1+x",
            "v0..1",
            "v.3.1",
            "v0.3.",
            "v-1.3.1",
            "v+1.3.1",
            "v 1.3.1",
            "v0.3.1 ",
            "v0.3.1\n",
            "v١.٣.١",
            "webwords-20261101",
        ] {
            assert_eq!(AppVersion::parse_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        let version = v(0, 3, 1);
        assert_eq!(version.to_string(), "0.3.1");
        assert_eq!(AppVersion::parse(&version.to_string()), Some(version));
    }

    #[test]
    fn parse_reads_the_cargo_version_form() {
        assert_eq!(AppVersion::parse("0.3.1"), Some(v(0, 3, 1)));
        assert_eq!(AppVersion::parse("v0.3.1"), None);
        assert!(AppVersion::parse(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn too_many_digits_are_refused() {
        let max = "9".repeat(MAX_VERSION_PART_DIGITS);
        let over = "9".repeat(MAX_VERSION_PART_DIGITS + 1);
        assert!(AppVersion::parse_tag(&format!("v{max}.{max}.{max}")).is_some());
        for bad in [
            format!("v{over}.0.0"),
            format!("v0.{over}.0"),
            format!("v0.0.{over}"),
            format!("v{}.0.0", "0".repeat(40)),
        ] {
            assert_eq!(AppVersion::parse_tag(&bad), None, "{bad}");
        }
    }

    #[test]
    fn verdicts_never_recommend_an_older_version() {
        assert_eq!(
            verdict("0.3.1", v(0, 3, 2)),
            Some(VersionVerdict::Newer(v(0, 3, 2)))
        );
        assert_eq!(
            verdict("0.3.1", v(1, 0, 0)),
            Some(VersionVerdict::Newer(v(1, 0, 0)))
        );
        assert_eq!(verdict("0.3.1", v(0, 3, 1)), Some(VersionVerdict::UpToDate));
        assert_eq!(verdict("0.3.1", v(0, 3, 0)), Some(VersionVerdict::Ahead));
        assert_eq!(verdict("0.10.0", v(0, 9, 9)), Some(VersionVerdict::Ahead));
        // 自分の版が読めなければ、確かめられない
        assert_eq!(verdict("dev", v(0, 3, 2)), None);
        assert_eq!(verdict("0.3.1-rc1", v(0, 3, 2)), None);
    }
}
