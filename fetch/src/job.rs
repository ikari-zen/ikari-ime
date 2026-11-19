//! 取得の仕事の流れ（ADR 0076 §5.1、§8）。
//!
//! 錠を取る → 前の残りを消す → ディスクの見積もり → 作業の場所に `download.bin` を作り、そのハンドルを最後まで持つ
//! （パスで開き直さない。ADR 0078 §3.1 の 2）→ そこに落とす（`Fetcher`。Windows は低い整合性の子が、追記だけの写しに
//! 書く。macOS は自分）→ **落としたファイルは信じず**、`inbox/<名前>.partial/` に写しながら sha256 を数え直す
//! （`stage_model_bundle`）→ rename → 作業の場所を消す → `done`。どの失敗でも、作業の場所と `.partial` を消し、
//! 理由の語を 1 つ出して終わる（自動では取り直さない）。
//!
//! 取り込みの判断（束の検証、試しの採点、入れ替え）はエンジン。ここは束を inbox に置くまで。

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use ikari_resource::fetch_report::{ChildBox, EXIT_ERROR, EXIT_OK, FetchErrorCode, Report};
use ikari_resource::sha256::sha256_reader;
use ikari_resource::{
    ComponentKind, Created, Expected, ModelPreset, Origin, Source, StagePlan, preset_by_id,
    stage_model_bundle,
};

use crate::args::ModelSource;
use crate::error::FetchError;
use crate::limits::{MODEL_MAX_SIZE, PROGRESS_INTERVAL};
use crate::url::HttpsUrl;
use crate::workspace::{Dirs, required_space, required_space_to_copy};

/// 自由指定の束の名前の前につける語と、sha256 から取る桁数（ADR 0076 §5.1）。エンジンの取り込みと同じ値
const USER_NAME_PREFIX: &str = ikari_resource::USER_MODEL_PREFIX;
const USER_NAME_HEX_DIGITS: usize = ikari_resource::USER_MODEL_HEX_DIGITS;
const MODEL_FILE_EXTENSION: &str = ikari_resource::MODEL_FILE_EXTENSION;

/// 取得の途中に `Fetcher` が知らせること。行にするのは `try_model_fetch`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchEvent {
    /// 受けたバイトと全体（間引いて `progress` の行にする）
    Progress { received: u64, total: Option<u64> },
    /// 取得の子を起こした檻（Windows だけ。`box` の行にする。ADR 0078 §3.2 の 5）
    Box(ChildBox),
}

/// URL の中身を、親が作った作業の場所のファイル（`Dirs::create_download`）に落とす部品。製品は `ProductFetcher`、
/// テストは差し替える
pub trait Fetcher {
    /// `out`（空のファイル。読み書きで開いている）の末尾に落とす。終えたら、書いた分がディスクに出ているようにして
    /// 返す。途中のことは `events` で知らせる。失敗の理由は語で返す
    fn fetch(
        &self,
        job: &DownloadJob<'_>,
        out: &File,
        events: &mut dyn FnMut(FetchEvent),
    ) -> Result<(), FetchError>;
}

#[derive(Debug, Clone)]
pub struct DownloadJob<'a> {
    pub url: &'a HttpsUrl,
    pub max_size: u64,
    pub expect_size: Option<u64>,
}

/// 実行の文脈
pub struct Context<'a> {
    pub dirs: &'a Dirs,
    pub fetcher: &'a dyn Fetcher,
    /// 作る束の manifest の日時
    pub created: Created,
}

/// モデルの取得。行を `out` に出し、終了コードを返す（0 または 1）
pub fn model_fetch(source: &ModelSource, ctx: &Context<'_>, out: &mut dyn Write) -> i32 {
    match try_model_fetch(source, ctx, out) {
        Ok(done) => {
            emit(out, &done);
            EXIT_OK
        }
        Err(e) => {
            emit(out, &Report::Error(e.code()));
            EXIT_ERROR
        }
    }
}

/// 落ちたあとの片づけ（通信しない）
pub fn cleanup(dirs: &Dirs, out: &mut dyn Write) -> i32 {
    let result = dirs.lock().and_then(|lock| {
        dirs.cleanup(&lock)
            .map_err(|_| FetchErrorCode::Internal.into())
    });
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            emit(out, &Report::Error(e.code()));
            EXIT_ERROR
        }
    }
}

