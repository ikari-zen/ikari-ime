//! パイプの相手が本物のエンジンかの判定（ADR 0040 §2.7、ADR 0063 §3 の 5。#409）。**OS に依存しない**（Mac でテストする）。
//!
//! TIP は読み・左文脈・確定文をパイプに送るので、つないだ相手のプロセスの実行ファイルが、インストール先の
//! `ikari-engine-host.exe` であることを確かめてから使う。OS の呼び出し（相手の PID → 実行ファイルのパス、
//! DLL 自身のパス、8.3 の短い名前の展開）は COM の層（`com::connector`）が行い、ここは**パス 2 つ**を受けて
//! 許すか拒むかだけを決める。
//!
//! # 規則（ここ 1 か所に置く）
//! - 期待するパスは、TIP の DLL の場所から決める（`ENGINE_LAYOUTS`）:
//!   - 版ごとの配置（ADR 0063 §3 の 1）: `<インストール先>\tip\<版>\ikari_tip.dll` → `<インストール先>\ikari-engine-host.exe`
//!   - 平らな配置（開発用の導入）: `<フォルダ>\ikari_tip.dll` → `<フォルダ>\ikari-engine-host.exe`
//! - 比べるのは、ドライブ文字で始まる絶対パス（`C:\…`）どうし。`\\?\` の接頭辞は外し、末尾や重なった区切りは
//!   無視する。`.` と `..` を含むもの、相対パス、UNC（ネットワーク）、デバイスのパス、`/` を含むものは拒む
//!   （OS が返すパスはこれらを含まない。含むなら何かがおかしいので、通さない）。
//! - 大文字と小文字は **ASCII だけ**区別しない。ASCII 以外は完全に一致したときだけ通す（Unicode の大文字化は
//!   NTFS の表と一致しないので、別のファイルを同じと見なすおそれがある。違えば拒むだけで、安全な側に倒れる）。
//! - シンボリックリンクとジャンクションは**たどらない**（文字列として比べる）。相手のパスは OS が記録した
//!   起動時のパスをそのまま使い、あとからファイルシステムで解決し直さない（解決し直すと、起動のあとで
//!   リンクを差し替えて期待のパスに見せられる）。
//!
//! # 限界
//! この確認は「インストール先に書き込めるのは管理者だけ」であることを前提にする（本番は Program Files。
//! ADR 0063 Q2）。開発用の導入のように利用者が書ける場所に置いた場合は、同じ利用者が書ける場所を信頼する
//! だけになり、強い保証にはならない。

/// エンジンの実行ファイルの名前（`windows/dev/common.ps1` の `$EngineExeName`。パッケージ名のまま）
pub const ENGINE_EXE_NAME: &str = "ikari-engine-host.exe";
/// 版ごとのフォルダを並べるフォルダの名前（`<インストール先>\tip\<版>\`。ADR 0063 §3 の 1）
pub const TIP_DIR_NAME: &str = "tip";
/// 長いパスの接頭辞（`\\?\C:\…`）
const VERBATIM_PREFIX: &str = r"\\?\";
/// 長いパスのネットワークの接頭辞（`\\?\UNC\server\share\…`）。外したあとに残る形
const VERBATIM_UNC_PREFIX: &str = r"UNC\";
const SEPARATOR: char = '\\';
/// Windows では区切りとして読まれるが、OS が返すパスには現れないので拒む
const ALT_SEPARATOR: char = '/';
const DRIVE_SEPARATOR: char = ':';

/// エンジンの置き場所の形（TIP の DLL の場所から、期待するエンジンのパスを決める）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineLayout {
    /// `<インストール先>\tip\<版>\<DLL>` → `<インストール先>\ikari-engine-host.exe`
    Versioned,
    /// `<フォルダ>\<DLL>` → `<フォルダ>\ikari-engine-host.exe`
    Flat,
}

