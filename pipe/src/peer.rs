//! パイプの相手が、インストール先の決まった実行ファイルかの判定（ADR 0068 §2 の 2。ADR 0040 §2.7、ADR 0063 §3 の 5）。
//! **OS に依存しない**（Mac でテストする）。
//!
//! 3 つの口（エンジン、設定、候補窓）で同じ規則を使うので、ここ 1 か所に置く。置き場所を `pipe` にしたのは、
//! 照合する側（TIP、engine-host、設定画面）と候補窓の核のどれもが `pipe` に依存し、`pipe` はほかに依存しない
//! （依存の向きを増やさない。`shell-core` は TIP と engine-host の判断の層で、口の相手の身元は持たない）。
//!
//! OS の呼び出し（相手の PID → 実行ファイルのパス、自分のモジュールのパス、8.3 の短い名前の展開）は各 OS 層が
//! 行い、ここは**パス 2 つ**と「だれが・どの口の相手を」照らすかを受けて、許すか拒むかだけを決める。
//!
//! # 規則
//! - インストール先は、照らす側の自分のモジュールのパスから決める（`SelfModule`。環境変数などから決めない。R2）:
//!   - TIP: 版ごとの配置 `<インストール先>\tip\<版>\ikari_tip.dll`、または平らな配置（開発用の導入）
//!     `<フォルダ>\ikari_tip.dll`（`TIP_LAYOUTS`）
//!   - engine-host: `<インストール先>\ikari-engine-host.exe`
//!   - 設定画面: `<インストール先>\settings\<exe>`
//! - 相手の口ごとに、インストール先の下の決まったパスと比べる（`Endpoint`。ADR 0067 §3.1 の配置）。
//! - 比べるのは、ドライブ文字で始まる絶対パス（`C:\…`）どうし。`\\?\` の接頭辞は外し、末尾や重なった区切りは
//!   無視する。`.` と `..` を含むもの、相対パス、UNC（ネットワーク）、デバイスのパス、`/` を含むものは拒む。
//! - 大文字と小文字は **ASCII だけ**区別しない。ASCII 以外は完全に一致したときだけ通す。
//! - シンボリックリンクとジャンクションは**たどらない**（文字列として比べる）。相手のパスは OS が記録した
//!   起動時のパスをそのまま使い、あとからファイルシステムで解決し直さない。
//!
//! # 前提
//! この確認は「インストール先に書き込めるのは管理者だけ」であることを前提にする（ADR 0067 §3）。
//! 署名が付いたら（NFR-42）、この 1 か所を署名の照合に替える（ADR 0068 §2 の 2）。

/// エンジンの実行ファイルの名前（`windows/dev/common.ps1` の `$EngineExeName`。パッケージ名のまま）
pub const ENGINE_EXE_NAME: &str = "ikari-engine-host.exe";
/// 版ごとのフォルダを並べるフォルダの名前（`<インストール先>\tip\<版>\`。ADR 0063 §3 の 1）
pub const TIP_DIR_NAME: &str = "tip";
/// 候補窓の一式を置くフォルダの名前（`<インストール先>\candidate\`。ADR 0067 §3.1）
pub const CANDIDATE_DIR_NAME: &str = "candidate";
/// 候補窓の実行ファイルの名前（ADR 0062 §3 の 3）
pub const CANDIDATE_EXE_NAME: &str = "ikari-candidate.exe";
/// 設定画面の一式を置くフォルダの名前（`<インストール先>\settings\`。ADR 0067 §3.1）
pub const SETTINGS_DIR_NAME: &str = "settings";
/// 設定画面の実行ファイルの名前。
///
/// TODO(ADR 0071 §6): 設定画面の方式（WinUI 3 か Slint）を確かめたあとに決める。決まるまで `None` で、
/// 設定画面を相手とする照合は常に拒む（安全な側）。
pub const SETTINGS_EXE_NAME: Option<&str> = None;

/// 長いパスの接頭辞（`\\?\C:\…`）
const VERBATIM_PREFIX: &str = r"\\?\";
/// 長いパスのネットワークの接頭辞（`\\?\UNC\server\share\…`）。外したあとに残る形
const VERBATIM_UNC_PREFIX: &str = r"UNC\";
const SEPARATOR: char = '\\';
/// Windows では区切りとして読まれるが、OS が返すパスには現れないので拒む
const ALT_SEPARATOR: char = '/';
const DRIVE_SEPARATOR: char = ':';

/// 照らす側（自分のモジュールのパスから、インストール先を決める）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfModule {
    /// TIP の DLL（`TIP_LAYOUTS` のどれか）
    Tip,
    /// `<インストール先>\ikari-engine-host.exe`
    EngineHost,
    /// `<インストール先>\settings\<exe>`
    Settings,
}