/// 1 行を出す（ここで切れたら、読む側が終わりの行を待たずに閉じる）
pub fn emit(out: &mut dyn Write, report: &Report) {
    let _ = writeln!(out, "{}", report.to_line());
    let _ = out.flush();
}

/// 進み具合の行の間引き（行が多すぎて、読む側が詰まらないように）
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    pub fn standard() -> Self {
        Self::new(PROGRESS_INTERVAL)
    }

    /// `now` に出してよいか。出すなら時刻を覚える（最初は必ず出す）
    pub fn ready(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// 求めた束の計画（URL を含む）
struct Resolved<'a> {
    url: HttpsUrl,
    max_size: u64,
    expect_size: Option<u64>,
    preset: Option<&'a ModelPreset>,
}

fn resolve(source: &ModelSource) -> Result<Resolved<'static>, FetchError> {
    match source {
        ModelSource::Preset { id } => {
            let preset = preset_by_id(id).ok_or(FetchErrorCode::BadUrl)?;
            let url = HttpsUrl::parse(preset.url)?;
            // 表の書き損じで別のホストに行かないように、要求の前に確かめる
            if url.host() != preset.host {
                return Err(FetchErrorCode::BadUrl.into());
            }
            Ok(Resolved {
                url,
                max_size: preset.size,
                expect_size: Some(preset.size),
                preset: Some(preset),
            })
        }
        ModelSource::Url { url, .. } => Ok(Resolved {
            url: HttpsUrl::parse(url)?,
            max_size: MODEL_MAX_SIZE,
            expect_size: None,
            preset: None,
        }),
    }
}

fn try_model_fetch(
    source: &ModelSource,
    ctx: &Context<'_>,
    out: &mut dyn Write,
) -> Result<Report, FetchError> {
    let resolved = resolve(source)?;
    let dirs = ctx.dirs;
    let lock = dirs.lock()?;
    let result = (|| {
        dirs.cleanup(&lock)
            .map_err(|_| FetchError(FetchErrorCode::Internal))?;
        dirs.check_space(required_space(resolved.expect_size, resolved.max_size))?;

        let job = DownloadJob {
            url: &resolved.url,
            max_size: resolved.max_size,
            expect_size: resolved.expect_size,
        };
        let download = dirs
            .create_download(&lock)
            .map_err(|_| FetchError(FetchErrorCode::Internal))?;
        let mut throttle = Throttle::standard();
        ctx.fetcher
            .fetch(&job, &download, &mut |event| match event {
                FetchEvent::Progress { received, total } => {
                    if throttle.ready(Instant::now()) {
                        emit(out, &Report::Progress { received, total });
                    }
                }
                FetchEvent::Box(child_box) => emit(out, &Report::Box(child_box)),
            })?;
        stage_downloaded(source, &resolved, ctx, &download)
        // `download` はここで閉じる（消す前に）
    })();
    // 成功でも失敗でも、作業の場所と `.partial` は残さない（消せなくても、元の結果を返す）
    let _ = dirs.cleanup(&lock);
    result
}

/// 落としたファイル（親が作って持っているハンドル）を、信じずに inbox の束へ写す
fn stage_downloaded(
    source: &ModelSource,
    resolved: &Resolved<'_>,
    ctx: &Context<'_>,
    download: &File,
) -> Result<Report, FetchError> {
    let dirs = ctx.dirs;
    // 大きさは、持っているハンドルで見る（パスでは見ない）
    let received = download
        .metadata()
        .map_err(|_| FetchError(FetchErrorCode::Internal))?
        .len();
    if received > resolved.max_size {
        return Err(FetchErrorCode::TooLarge.into());
    }
    dirs.check_space(required_space_to_copy(received))?;

    // 名前（`user-<sha256 の先頭 12 桁>`）のために、落としたファイルの sha256 を先に数える。
    // 写すときに数え直し、食い違えば（途中で書き換えられた）捨てる
    let (plan, pre_hash) = match (resolved.preset, source) {
        (Some(preset), _) => (preset_plan(preset, ctx), None),
        (
            None,
            ModelSource::Url {
                url,
                format,
                sha256,
                name,
            },
        ) => {
            let hash = sha256_reader(ReadFromStart::new(download))
                .map_err(|_| FetchError(FetchErrorCode::Internal))?;
            let plan = user_plan(&hash, url, format, sha256.as_deref(), name.as_deref(), ctx);
            (plan, Some(hash))
        }
        (None, ModelSource::Preset { .. }) => return Err(FetchErrorCode::Internal.into()),
    };

    let staged = stage_model_bundle(&mut ReadFromStart::new(download), &dirs.inbox, &plan)?;
    if pre_hash.is_some_and(|h| h != staged.sha256) {
        let _ = fs::remove_dir_all(&staged.path);
        return Err(FetchErrorCode::Checksum.into());
    }
    Ok(Report::Done {
        name: staged.name,
        sha256: staged.sha256,
    })
}