/// 許す配置。**照合の規則はここ 1 か所**（平らな配置をやめるときは、ここから外す）。
/// 平らな配置は開発用の導入（`windows/dev/register.ps1`）のため。本番の配置では DLL のフォルダ
/// （`<インストール先>\tip\<版>\`）も管理者だけが書ける場所なので、許しても弱くならない。
pub const ENGINE_LAYOUTS: &[EngineLayout] = &[EngineLayout::Versioned, EngineLayout::Flat];

/// 相手を拒んだ理由（診断のログに出す。パスや利用者名は含めない）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerRejection {
    /// 相手のプロセスの番号が取れない
    NoPeerPid,
    /// 相手の実行ファイルのパスが取れない（プロセスを開けない、終わった、など）
    NoPeerPath,
    /// TIP の DLL 自身のパスが取れない
    NoTipPath,
    /// TIP の DLL のパスが、比べられる形（ドライブで始まる正規化済みの絶対パス）でない
    TipPathInvalid,
    /// 相手のパスが、比べられる形でない
    PeerPathInvalid,
    /// 相手が期待するエンジンのパスでない
    NotTheEngine,
}

impl PeerRejection {
    /// ログに出す固定の文
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPeerPid => "no-peer-pid",
            Self::NoPeerPath => "no-peer-path",
            Self::NoTipPath => "no-tip-path",
            Self::TipPathInvalid => "tip-path-invalid",
            Self::PeerPathInvalid => "peer-path-invalid",
            Self::NotTheEngine => "not-the-engine",
        }
    }
}

/// 比べられる形にしたパス（ドライブ文字と、その下の名前の並び）
#[derive(Debug)]
struct NormalPath<'a> {
    /// `C:` など（ドライブ文字は ASCII の英字）
    drive: &'a str,
    components: Vec<&'a str>,
}

impl NormalPath<'_> {
    fn same_as(&self, drive: &str, components: &[&str]) -> bool {
        self.drive.eq_ignore_ascii_case(drive)
            && self.components.len() == components.len()
            && self
                .components
                .iter()
                .zip(components)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
}

/// `path` を比べられる形にする。形が合わなければ `None`（拒む）。
fn normalize(path: &str) -> Option<NormalPath<'_>> {
    if path.contains(ALT_SEPARATOR) {
        return None;
    }
    let rest = path.strip_prefix(VERBATIM_PREFIX).unwrap_or(path);
    if rest.starts_with(VERBATIM_UNC_PREFIX) || rest.starts_with(SEPARATOR) {
        return None;
    }
    // `C:\` で始まること（`C:foo` はドライブの今のフォルダからの相対パス）
    let mut chars = rest.chars();
    let (Some(letter), Some(DRIVE_SEPARATOR), Some(SEPARATOR)) =
        (chars.next(), chars.next(), chars.next())
    else {
        return None;
    };
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let (drive, below) = rest.split_at(2);
    let components: Vec<&str> = below.split(SEPARATOR).filter(|c| !c.is_empty()).collect();
    if components.iter().any(|c| *c == "." || *c == "..") {
        return None;
    }
    Some(NormalPath { drive, components })
}

/// `layout` のとき、DLL のパスから期待するエンジンのパス（ドライブの下の名前の並び）。配置の形に合わなければ `None`。
fn expected_engine<'a>(tip_dll: &NormalPath<'a>, layout: EngineLayout) -> Option<Vec<&'a str>> {
    let dll = &tip_dll.components;
    // 最後は DLL の名前。その上のフォルダ
    let dll_dir = dll.split_last()?.1;
    let base = match layout {
        EngineLayout::Flat => dll_dir,
        EngineLayout::Versioned => {
            // `…\tip\<版>` の 2 つ
            let (_version, above) = dll_dir.split_last()?;
            let (tip, install) = above.split_last()?;
            if !tip.eq_ignore_ascii_case(TIP_DIR_NAME) {
                return None;
            }
            install
        }
    };
    let mut expected = base.to_vec();
    expected.push(ENGINE_EXE_NAME);
    Some(expected)
}