/// 照らす相手（どの口の相手か。インストール先の下の決まった実行ファイル）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// エンジンの口と設定の口の受け手: `<インストール先>\ikari-engine-host.exe`
    Engine,
    /// 設定の口のつなぐ側: `<インストール先>\settings\<SETTINGS_EXE_NAME>`
    Settings,
    /// 候補窓の口の受け手: `<インストール先>\candidate\ikari-candidate.exe`
    Candidate,
}

/// TIP の DLL の置き場所の形（TIP の DLL の場所から、インストール先を決める）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipLayout {
    /// `<インストール先>\tip\<版>\<DLL>`
    Versioned,
    /// `<フォルダ>\<DLL>`（開発用の導入）
    Flat,
}

/// TIP に許す配置。**照合の規則はここ 1 か所**（平らな配置をやめるときは、ここから外す。ADR 0067 §6）。
#[cfg(feature = "dev-knobs")]
pub const TIP_LAYOUTS: &[TipLayout] = &[TipLayout::Versioned, TipLayout::Flat];
/// 製品のビルドは版つきの配置だけ（平らな配置は feature `dev-knobs` のとき）
#[cfg(not(feature = "dev-knobs"))]
pub const TIP_LAYOUTS: &[TipLayout] = &[TipLayout::Versioned];

/// 相手を拒んだ理由（診断のログに出す。パスや利用者名は含めない）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerRejection {
    /// 相手のプロセスの番号が取れない
    NoPeerPid,
    /// 相手の実行ファイルのパスが取れない（プロセスを開けない、終わった、など）
    NoPeerPath,
    /// 自分のモジュールのパスが取れない
    NoSelfPath,
    /// 自分のモジュールのパスが、比べられる形でないか、`SelfModule` の配置に合わない
    SelfPathInvalid,
    /// 相手のパスが、比べられる形でない
    PeerPathInvalid,
    /// 相手が、その口に期待する実行ファイルでない
    NotThePeer,
}

impl PeerRejection {
    /// ログに出す固定の文
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPeerPid => "no-peer-pid",
            Self::NoPeerPath => "no-peer-path",
            Self::NoSelfPath => "no-self-path",
            Self::SelfPathInvalid => "self-path-invalid",
            Self::PeerPathInvalid => "peer-path-invalid",
            Self::NotThePeer => "not-the-peer",
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

/// `layout` のとき、TIP の DLL のパスから決まるインストール先（ドライブの下の名前の並び）。形に合わなければ `None`。
fn tip_install_root<'p, 'a>(dll: &'p [&'a str], layout: TipLayout) -> Option<&'p [&'a str]> {
    // 最後は DLL の名前。その上のフォルダ
    let dll_dir = dll.split_last()?.1;
    match layout {
        TipLayout::Flat => Some(dll_dir),
        TipLayout::Versioned => {
            // `…\tip\<版>` の 2 つ
            let (_version, above) = dll_dir.split_last()?;
            let (tip, install) = above.split_last()?;
            tip.eq_ignore_ascii_case(TIP_DIR_NAME).then_some(install)
        }
    }
}

/// 自分のモジュールのパスから決まるインストール先の候補（TIP は配置の形ごとに 1 つずつ）。
fn install_roots<'p, 'a>(module: SelfModule, own: &'p [&'a str]) -> Vec<&'p [&'a str]> {
    match module {
        SelfModule::Tip => TIP_LAYOUTS
            .iter()
            .filter_map(|&layout| tip_install_root(own, layout))
            .collect(),
        SelfModule::EngineHost => own
            .split_last()
            .filter(|(name, _)| name.eq_ignore_ascii_case(ENGINE_EXE_NAME))
            .map(|(_, root)| root)
            .into_iter()
            .collect(),
        SelfModule::Settings => own
            .split_last()
            .and_then(|(_, dir)| dir.split_last())
            .filter(|(dir, _)| dir.eq_ignore_ascii_case(SETTINGS_DIR_NAME))
            .map(|(_, root)| root)
            .into_iter()
            .collect(),
    }
}

/// インストール先の下で、口の相手が置かれる相対パス。決まっていなければ `None`（拒む）。
fn endpoint_relative(endpoint: Endpoint) -> Option<Vec<&'static str>> {
    match endpoint {
        Endpoint::Engine => Some(vec![ENGINE_EXE_NAME]),
        Endpoint::Candidate => Some(vec![CANDIDATE_DIR_NAME, CANDIDATE_EXE_NAME]),
        Endpoint::Settings => SETTINGS_EXE_NAME.map(|exe| vec![SETTINGS_DIR_NAME, exe]),
    }
}

