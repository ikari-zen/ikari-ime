//! `cargo xtask data install`（ADR 0043 段 X3）。辞書とモデルをユーザーのデータのディレクトリに入れる（ADR 0004 §8.3）。
//! `scripts/install-data.sh` の移し先。
//!
//! - `<src>/build/dict/{grammar.bin,system.dict,vectors.bin}` -> `<dest>/dict/`
//! - `<src>/build/dict/emoji.tsv`、`postal.tsv`（あれば。なくてもよい） -> `<dest>/dict/`
//! - `<src>/raw/zenz/<モデル>.gguf` -> `<dest>/models/`
//!
//! コピーは同じディレクトリの一時ファイルに書いてから rename する。エンジンが mmap している最中のファイルを
//! 直接書き換えない（古い inode はエンジンが閉じるまで生きる）。内容が同じファイルは飛ばす。
//! `--src` の既定: このリポジトリの data/。なければ（git worktree のとき）本体のチェックアウトの data/。
//! `--dest` の既定は [`default_dest`]。

use std::path::{Path, PathBuf};

use super::consts::LibConsts;
use super::non_empty;
use super::run::{Failure, capture, git, repo_root, warn};

/// ユーザーのデータのディレクトリを指す環境変数（Rust 側 `ikari_core::user::dir::USER_DIR_ENV` と同じ名前）
const USER_DIR_ENV: &str = "IKARI_USER_DIR";
/// 旧名（当面は読む）
const LEGACY_DATA_DIR_ENV: &str = "IKARI_DATA_DIR";
const APP_DIR_NAME: &str = "ikari-ime";
const MACOS_SUPPORT_DIR: &str = "Library/Application Support";
const XDG_DATA_FALLBACK: &str = ".local/share";
const DICT_SUBDIR: &str = "dict";
const MODELS_SUBDIR: &str = "models";
const BUILT_DICT_SUBDIR: &str = "build/dict";

/// 辞書とモデルを、ユーザーのデータのディレクトリに入れる
#[derive(Debug, clap::Args)]
pub struct Args {
    /// data のディレクトリ（既定: このリポジトリの data/。なければ git worktree の本体の data/）
    #[arg(long)]
    src: Option<PathBuf>,
    /// ikari-ime のディレクトリ（既定: $IKARI_USER_DIR、なければ OS ごとの既定。macOS は ~/Library/Application Support/ikari-ime）
    #[arg(long)]
    dest: Option<PathBuf>,
    /// モデルを入れない
    #[arg(long)]
    no_model: bool,
    /// コピーせず、何をするかだけ表示する
    #[arg(long)]
    dry_run: bool,
}

/// 既定の置き場所を決めるための環境の値（環境変数の読み出しと分け、純粋関数でテストする）。
#[derive(Debug, Default, Clone)]
struct DestEnv {
    ikari_user_dir: Option<PathBuf>,
    legacy_data_dir: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    local_appdata: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl DestEnv {
    fn from_process() -> Self {
        let get = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            ikari_user_dir: get(USER_DIR_ENV),
            legacy_data_dir: get(LEGACY_DATA_DIR_ENV),
            xdg_data_home: get("XDG_DATA_HOME"),
            local_appdata: get("LOCALAPPDATA"),
            home: get("HOME").or_else(|| get("USERPROFILE")),
        }
    }
}

/// 置き場所。`warn_legacy` が true なら旧名の環境変数を使ったことを知らせる。
/// OS ごと: macOS は `~/Library/Application Support/ikari-ime`、Windows は `%LOCALAPPDATA%\ikari-ime`、
/// それ以外は `$XDG_DATA_HOME/ikari-ime`（なければ `~/.local/share/ikari-ime`。`ikari_core::user::dir` と同じ）。
fn default_dest(env: &DestEnv, os: &str) -> Result<(PathBuf, bool), Failure> {
    if let Some(dir) = &env.ikari_user_dir {
        return Ok((dir.clone(), false));
    }
    if let Some(dir) = &env.legacy_data_dir {
        return Ok((dir.clone(), true));
    }
    let no_home =
        || Failure::new("置き場所を決められません（HOME がありません）。--dest で指定してください");
    let dir = match os {
        "macos" => env
            .home
            .as_ref()
            .ok_or_else(no_home)?
            .join(MACOS_SUPPORT_DIR)
            .join(APP_DIR_NAME),
        "windows" => match (&env.local_appdata, &env.home) {
            (Some(local_appdata), _) => local_appdata.join(APP_DIR_NAME),
            (None, Some(home)) => home.join("AppData/Local").join(APP_DIR_NAME),
            (None, None) => return Err(no_home()),
        },
        _ => match (&env.xdg_data_home, &env.home) {
            (Some(xdg), _) => xdg.join(APP_DIR_NAME),
            (None, Some(home)) => home.join(XDG_DATA_FALLBACK).join(APP_DIR_NAME),
            (None, None) => return Err(no_home()),
        },
    };
    Ok((dir, false))
}