/// つないだ相手（実行ファイルのパス `peer_exe`）が、TIP の DLL（`tip_dll`）から決まるエンジンかを判定する。
///
/// どちらも OS が返したフルパス（8.3 の短い名前は呼ぶ側が展開しておく）。`ENGINE_LAYOUTS` のどれかに一致すれば許す。
pub fn verify_engine_path(tip_dll: &str, peer_exe: &str) -> Result<(), PeerRejection> {
    let tip = normalize(tip_dll).ok_or(PeerRejection::TipPathInvalid)?;
    let peer = normalize(peer_exe).ok_or(PeerRejection::PeerPathInvalid)?;
    let matches = ENGINE_LAYOUTS.iter().any(|&layout| {
        expected_engine(&tip, layout).is_some_and(|expected| peer.same_as(tip.drive, &expected))
    });
    if matches {
        Ok(())
    } else {
        Err(PeerRejection::NotTheEngine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSIONED_DLL: &str = r"C:\Program Files\ikari-ime\tip\0.3.1\ikari_tip.dll";
    const VERSIONED_ENGINE: &str = r"C:\Program Files\ikari-ime\ikari-engine-host.exe";
    const FLAT_DLL: &str = r"C:\Users\example\AppData\Local\ikari-ime-dev\tip\ikari_tip.dll";
    const FLAT_ENGINE: &str =
        r"C:\Users\example\AppData\Local\ikari-ime-dev\tip\ikari-engine-host.exe";

    const OK: Result<(), PeerRejection> = Ok(());
    const NOT_ENGINE: Result<(), PeerRejection> = Err(PeerRejection::NotTheEngine);
    const BAD_PEER: Result<(), PeerRejection> = Err(PeerRejection::PeerPathInvalid);
    const BAD_TIP: Result<(), PeerRejection> = Err(PeerRejection::TipPathInvalid);

    #[test]
    fn table() {
        let cases: &[(&str, &str, &str, Result<(), PeerRejection>)] = &[
            // 一致
            ("versioned", VERSIONED_DLL, VERSIONED_ENGINE, OK),
            ("flat", FLAT_DLL, FLAT_ENGINE, OK),
            (
                "versioned under the dev tip folder",
                r"C:\Users\example\AppData\Local\ikari-ime-dev\tip\20261115-1200\ikari_tip.dll",
                r"C:\Users\example\AppData\Local\ikari-ime-dev\ikari-engine-host.exe",
                OK,
            ),
            (
                "install at the drive root",
                r"D:\tip\1\ikari_tip.dll",
                r"D:\ikari-engine-host.exe",
                OK,
            ),
            // 大文字と小文字
            (
                "ascii case",
                VERSIONED_DLL,
                r"c:\PROGRAM FILES\Ikari-IME\IKARI-ENGINE-HOST.EXE",
                OK,
            ),
            (
                "tip folder case",
                r"C:\Program Files\ikari-ime\TIP\0.3.1\ikari_tip.dll",
                VERSIONED_ENGINE,
                OK,
            ),
            (
                "non-ascii must match exactly",
                r"C:\Users\Ä\ikari\tip\1\ikari_tip.dll",
                r"C:\Users\ä\ikari\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "non-ascii exact",
                r"C:\Users\太郎\ikari\tip\1\ikari_tip.dll",
                r"C:\Users\太郎\ikari\ikari-engine-host.exe",
                OK,
            ),
            (
                "no unicode folding (dotless i)",
                VERSIONED_DLL,
                r"C:\Program Fıles\ikari-ime\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            // 末尾と重なった区切り
            (
                "trailing separator",
                VERSIONED_DLL,
                &format!(r"{VERSIONED_ENGINE}\"),
                OK,
            ),
            (
                "doubled separator",
                VERSIONED_DLL,
                r"C:\Program Files\\ikari-ime\ikari-engine-host.exe",
                OK,
            ),
            // `\\?\`
            (
                "verbatim peer",
                VERSIONED_DLL,
                &format!(r"\\?\{VERSIONED_ENGINE}"),
                OK,
            ),
            (
                "verbatim tip",
                &format!(r"\\?\{VERSIONED_DLL}"),
                VERSIONED_ENGINE,
                OK,
            ),
            (
                "verbatim unc",
                VERSIONED_DLL,
                r"\\?\UNC\server\share\ikari-engine-host.exe",
                BAD_PEER,
            ),
            // `..` と `.`
            (
                "dotdot in peer",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\tip\..\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "dotdot reaching the expected path",
                VERSIONED_DLL,
                r"C:\Users\example\..\..\Program Files\ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "dot in peer",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\.\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "dotdot in tip",
                r"C:\Users\example\..\..\Program Files\ikari-ime\tip\1\ikari_tip.dll",
                VERSIONED_ENGINE,
                BAD_TIP,
            ),
            // 似た名前
            (
                "suffix",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe.evil",
                NOT_ENGINE,
            ),
            (
                "similar name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host2.exe",
                NOT_ENGINE,
            ),
            (
                "hidden build name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host-hidden.exe",
                NOT_ENGINE,
            ),
            (
                "alternate data stream",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe:evil",
                NOT_ENGINE,
            ),
            (
                "trailing dot",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe.",
                NOT_ENGINE,
            ),
            // 別のフォルダの同名
            (
                "other folder",
                VERSIONED_DLL,
                r"C:\Users\example\Downloads\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "other drive",
                VERSIONED_DLL,
                r"D:\Program Files\ikari-ime\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "prefix folder name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime-evil\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "inside the version folder of a versioned dll is flat",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\tip\0.3.1\ikari-engine-host.exe",
                OK,
            ),
            (
                "inside the tip folder",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\tip\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "versioned rule needs the tip folder",
                r"C:\Users\example\Downloads\x\1\ikari_tip.dll",
                r"C:\Users\example\Downloads\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            (
                "versioned needs two folders above",
                r"C:\tip\ikari_tip.dll",
                r"C:\ikari-engine-host.exe",
                NOT_ENGINE,
            ),
            // 空
            ("empty peer", VERSIONED_DLL, "", BAD_PEER),
            ("empty tip", "", VERSIONED_ENGINE, BAD_TIP),
            ("drive only", VERSIONED_DLL, r"C:\", NOT_ENGINE),
            // 相対パスなど、比べられない形
            (
                "relative",
                VERSIONED_DLL,
                r"ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "drive relative",
                VERSIONED_DLL,
                r"C:Program Files\ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "rooted without drive",
                VERSIONED_DLL,
                r"\Program Files\ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "unc",
                VERSIONED_DLL,
                r"\\server\share\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "device path",
                VERSIONED_DLL,
                r"\\.\C:\Program Files\ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "forward slashes",
                VERSIONED_DLL,
                "C:/Program Files/ikari-ime/ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "non-letter drive",
                VERSIONED_DLL,
                r"1:\Program Files\ikari-ime\ikari-engine-host.exe",
                BAD_PEER,
            ),
            (
                "relative tip",
                r"tip\1\ikari_tip.dll",
                VERSIONED_ENGINE,
                BAD_TIP,
            ),
        ];
        for (name, tip, peer, expected) in cases {
            assert_eq!(verify_engine_path(tip, peer), *expected, "{name}");
        }
    }

    #[test]
    fn rejection_texts_are_distinct() {
        let all = [
            PeerRejection::NoPeerPid,
            PeerRejection::NoPeerPath,
            PeerRejection::NoTipPath,
            PeerRejection::TipPathInvalid,
            PeerRejection::PeerPathInvalid,
            PeerRejection::NotTheEngine,
        ];
        let texts: std::collections::HashSet<_> = all.iter().map(|r| r.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