/// つないだ相手（実行ファイルのパス `peer_exe`）が、`module`（パス `self_path`）から決まるインストール先の
/// `endpoint` の実行ファイルかを判定する。
///
/// どちらも OS が返したフルパス（8.3 の短い名前は呼ぶ側が展開しておく）。
pub fn verify_peer_path(
    module: SelfModule,
    self_path: &str,
    endpoint: Endpoint,
    peer_exe: &str,
) -> Result<(), PeerRejection> {
    let own = normalize(self_path).ok_or(PeerRejection::SelfPathInvalid)?;
    let roots = install_roots(module, &own.components);
    if roots.is_empty() {
        return Err(PeerRejection::SelfPathInvalid);
    }
    let peer = normalize(peer_exe).ok_or(PeerRejection::PeerPathInvalid)?;
    let relative = endpoint_relative(endpoint).ok_or(PeerRejection::NotThePeer)?;
    let matches = roots.iter().any(|root| {
        let expected: Vec<&str> = root.iter().chain(relative.iter()).copied().collect();
        peer.same_as(own.drive, &expected)
    });
    if matches {
        Ok(())
    } else {
        Err(PeerRejection::NotThePeer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSIONED_DLL: &str = r"C:\Program Files\ikari-ime\tip\0.3.1\ikari_tip.dll";
    const VERSIONED_ENGINE: &str = r"C:\Program Files\ikari-ime\ikari-engine-host.exe";
    const VERSIONED_CANDIDATE: &str = r"C:\Program Files\ikari-ime\candidate\ikari-candidate.exe";
    const SETTINGS_EXE: &str = r"C:\Program Files\ikari-ime\settings\ikari-settings.exe";
    const FLAT_DLL: &str = r"C:\Users\example\AppData\Local\ikari-ime-dev\tip\ikari_tip.dll";
    const FLAT_ENGINE: &str =
        r"C:\Users\example\AppData\Local\ikari-ime-dev\tip\ikari-engine-host.exe";

    const OK: Result<(), PeerRejection> = Ok(());
    const NOT_PEER: Result<(), PeerRejection> = Err(PeerRejection::NotThePeer);
    const BAD_PEER: Result<(), PeerRejection> = Err(PeerRejection::PeerPathInvalid);
    const BAD_SELF: Result<(), PeerRejection> = Err(PeerRejection::SelfPathInvalid);

    /// 平らな配置を許すビルド（`dev-knobs`）と、製品のビルドで、結果が違う場合の期待
    fn with_flat(
        dev: Result<(), PeerRejection>,
        product: Result<(), PeerRejection>,
    ) -> Result<(), PeerRejection> {
        if cfg!(feature = "dev-knobs") {
            dev
        } else {
            product
        }
    }

    fn tip_engine(tip: &str, peer: &str) -> Result<(), PeerRejection> {
        verify_peer_path(SelfModule::Tip, tip, Endpoint::Engine, peer)
    }

    /// TIP → エンジン（#409 の規則。前の `windows/tip/src/peer.rs` の表をそのまま移した）
    #[test]
    fn tip_to_engine_table() {
        let cases: &[(&str, &str, &str, Result<(), PeerRejection>)] = &[
            // 一致
            ("versioned", VERSIONED_DLL, VERSIONED_ENGINE, OK),
            // 平らな配置は feature `dev-knobs` のビルドだけ許す
            (
                "flat",
                FLAT_DLL,
                FLAT_ENGINE,
                if cfg!(feature = "dev-knobs") {
                    OK
                } else {
                    BAD_SELF
                },
            ),
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
                NOT_PEER,
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
                NOT_PEER,
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
                BAD_SELF,
            ),
            // 似た名前
            (
                "suffix",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe.evil",
                NOT_PEER,
            ),
            (
                "similar name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host2.exe",
                NOT_PEER,
            ),
            (
                "hidden build name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host-hidden.exe",
                NOT_PEER,
            ),
            (
                "alternate data stream",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe:evil",
                NOT_PEER,
            ),
            (
                "trailing dot",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe.",
                NOT_PEER,
            ),
            // 別のフォルダの同名
            (
                "other folder",
                VERSIONED_DLL,
                r"C:\Users\example\Downloads\ikari-engine-host.exe",
                NOT_PEER,
            ),
            (
                "other drive",
                VERSIONED_DLL,
                r"D:\Program Files\ikari-ime\ikari-engine-host.exe",
                NOT_PEER,
            ),
            (
                "prefix folder name",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime-evil\ikari-engine-host.exe",
                NOT_PEER,
            ),
            (
                "inside the version folder of a versioned dll is flat",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\tip\0.3.1\ikari-engine-host.exe",
                if cfg!(feature = "dev-knobs") {
                    OK
                } else {
                    NOT_PEER
                },
            ),
            (
                "inside the tip folder",
                VERSIONED_DLL,
                r"C:\Program Files\ikari-ime\tip\ikari-engine-host.exe",
                NOT_PEER,
            ),
            (
                "versioned rule needs the tip folder",
                r"C:\Users\example\Downloads\x\1\ikari_tip.dll",
                r"C:\Users\example\Downloads\ikari-engine-host.exe",
                with_flat(NOT_PEER, BAD_SELF),
            ),
            (
                "versioned needs two folders above",
                r"C:\tip\ikari_tip.dll",
                r"C:\ikari-engine-host.exe",
                with_flat(NOT_PEER, BAD_SELF),
            ),
            // 空
            ("empty peer", VERSIONED_DLL, "", BAD_PEER),
            ("empty tip", "", VERSIONED_ENGINE, BAD_SELF),
            ("drive only", VERSIONED_DLL, r"C:\", NOT_PEER),
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
                BAD_SELF,
            ),
        ];
        for (name, tip, peer, expected) in cases {
            assert_eq!(tip_engine(tip, peer), *expected, "{name}");
        }
    }

    /// TIP → 候補窓（ADR 0068 §5）
    #[test]
    fn tip_to_candidate() {
        let check =
            |peer| verify_peer_path(SelfModule::Tip, VERSIONED_DLL, Endpoint::Candidate, peer);
        assert_eq!(check(VERSIONED_CANDIDATE), OK);
        assert_eq!(
            check(r"C:\PROGRAM FILES\ikari-ime\Candidate\IKARI-CANDIDATE.EXE"),
            OK
        );
        // インストール先の直下や、ほかのフォルダの同名は通さない
        assert_eq!(
            check(r"C:\Program Files\ikari-ime\ikari-candidate.exe"),
            NOT_PEER
        );
        assert_eq!(
            check(r"C:\Users\example\candidate\ikari-candidate.exe"),
            NOT_PEER
        );
        // エンジンは候補窓の口の相手ではない
        assert_eq!(check(VERSIONED_ENGINE), NOT_PEER);
        assert_eq!(
            check(r"C:\Program Files\ikari-ime\candidate\..\x.exe"),
            BAD_PEER
        );
    }

    /// 設定画面 → engine-host（ADR 0068 §4）
    #[test]
    fn settings_to_engine() {
        let check = |own, peer| verify_peer_path(SelfModule::Settings, own, Endpoint::Engine, peer);
        assert_eq!(check(SETTINGS_EXE, VERSIONED_ENGINE), OK);
        assert_eq!(
            check(
                SETTINGS_EXE,
                r"C:\Program Files\ikari-ime\settings\ikari-engine-host.exe"
            ),
            NOT_PEER
        );
        // 設定画面が `settings\` の下にいなければ、インストール先を決めない
        assert_eq!(
            check(
                r"C:\Program Files\ikari-ime\ikari-settings.exe",
                VERSIONED_ENGINE
            ),
            BAD_SELF
        );
    }

    /// engine-host → 設定画面（ADR 0068 §4）。設定画面の exe の名前が決まるまでは常に拒む
    #[test]
    fn engine_host_to_settings_rejects_until_the_name_is_decided() {
        assert_eq!(SETTINGS_EXE_NAME, None);
        assert_eq!(
            verify_peer_path(
                SelfModule::EngineHost,
                VERSIONED_ENGINE,
                Endpoint::Settings,
                SETTINGS_EXE
            ),
            NOT_PEER
        );
    }

    #[test]
    fn engine_host_self_path_must_be_the_engine_exe() {
        // engine-host の自分のパスからインストール先を決める（候補窓との照合で確かめる）
        let check = |own| {
            verify_peer_path(
                SelfModule::EngineHost,
                own,
                Endpoint::Candidate,
                VERSIONED_CANDIDATE,
            )
        };
        assert_eq!(check(VERSIONED_ENGINE), OK);
        assert_eq!(check(r"C:\Program Files\ikari-ime\other.exe"), BAD_SELF);
        assert_eq!(check(r"C:\"), BAD_SELF);
    }

    #[test]
    fn rejection_texts_are_distinct() {
        let all = [
            PeerRejection::NoPeerPid,
            PeerRejection::NoPeerPath,
            PeerRejection::NoSelfPath,
            PeerRejection::SelfPathInvalid,
            PeerRejection::PeerPathInvalid,
            PeerRejection::NotThePeer,
        ];
        let texts: std::collections::HashSet<_> = all.iter().map(|r| r.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
