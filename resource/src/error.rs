//! 束を拒む理由。理由ごとに区別できる（エンジンは `rejected/` への移動と状態の表示で出し分ける）。

use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum ResourceError {
    /// 読み書きに失敗した（権限、ディスクなど）
    Io { path: PathBuf, source: io::Error },
    /// manifest.json がない
    ManifestMissing,
    /// manifest.json が JSON として、または型として読めない（壊れている）
    ManifestMalformed(String),
    /// 知らない schema の版（新しいアプリが要る）
    UnknownSchema { found: u64, supported: u32 },
    /// 知らない kind
    UnknownKind { component: String, kind: String },
    /// 名前が束の直下に収まらない（`..`、区切り、絶対パス、空）
    UnsafePath { name: String },
    /// シンボリックリンク（rename するとリンクのまま移るので拒む）
    Symlink { name: String },
    /// 普通のファイルではないもの（ディレクトリなど）
    NotRegularFile { name: String },
    /// manifest に載っているのに束にない
    MissingFile { name: String },
    /// 大きさが manifest より小さい（途中で切れた）
    Truncated {
        name: String,
        expected: u64,
        actual: u64,
    },
    /// 大きさが manifest より大きい
    SizeMismatch {
        name: String,
        expected: u64,
        actual: u64,
    },
    /// sha256 が manifest と違う（壊れている）
    Sha256Mismatch {
        name: String,
        expected: String,
        actual: String,
    },
    /// 束にあるのに manifest にない
    UnlistedFile { name: String },
    /// 形式の版がこのアプリの読める版と違う（アプリの更新が要る、または束が古い）
    FormatMismatch {
        component: String,
        found: u32,
        supported: u32,
    },
    /// 辞書の部品に、3 ファイルのうち足りないものがある（一部だけの辞書は拒む）
    IncompleteDict { component: String, missing: String },
    /// アプリに埋めた既知のモデルにない
    UnknownModel { name: String, sha256: String },
    /// 今の版より古い（戻すのは「前の版に戻す」で行う）
    Stale {
        component: String,
        bundle_created: String,
        installed_created: String,
    },
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "{} を読み書きできません: {source}", path.display())
            }
            Self::ManifestMissing => write!(f, "manifest.json がありません"),
            Self::ManifestMalformed(why) => write!(f, "manifest.json が壊れています: {why}"),
            Self::UnknownSchema { found, supported } => write!(
                f,
                "manifest の schema {found} は読めません（このアプリは {supported}。アプリの更新が要ります）"
            ),
            Self::UnknownKind { component, kind } => {
                write!(f, "部品 {component} の kind {kind:?} は知りません")
            }
            Self::UnsafePath { name } => write!(f, "束の中に収まらない名前です: {name:?}"),
            Self::Symlink { name } => write!(f, "シンボリックリンクは受けません: {name}"),
            Self::NotRegularFile { name } => write!(f, "普通のファイルではありません: {name}"),
            Self::MissingFile { name } => write!(f, "manifest にあるファイルがありません: {name}"),
            Self::Truncated {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} が途中で切れています（期待 {expected} バイト、実際 {actual} バイト）"
            ),
            Self::SizeMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} の大きさが違います（期待 {expected} バイト、実際 {actual} バイト）"
            ),
            Self::Sha256Mismatch {
                name,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "{name} の sha256 が違います（期待 {expected}、実際 {actual}）"
                )
            }
            Self::UnlistedFile { name } => write!(f, "manifest にないファイルがあります: {name}"),
            Self::FormatMismatch {
                component,
                found,
                supported,
            } => write!(
                f,
                "部品 {component} の形式の版 {found} は読めません（このアプリは {supported}）"
            ),
            Self::IncompleteDict { component, missing } => write!(
                f,
                "辞書 {component} に {missing} がありません（一部だけの辞書は入れません）"
            ),
            Self::UnknownModel { name, sha256 } => {
                write!(f, "既知のモデルにありません: {name}（sha256 {sha256}）")
            }
            Self::Stale {
                component,
                bundle_created,
                installed_created,
            } => write!(
                f,
                "部品 {component} は今の版（{installed_created}）より古い束です（{bundle_created}）"
            ),
        }
    }
}

impl std::error::Error for ResourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl ResourceError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