/// ファイルを先頭から読む。ファイルの位置は使わない（読む場所を自分で数え、その場所を指定して読む）。
/// Windows では子に渡した写しとファイルの位置を分け合うので、位置に頼らない（ADR 0078 §3.1 の 7）
struct ReadFromStart<'a> {
    file: &'a File,
    offset: u64,
}

impl<'a> ReadFromStart<'a> {
    fn new(file: &'a File) -> Self {
        Self { file, offset: 0 }
    }
}

impl Read for ReadFromStart<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(self.file, buf, self.offset)?;
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(self.file, buf, self.offset)?;
        #[cfg(not(any(unix, windows)))]
        let n: usize = {
            let _ = buf;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "positional read",
            ));
        };
        self.offset = self.offset.saturating_add(n as u64);
        Ok(n)
    }
}

fn preset_plan(preset: &ModelPreset, ctx: &Context<'_>) -> StagePlan {
    StagePlan {
        name: preset.id.to_owned(),
        file_name: preset.file_name.to_owned(),
        kind: ComponentKind::Model,
        origin: Origin::Preset,
        created: ctx.created.clone(),
        expected: Some(Expected {
            sha256: preset.sha256.to_owned(),
            size: Some(preset.size),
        }),
        profile_format: None,
        display_name: None,
        source: Some(Source {
            url: preset.url.to_owned(),
            sha256_given: true,
        }),
        max_size: preset.size,
    }
}