/// `--src` の既定: このリポジトリの data/。なければ（git worktree のとき）共通の .git のあるチェックアウトの data/。
fn find_src(root: &Path) -> Result<PathBuf, Failure> {
    let local = root.join("data");
    if local.join(BUILT_DICT_SUBDIR).is_dir() {
        return Ok(local);
    }
    let common =
        capture(git(root).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]));
    if let Some(common) = common
        && let Some(main_checkout) = Path::new(common.trim_end_matches(['\n', '\r'])).parent()
        && main_checkout.join("data").join(BUILT_DICT_SUBDIR).is_dir()
    {
        return Ok(main_checkout.join("data"));
    }
    Err(Failure::new(
        "data/build/dict が見つかりません。--src で指定してください",
    ))
}

pub fn run(args: Args) -> Result<(), Failure> {
    let root = repo_root();
    let consts = LibConsts::load(&root).map_err(|e| Failure::new(e.to_string()))?;
    let dest = match args.dest {
        Some(dest) => dest,
        None => {
            let (dest, legacy) = default_dest(&DestEnv::from_process(), std::env::consts::OS)?;
            if legacy {
                warn("IKARI_DATA_DIR は旧名です。IKARI_USER_DIR を使ってください");
            }
            dest
        }
    };
    let src = match args.src {
        Some(src) => src,
        None => find_src(&root)?,
    };
    non_empty(&dest, "--dest")?;

    let get = |name: &str| consts.get(name).map_err(|e| Failure::new(e.to_string()));
    let mut installer = Installer {
        dry_run: args.dry_run,
    };
    println!("src:  {}\ndest: {}", src.display(), dest.display());
    let built = src.join(BUILT_DICT_SUBDIR);
    let dict_dest = dest.join(DICT_SUBDIR);
    for file in consts
        .array("DICT_FILES")
        .map_err(|e| Failure::new(e.to_string()))?
    {
        installer.install_file(&built.join(file), &dict_dest.join(file))?;
    }
    // 絵文字の表は任意（ない環境では入れない。ショートコードが働かないだけ。ADR 0027 §8）。
    // 住所と郵便番号の表も任意（郵便番号の変換が働かないだけ。ADR 0032 §3）
    for name in ["EMOJI_TABLE_FILE", "POSTAL_TABLE_FILE"] {
        let file = get(name)?;
        if built.join(file).is_file() {
            installer.install_file(&built.join(file), &dict_dest.join(file))?;
        }
    }
    if !args.no_model {
        let model = get("ZENZ_MODEL_FILE")?;
        installer.install_file(
            &src.join("raw/zenz").join(model),
            &dest.join(MODELS_SUBDIR).join(model),
        )?;
    }
    Ok(())
}

struct Installer {
    dry_run: bool,
}

impl Installer {
    /// tmp に書いて rename する。同じ内容なら何もしない
    fn install_file(&mut self, from: &Path, to: &Path) -> Result<(), Failure> {
        if !from.is_file() {
            return Err(Failure::new(format!("{} がありません", from.display())));
        }
        if to.is_file() && same_content(from, to) {
            println!("unchanged  {}", to.display());
            return Ok(());
        }
        if self.dry_run {
            println!("would copy {} -> {}", from.display(), to.display());
            return Ok(());
        }
        let parent = to.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|e| Failure::new(format!("{} を作れません: {e}", parent.display())))?;
        let mut tmp_name = to.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".tmp.{}", std::process::id()));
        let tmp = to.with_file_name(tmp_name);
        let result = std::fs::copy(from, &tmp)
            .map_err(|e| Failure::new(format!("{} をコピーできません: {e}", from.display())))
            .and_then(|_| set_readable(&tmp))
            .and_then(|()| {
                std::fs::rename(&tmp, to)
                    .map_err(|e| Failure::new(format!("{} に置けません: {e}", to.display())))
            });
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result?;
        println!("installed  {}", to.display());
        Ok(())
    }
}

#[cfg(unix)]
fn set_readable(path: &Path) -> Result<(), Failure> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
        .map_err(|e| Failure::new(format!("{} の権限を設定できません: {e}", path.display())))
}

#[cfg(not(unix))]
fn set_readable(_path: &Path) -> Result<(), Failure> {
    Ok(())
}

/// 内容が同じか（大きさが違えば読まずに false。読めなければ false で、コピーに進む）
fn same_content(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else {
        return false;
    };
    let mut ra = std::io::BufReader::with_capacity(1 << 20, fa);
    let mut rb = std::io::BufReader::with_capacity(1 << 20, fb);
    let mut buf_a = vec![0u8; 1 << 16];
    let mut buf_b = vec![0u8; 1 << 16];
    loop {
        let Ok(na) = ra.read(&mut buf_a) else {
            return false;
        };
        if na == 0 {
            return rb.read(&mut buf_b).is_ok_and(|n| n == 0);
        }
        if rb.read_exact(&mut buf_b[..na]).is_err() || buf_a[..na] != buf_b[..na] {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(f: impl FnOnce(&mut DestEnv)) -> DestEnv {
        let mut e = DestEnv {
            home: Some(PathBuf::from("/home/u")),
            ..DestEnv::default()
        };
        f(&mut e);
        e
    }

    #[test]
    fn dest_defaults_per_os() {
        let e = env(|_| {});
        assert_eq!(
            default_dest(&e, "macos").unwrap().0,
            Path::new("/home/u/Library/Application Support/ikari-ime")
        );
        assert_eq!(
            default_dest(&e, "linux").unwrap().0,
            Path::new("/home/u/.local/share/ikari-ime")
        );
        let xdg = env(|e| e.xdg_data_home = Some("/x".into()));
        assert_eq!(
            default_dest(&xdg, "linux").unwrap().0,
            Path::new("/x/ikari-ime")
        );
        let win = env(|e| e.local_appdata = Some("C:/Local".into()));
        assert_eq!(
            default_dest(&win, "windows").unwrap().0,
            Path::new("C:/Local/ikari-ime")
        );
    }

    #[test]
    fn dest_env_precedence_and_legacy_flag() {
        let e = env(|e| {
            e.ikari_user_dir = Some("/new".into());
            e.legacy_data_dir = Some("/old".into());
        });
        assert_eq!(
            default_dest(&e, "macos").unwrap(),
            (PathBuf::from("/new"), false)
        );
        let e = env(|e| e.legacy_data_dir = Some("/old".into()));
        assert_eq!(
            default_dest(&e, "macos").unwrap(),
            (PathBuf::from("/old"), true)
        );
        assert!(default_dest(&DestEnv::default(), "macos").is_err());
    }

    #[test]
    fn install_copies_skips_same_and_honors_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.bin");
        let to = dir.path().join("out/sub/a.bin");
        std::fs::write(&from, b"hello").unwrap();

        Installer { dry_run: true }
            .install_file(&from, &to)
            .unwrap();
        assert!(!to.exists());

        Installer { dry_run: false }
            .install_file(&from, &to)
            .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"hello");
        assert!(same_content(&from, &to));
        // 一時ファイルを残さない
        assert_eq!(std::fs::read_dir(to.parent().unwrap()).unwrap().count(), 1);

        std::fs::write(&from, b"hellp").unwrap();
        assert!(!same_content(&from, &to));
        Installer { dry_run: false }
            .install_file(&from, &to)
            .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"hellp");
    }

    #[test]
    fn missing_source_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = Installer { dry_run: false }
            .install_file(&dir.path().join("none"), &dir.path().join("x"))
            .unwrap_err();
        assert!(err.message.contains("がありません"));
    }
}