fn user_plan(
    hash: &str,
    url: &str,
    format: &str,
    given_sha256: Option<&str>,
    display_name: Option<&str>,
    ctx: &Context<'_>,
) -> StagePlan {
    let name = format!("{USER_NAME_PREFIX}{}", &hash[..USER_NAME_HEX_DIGITS]);
    StagePlan {
        file_name: format!("{name}{MODEL_FILE_EXTENSION}"),
        name,
        kind: ComponentKind::Model,
        origin: Origin::User,
        created: ctx.created.clone(),
        expected: given_sha256.map(|sha256| Expected {
            sha256: sha256.to_owned(),
            size: None,
        }),
        profile_format: Some(format.to_owned()),
        display_name: display_name.map(str::to_owned),
        source: Some(Source {
            url: url.to_owned(),
            sha256_given: given_sha256.is_some(),
        }),
        max_size: MODEL_MAX_SIZE,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use ikari_resource::fetch_report::parse_line;
    use ikari_resource::{MANIFEST_FILE_NAME, Manifest};

    use super::*;

    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// 決めたバイト列を、作業の場所に書くだけの `Fetcher`
    struct Fake {
        body: Vec<u8>,
        /// 書く前に知らせる檻（Windows の製品の形をまねる）
        child_box: Option<ChildBox>,
        fail: Option<FetchErrorCode>,
        calls: Cell<u32>,
        seen_url: RefCell<String>,
    }

    impl Fake {
        fn serving(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                child_box: None,
                fail: None,
                calls: Cell::new(0),
                seen_url: RefCell::new(String::new()),
            }
        }
    }

    impl Fetcher for Fake {
        fn fetch(
            &self,
            job: &DownloadJob<'_>,
            mut out: &File,
            events: &mut dyn FnMut(FetchEvent),
        ) -> Result<(), FetchError> {
            self.calls.set(self.calls.get() + 1);
            *self.seen_url.borrow_mut() = job.url.as_str().to_owned();
            if let Some(child_box) = self.child_box {
                events(FetchEvent::Box(child_box));
            }
            out.write_all(&self.body).unwrap();
            let len = self.body.len() as u64;
            events(FetchEvent::Progress {
                received: len,
                total: Some(len),
            });
            match self.fail {
                Some(code) => Err(code.into()),
                None => Ok(()),
            }
        }
    }

    fn setup() -> (tempfile::TempDir, Dirs) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::new(tmp.path().join("inbox"), tmp.path().join("fetch"));
        (tmp, dirs)
    }

    fn ctx<'a>(dirs: &'a Dirs, fetcher: &'a dyn Fetcher) -> Context<'a> {
        Context {
            dirs,
            fetcher,
            created: Created::parse("2026-11-17T00:00:00Z").unwrap(),
        }
    }

    fn url_source(sha256: Option<&str>) -> ModelSource {
        ModelSource::Url {
            url: "https://example.com/m.gguf".into(),
            format: "zenz".into(),
            sha256: sha256.map(str::to_owned),
            name: Some("my model".into()),
        }
    }

    fn reports(out: &[u8]) -> Vec<Report> {
        String::from_utf8_lossy(out)
            .lines()
            .map(|l| parse_line(l).unwrap_or_else(|| panic!("読めない行: {l}")))
            .collect()
    }

    fn leftovers(dirs: &Dirs) -> Vec<String> {
        let mut v = Vec::new();
        for dir in [&dirs.work, &dirs.inbox] {
            for e in fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name != ".lock" && name.ends_with(".partial") || e.path() == dirs.download_path()
                {
                    v.push(name);
                }
            }
        }
        v
    }

    #[test]
    fn user_url_with_matching_sha256_is_staged_and_reported() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        let code = model_fetch(&url_source(Some(ABC_SHA)), &ctx(&dirs, &fake), &mut out);
        assert_eq!(code, EXIT_OK);
        let r = reports(&out);
        assert_eq!(
            r.last(),
            Some(&Report::Done {
                name: "user-ba7816bf8f01".into(),
                sha256: ABC_SHA.into()
            })
        );
        let bundle = dirs.inbox.join("user-ba7816bf8f01");
        let m = Manifest::read(&bundle.join(MANIFEST_FILE_NAME)).unwrap();
        assert_eq!(m.origin, Origin::User);
        let c = &m.components["model"];
        assert_eq!(c.profile.as_ref().unwrap()["format"], "zenz");
        assert_eq!(c.profile.as_ref().unwrap()["name"], "my model");
        assert_eq!(c.sources.as_ref().unwrap()["sha256_given"], true);
        assert_eq!(
            fs::read(bundle.join("user-ba7816bf8f01.gguf")).unwrap(),
            b"abc"
        );
        assert!(leftovers(&dirs).is_empty());
        assert!(!dirs.download_path().exists());
    }

    #[test]
    fn no_sha256_is_allowed_and_recorded() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        assert_eq!(
            model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out),
            EXIT_OK
        );
        let m = Manifest::read(
            &dirs
                .inbox
                .join("user-ba7816bf8f01")
                .join(MANIFEST_FILE_NAME),
        )
        .unwrap();
        assert_eq!(
            m.components["model"].sources.as_ref().unwrap()["sha256_given"],
            false
        );
    }

    #[test]
    fn sha256_mismatch_discards_everything() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        let wrong = "0".repeat(64);
        let mut out = Vec::new();
        let code = model_fetch(&url_source(Some(&wrong)), &ctx(&dirs, &fake), &mut out);
        assert_eq!(code, EXIT_ERROR);
        assert_eq!(
            reports(&out).last(),
            Some(&Report::Error(FetchErrorCode::Checksum))
        );
        assert!(leftovers(&dirs).is_empty());
        assert!(!dirs.inbox.join("user-000000000000").exists());
    }

    #[test]
    fn fetcher_failure_is_reported_by_word_and_cleans_up() {
        let (_tmp, dirs) = setup();
        let mut fake = Fake::serving(b"abc");
        fake.fail = Some(FetchErrorCode::Tls);
        let mut out = Vec::new();
        let code = model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out);
        assert_eq!(code, EXIT_ERROR);
        assert_eq!(
            reports(&out).last(),
            Some(&Report::Error(FetchErrorCode::Tls))
        );
        assert!(leftovers(&dirs).is_empty());
        // 自動では取り直さない
        assert_eq!(fake.calls.get(), 1);
    }

    #[test]
    fn bad_urls_never_reach_the_fetcher() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        for url in [
            "http://example.com/m",
            "https://user@example.com/m",
            "https://例え.jp/m",
        ] {
            let source = ModelSource::Url {
                url: url.into(),
                format: "zenz".into(),
                sha256: None,
                name: None,
            };
            let mut out = Vec::new();
            assert_eq!(
                model_fetch(&source, &ctx(&dirs, &fake), &mut out),
                EXIT_ERROR
            );
            assert_eq!(
                reports(&out),
                [Report::Error(FetchErrorCode::BadUrl)],
                "{url}"
            );
        }
        assert_eq!(fake.calls.get(), 0);
    }

    #[test]
    fn unknown_preset_is_bad_url() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        let source = ModelSource::Preset { id: "nope".into() };
        assert_eq!(
            model_fetch(&source, &ctx(&dirs, &fake), &mut out),
            EXIT_ERROR
        );
        assert_eq!(fake.calls.get(), 0);
    }

    #[test]
    fn preset_fetch_uses_the_table_url_and_rejects_wrong_content() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"not the model");
        let mut out = Vec::new();
        let source = ModelSource::Preset {
            id: "zenz-small".into(),
        };
        let code = model_fetch(&source, &ctx(&dirs, &fake), &mut out);
        assert_eq!(code, EXIT_ERROR);
        assert_eq!(*fake.seen_url.borrow(), ikari_resource::ZENZ_SMALL_URL);
        // 表の sha256 と大きさに合わなければ、必ず捨てる（警告で通す道はない）
        assert_eq!(
            reports(&out).last(),
            Some(&Report::Error(FetchErrorCode::Checksum))
        );
        assert!(leftovers(&dirs).is_empty());
        assert!(!dirs.inbox.join("zenz-small").exists());
    }

    #[test]
    fn a_second_fetch_while_locked_is_busy_and_touches_nothing() {
        let (_tmp, dirs) = setup();
        let held = dirs.lock().unwrap();
        fs::write(dirs.download_path(), b"someone else's file").unwrap();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        assert_eq!(
            model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out),
            EXIT_ERROR
        );
        assert_eq!(reports(&out), [Report::Error(FetchErrorCode::Busy)]);
        assert_eq!(fake.calls.get(), 0);
        assert!(
            dirs.download_path().exists(),
            "先に走っている取得の作業を消さない"
        );
        drop(held);
    }

    #[test]
    fn leftovers_from_a_crash_are_removed_at_the_start() {
        let (_tmp, dirs) = setup();
        fs::create_dir_all(dirs.inbox.join("user-old.partial")).unwrap();
        fs::create_dir_all(&dirs.work).unwrap();
        fs::write(dirs.work.join("stale.bin"), b"x").unwrap();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        assert_eq!(
            model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out),
            EXIT_OK
        );
        assert!(!dirs.inbox.join("user-old.partial").exists());
        assert!(!dirs.work.join("stale.bin").exists());
    }

    #[test]
    fn progress_lines_are_valid_reports_and_the_last_line_is_final() {
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(b"abc");
        let mut out = Vec::new();
        model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out);
        let r = reports(&out);
        assert!(matches!(
            r.first(),
            Some(Report::Progress {
                received: 3,
                total: Some(3)
            })
        ));
        assert!(r.last().unwrap().is_final());
        assert!(r[..r.len() - 1].iter().all(|x| !x.is_final()));
    }

    #[test]
    fn the_box_event_becomes_a_line_before_progress_and_does_not_end_the_report() {
        let (_tmp, dirs) = setup();
        let mut fake = Fake::serving(b"abc");
        fake.child_box = Some(ChildBox::Low);
        let mut out = Vec::new();
        assert_eq!(
            model_fetch(&url_source(None), &ctx(&dirs, &fake), &mut out),
            EXIT_OK
        );
        let r = reports(&out);
        assert_eq!(r.first(), Some(&Report::Box(ChildBox::Low)));
        assert!(matches!(r.get(1), Some(Report::Progress { .. })));
        assert!(matches!(r.last(), Some(Report::Done { .. })));
    }

    #[test]
    fn cleanup_command_removes_leftovers_and_reports_busy_when_locked() {
        let (_tmp, dirs) = setup();
        fs::create_dir_all(dirs.inbox.join("x.partial")).unwrap();
        let mut out = Vec::new();
        assert_eq!(cleanup(&dirs, &mut out), EXIT_OK);
        assert!(out.is_empty());
        assert!(!dirs.inbox.join("x.partial").exists());

        let _held = dirs.lock().unwrap();
        assert_eq!(cleanup(&dirs, &mut out), EXIT_ERROR);
        assert_eq!(reports(&out), [Report::Error(FetchErrorCode::Busy)]);
    }

    #[test]
    fn throttle_emits_first_then_at_intervals() {
        let mut t = Throttle::new(Duration::from_millis(100));
        let start = Instant::now();
        assert!(t.ready(start));
        assert!(!t.ready(start + Duration::from_millis(50)));
        assert!(t.ready(start + Duration::from_millis(100)));
        assert!(!t.ready(start + Duration::from_millis(150)));
    }

    #[test]
    fn work_file_over_the_limit_is_too_large() {
        // 取得の部品が上限を守らなくても（作業の場所は信じない）、写す前に断る
        let (_tmp, dirs) = setup();
        let fake = Fake::serving(&[0u8; 8]);
        let source = ModelSource::Preset {
            id: "zenz-small".into(),
        };
        let mut ctx = ctx(&dirs, &fake);
        ctx.created = Created::parse("2026-11-17T00:00:00Z").unwrap();
        let resolved = Resolved {
            url: HttpsUrl::parse("https://example.com/x").unwrap(),
            max_size: 4,
            expect_size: None,
            preset: None,
        };
        let lock = dirs.lock().unwrap();
        fs::create_dir_all(&dirs.inbox).unwrap();
        let mut download = dirs.create_download(&lock).unwrap();
        download.write_all(&[0u8; 8]).unwrap();
        let err = stage_downloaded(&source, &resolved, &ctx, &download).unwrap_err();
        assert_eq!(err.code(), FetchErrorCode::TooLarge);
    }

    #[test]
    fn reading_from_the_start_ignores_the_file_position() {
        use std::io::{Seek, SeekFrom};
        let tmp = tempfile::tempdir().unwrap();
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(tmp.path().join("f"))
            .unwrap();
        file.write_all(b"abcdef").unwrap();
        file.seek(SeekFrom::Start(4)).unwrap();
        let mut got = Vec::new();
        ReadFromStart::new(&file).read_to_end(&mut got).unwrap();
        assert_eq!(got, b"abcdef");
    }

    #[test]
    fn appending_after_staging_does_not_change_the_bundle() {
        // 写したあとに、子に渡した写しの残りから追記されても、束は写した時点のまま（ADR 0078 §6 の (e)）
        let (_tmp, dirs) = setup();
        let lock = dirs.lock().unwrap();
        fs::create_dir_all(&dirs.inbox).unwrap();
        let mut download = dirs.create_download(&lock).unwrap();
        download.write_all(b"abc").unwrap();
        let fake = Fake::serving(b"");
        let ctx = ctx(&dirs, &fake);
        let source = url_source(Some(ABC_SHA));
        let resolved = resolve(&source).unwrap();
        let done = stage_downloaded(&source, &resolved, &ctx, &download).unwrap();
        download.write_all(b"more").unwrap();
        assert_eq!(
            done,
            Report::Done {
                name: "user-ba7816bf8f01".into(),
                sha256: ABC_SHA.into()
            }
        );
        assert_eq!(
            fs::read(
                dirs.inbox
                    .join("user-ba7816bf8f01")
                    .join("user-ba7816bf8f01.gguf")
            )
            .unwrap(),
            b"abc"
        );
    }
}
