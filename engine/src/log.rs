//! `logs/engine.log`（ADR 0004 §7.6、ADR 0034 §3.1・§3.2）。時刻、要求の種類、所要時間、LM の状態、
//! エラーの種類だけを書く。読み、候補、文脈の文字列は書かない（NFR-19）。
//!
//! 管理の要求（ADR 0005 §8）も同じ行で、`kind` は要求の種類（`add_user_dict` など）、`err` は
//! `ErrorCode` か `DataIssueKind` の名前（`SCHEMA_UNSUPPORTED` など）。辞書の語、読み、問題の行の
//! 説明（`DataIssue.detail`）は返事に入るが、ログには書かない。
//!
//! 書き方（ADR 0034 §3.1）: 呼ぶ側は有界のキューに 1 行を入れるだけ（`try_send`。ディスクに触らない）。
//! 満杯なら捨てて数え、次に書けたときに `log_dropped n=N` の行を書く。専用のスレッド `ikari-log` が
//! バッファつきで書き、キューが 1 秒空いたら（または前の flush から 1 秒たったら）flush する。
//!
//! 回し方（ADR 0034 §3.2）: 4MB を超えるとき、またはローカルの日付が変わって最初の行を書くときに、
//! `engine.log` を `engine-YYYYMMDD-HHMMSS.log`（回した時刻）にして gzip（`.log.gz`）にする。
//! 回した世代は 180 日と合計 100MB のどちらか先に来た方で、古い順に消す。

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::write::GzEncoder;
use jiff::Timestamp;
use jiff::tz::TimeZone;

pub const LOG_DIR_NAME: &str = "logs";
pub const LOG_FILE_NAME: &str = "engine.log";
/// キューに置ける行の数（ADR 0034 §3.1）
const QUEUE_CAPACITY: usize = 8192;
/// 書き込みのバッファ（バイト）
const BUFFER_BYTES: usize = 64 * 1024;
/// flush の間隔
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// 書き中の `engine.log` がこれを超えたら回す（ADR 0034 §3.2）
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
/// 回した世代を残す日数（ADR 0034 §9 のユーザー決定）
const KEEP_DAYS: u64 = 180;
/// 回した世代と書き中のログの合計の上限（バイト。同上）
const KEEP_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const THREAD_NAME: &str = "ikari-log";
/// 回した世代の名前: `engine-YYYYMMDD-HHMMSS[-N].log[.gz]`
const ROTATED_PREFIX: &str = "engine-";
const ROTATED_SUFFIX: &str = ".log";
const GZIP_SUFFIX: &str = ".gz";
/// gzip の書き途中（終わったら `.gz` に rename する）
const PARTIAL_SUFFIX: &str = ".tmp";
/// 以前の形（1MB で回した `engine.log.1` など）。起動のあとで今の形にして gzip にする
const LEGACY_PREFIX: &str = "engine.log.";
const STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";
const MILLIS_PER_SECOND: u64 = 1000;
const MILLIS_PER_DAY: u64 = 24 * 60 * 60 * MILLIS_PER_SECOND;
/// 名前がすでにあるとき、後ろにずらして探す秒数の上限（これを超えたら回さない）
const MAX_SAME_SECOND: u32 = 1000;

/// 回し方と保持の値（本番は定数。テストだけが小さくする）
#[derive(Clone, Copy, Debug)]
struct Policy {
    max_bytes: u64,
    keep_days: u64,
    keep_total_bytes: u64,
}

const POLICY: Policy = Policy {
    max_bytes: MAX_LOG_BYTES,
    keep_days: KEEP_DAYS,
    keep_total_bytes: KEEP_TOTAL_BYTES,
};

/// キューに入れる 1 行。文字列の組み立ては書き込みのスレッドで行う。時刻は呼んだ側で取る
struct LogLine {
    millis: u64,
    kind: &'static str,
    micros: u64,
    lm: &'static str,
    error: Option<&'static str>,
    note: Option<String>,
}

enum Msg {
    Line(LogLine),
    /// ここまでに入れた行を書いて flush したら返事をする（テスト用）
    #[cfg_attr(not(test), allow(dead_code))]
    Flush(SyncSender<()>),
    /// 残りを書いて flush し、スレッドを終える
    Stop,
}

struct Shared {
    tx: SyncSender<Msg>,
    /// キューが満杯で捨てた行の数（書き込みのスレッドが `log_dropped` の行にして 0 に戻す）
    dropped: Arc<AtomicU64>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// 開けなければ何も書かない（ログのために入力を止めない）。
pub struct EngineLog {
    inner: Option<Shared>,
}

impl EngineLog {
    pub fn disabled() -> EngineLog {
        EngineLog { inner: None }
    }

    pub fn open(data_dir: &Path) -> EngineLog {
        EngineLog::open_with(data_dir, POLICY, QUEUE_CAPACITY)
    }

    fn open_with(data_dir: &Path, policy: Policy, capacity: usize) -> EngineLog {
        let dir = data_dir.join(LOG_DIR_NAME);
        let Ok(writer) = ikari_core::user::create_private_dir_all(&dir)
            .and_then(|()| Writer::open(&dir, policy))
        else {
            return EngineLog::disabled();
        };
        let (tx, rx) = sync_channel(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        let spawned = std::thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || run(writer, &rx, &counter));
        match spawned {
            Ok(handle) => EngineLog {
                inner: Some(Shared {
                    tx,
                    dropped,
                    worker: Mutex::new(Some(handle)),
                }),
            },
            Err(_) => EngineLog::disabled(),
        }
    }

    /// 1行書く。`kind` と `lm` と `error` は固定の語だけを渡すこと（ユーザーの文字列を入れない）。
    /// 管理の失敗は `DataIssueKind` の名前を `error` に渡す。
    pub fn record(
        &self,
        kind: &'static str,
        micros: u64,
        lm: &'static str,
        error: Option<&'static str>,
    ) {
        self.record_note(kind, micros, lm, error, None);
    }

    /// `record` に、固定の語だけの注記（`segments=3 dir=shrink` など）を添える。ユーザーの文字列を入れないこと。
    /// キューに入れるだけで、ディスクには触らない（満杯なら捨てて数える）
    pub fn record_note(
        &self,
        kind: &'static str,
        micros: u64,
        lm: &'static str,
        error: Option<&'static str>,
        note: Option<&str>,
    ) {
        let Some(shared) = &self.inner else {
            return;
        };
        let line = LogLine {
            millis: now_millis(),
            kind,
            micros,
            lm,
            error,
            note: note.map(str::to_owned),
        };
        match shared.tx.try_send(Msg::Line(line)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// 残りを書いて flush し、書き込みのスレッドを終えるまで待つ（`Engine` の drop から。2 回目以降は何もしない）
    pub fn shutdown(&self) {
        let Some(shared) = &self.inner else {
            return;
        };
        let Some(handle) = shared
            .worker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        // 満杯なら書き込みのスレッドが空けるまで待つ。スレッドがもうなければすぐ失敗する
        let _ = shared.tx.send(Msg::Stop);
        let _ = handle.join();
    }

    /// ここまでに入れた行が書かれて flush されるまで待つ（テストがファイルを読む前に呼ぶ）
    #[cfg(test)]
    pub(crate) fn flush_for_test(&self) {
        let Some(shared) = &self.inner else {
            return;
        };
        let (ack, done) = sync_channel(1);
        if shared.tx.send(Msg::Flush(ack)).is_ok() {
            let _ = done.recv();
        }
    }
}

impl Drop for EngineLog {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// 書き込みのスレッド。止まれと言われるか、送り手がすべてなくなるまで回る
fn run(mut writer: Writer, rx: &Receiver<Msg>, dropped: &AtomicU64) {
    writer.tidy(now_millis());
    loop {
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(Msg::Line(line)) => writer.write(&line, take_dropped(dropped)),
            Ok(Msg::Flush(ack)) => {
                writer.flush();
                let _ = ack.send(());
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => writer.flush(),
        }
        writer.flush_if_due();
    }
    // 止まれのあとに入った行も書く
    while let Ok(msg) = rx.try_recv() {
        match msg {
            Msg::Line(line) => writer.write(&line, take_dropped(dropped)),
            Msg::Flush(ack) => {
                writer.flush();
                let _ = ack.send(());
            }
            Msg::Stop => {}
        }
    }
    writer.flush();
}

fn take_dropped(dropped: &AtomicU64) -> u64 {
    if dropped.load(Ordering::Relaxed) == 0 {
        0
    } else {
        dropped.swap(0, Ordering::Relaxed)
    }
}

/// `engine.log` に書き、回し、回した世代を gzip にして、保持の上限を超えた分を消す（書き込みのスレッドだけが持つ）
struct Writer {
    dir: PathBuf,
    path: PathBuf,
    out: BufWriter<File>,
    size: u64,
    /// 次のローカルの日付が始まる時刻（ミリ秒）。これ以降の行を書く前に回す
    day_end: u64,
    /// まだ flush していない行があるか、と前の flush の時刻
    dirty: bool,
    last_flush: Instant,
    policy: Policy,
    line: String,
}

impl Writer {
    fn open(dir: &Path, policy: Policy) -> io::Result<Writer> {
        let path = dir.join(LOG_FILE_NAME);
        let (file, size) = open_append(&path)?;
        // 書きかけのログがあれば、その最後の更新の日付で日の境目を決める（前の日のものなら最初の行で回す）
        let since = if size > 0 {
            file.metadata()
                .and_then(|m| m.modified())
                .map_or_else(|_| now_millis(), system_time_millis)
        } else {
            now_millis()
        };
        Ok(Writer {
            dir: dir.to_path_buf(),
            path,
            out: BufWriter::with_capacity(BUFFER_BYTES, file),
            size,
            day_end: next_local_midnight(since),
            dirty: false,
            last_flush: Instant::now(),
            policy,
            line: String::new(),
        })
    }

    fn write(&mut self, line: &LogLine, dropped: u64) {
        if dropped > 0 {
            self.line.clear();
            use std::fmt::Write as _;
            let _ = writeln!(
                self.line,
                "{} log_dropped 0us lm={} err=- n={dropped}",
                line.millis, line.lm
            );
            self.put(line.millis);
        }
        self.line.clear();
        format_line(&mut self.line, line);
        self.put(line.millis);
    }

    /// `self.line` を書く。要れば先に回す
    fn put(&mut self, millis: u64) {
        let len = self.line.len() as u64;
        let new_day = millis >= self.day_end;
        if self.size > 0 && (new_day || self.size + len > self.policy.max_bytes) {
            self.rotate(millis);
        }
        if new_day {
            self.day_end = next_local_midnight(millis);
        }
        if self.out.write_all(self.line.as_bytes()).is_ok() {
            self.size += len;
            self.dirty = true;
        }
    }

    fn flush(&mut self) {
        let _ = self.out.flush();
        self.dirty = false;
        self.last_flush = Instant::now();
    }

    fn flush_if_due(&mut self) {
        if self.dirty && self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush();
        }
    }

    /// `engine.log` を回した世代の名前にして開き直し、gzip にして、保持の上限を超えた分を消す
    fn rotate(&mut self, millis: u64) {
        self.flush();
        let Some(target) = self.rotated_path(millis) else {
            // 名前が取れない（同じ秒に回しすぎた）。今のファイルに書き続け、次は上限ぶん書いてから試す
            self.size = 0;
            return;
        };
        if fs::rename(&self.path, &target).is_err() {
            self.size = 0;
            return;
        }
        match open_append(&self.path) {
            Ok((file, size)) => {
                self.out = BufWriter::with_capacity(BUFFER_BYTES, file);
                self.size = size;
            }
            Err(_) => {
                // 開き直せなければ、回した先のファイルに書き続ける（開いたままの File はそちらを指す）。
                // 書き続けるので gzip にしない（次に起動したときの後始末で gzip になる）
                self.size = 0;
                return;
            }
        }
        let _ = gzip(&target);
        self.retain(millis);
    }

    /// `engine-YYYYMMDD-HHMMSS.log`（ローカルの時刻）。`.log` か `.log.gz` がすでにあれば、1 秒ずつ後の時刻の
    /// 名前にする（名前の形を変えない。集計の `scripts/diag-report.py` がこの形だけを読み、名前の順が時刻の順のまま）
    fn rotated_path(&self, millis: u64) -> Option<PathBuf> {
        (0..MAX_SAME_SECOND).find_map(|n| {
            let stamp = local_stamp(millis.saturating_add(u64::from(n) * MILLIS_PER_SECOND));
            let name = format!("{ROTATED_PREFIX}{stamp}{ROTATED_SUFFIX}");
            let path = self.dir.join(&name);
            let gz = self.dir.join(format!("{name}{GZIP_SUFFIX}"));
            (!path.exists() && !gz.exists()).then_some(path)
        })
    }

    /// 起動のあとの後始末: gzip の書き途中を消し、gzip になっていない世代（以前の形も）を gzip にし、保持を当てる
    fn tidy(&mut self, now: u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut pending = Vec::new();
        let mut legacy = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let path = entry.path();
            if name.starts_with(ROTATED_PREFIX) && name.ends_with(PARTIAL_SUFFIX) {
                let _ = fs::remove_file(&path);
            } else if name.starts_with(ROTATED_PREFIX) && name.ends_with(ROTATED_SUFFIX) {
                pending.push(path);
            } else if name
                .strip_prefix(LEGACY_PREFIX)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            {
                legacy.push(path);
            }
        }
        for path in legacy {
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .map_or(now, system_time_millis);
            if let Some(target) = self.rotated_path(modified)
                && fs::rename(&path, &target).is_ok()
            {
                pending.push(target);
            }
        }
        for path in pending {
            let _ = gzip(&path);
        }
        self.retain(now);
    }

    /// 回した世代（`.log.gz`）を、古い順に、日数を超えたものと、合計（書き中を含む）が上限を超える分だけ消す
    fn retain(&self, now: u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut kept: Vec<(u64, PathBuf, u64)> = entries
            .flatten()
            .filter(|e| {
                e.file_name().to_str().is_some_and(|n| {
                    n.starts_with(ROTATED_PREFIX)
                        && n.ends_with(&format!("{ROTATED_SUFFIX}{GZIP_SUFFIX}"))
                })
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().map_or(0, system_time_millis);
                Some((modified, e.path(), meta.len()))
            })
            .collect();
        kept.sort();
        let oldest_kept = now.saturating_sub(self.policy.keep_days.saturating_mul(MILLIS_PER_DAY));
        let mut total: u64 = self.size + kept.iter().map(|(_, _, len)| *len).sum::<u64>();
        for (modified, path, len) in kept {
            if modified >= oldest_kept && total <= self.policy.keep_total_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

fn format_line(out: &mut String, line: &LogLine) {
    use std::fmt::Write as _;
    let err = line.error.unwrap_or("-");
    let _ = match &line.note {
        Some(note) => writeln!(
            out,
            "{} {} {}us lm={} err={err} {note}",
            line.millis, line.kind, line.micros, line.lm
        ),
        None => writeln!(
            out,
            "{} {} {}us lm={} err={err}",
            line.millis, line.kind, line.micros, line.lm
        ),
    };
}

fn open_append(path: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(ikari_core::user::PRIVATE_FILE_MODE)
        .open(path)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((file, size))
}

/// `src` を `src.gz` にして `src` を消す。書き途中は `.gz.tmp` に置き、終わってから rename する
fn gzip(src: &Path) -> io::Result<()> {
    let mut gz = src.as_os_str().to_owned();
    gz.push(GZIP_SUFFIX);
    let gz = PathBuf::from(gz);
    let mut partial = gz.as_os_str().to_owned();
    partial.push(PARTIAL_SUFFIX);
    let partial = PathBuf::from(partial);
    let written = (|| {
        let mut input = File::open(src)?;
        let mut encoder = GzEncoder::new(
            ikari_core::user::create_private_new(&partial)?,
            Compression::default(),
        );
        io::copy(&mut input, &mut encoder)?;
        encoder.finish()?;
        fs::rename(&partial, &gz)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&partial);
        return written;
    }
    fs::remove_file(src)
}

fn system_time_millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn zoned(millis: u64) -> Option<jiff::Zoned> {
    let ts = Timestamp::from_millisecond(i64::try_from(millis).ok()?).ok()?;
    Some(ts.to_zoned(TimeZone::system()))
}

/// `millis` の次のローカルの日付が始まる時刻（ミリ秒）。取れなければ 24 時間後
fn next_local_midnight(millis: u64) -> u64 {
    zoned(millis)
        .and_then(|z| {
            let next = z
                .date()
                .tomorrow()
                .ok()?
                .to_zoned(z.time_zone().clone())
                .ok()?;
            u64::try_from(next.timestamp().as_millisecond()).ok()
        })
        .unwrap_or_else(|| millis.saturating_add(MILLIS_PER_DAY))
}

fn local_stamp(millis: u64) -> String {
    zoned(millis).map_or_else(
        || millis.to_string(),
        |z| z.strftime(STAMP_FORMAT).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ikari-engine-log-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn log_path(dir: &Path) -> PathBuf {
        dir.join(LOG_DIR_NAME).join(LOG_FILE_NAME)
    }

    fn line(millis: u64, kind: &'static str) -> LogLine {
        LogLine {
            millis,
            kind,
            micros: 1,
            lm: "ready",
            error: None,
            note: None,
        }
    }

    /// `logs/` の中の、回した世代（`.log.gz`）の名前（昇順）
    fn rotated(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.join(LOG_DIR_NAME))
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n != LOG_FILE_NAME)
            .collect();
        names.sort();
        names
    }

    /// `engine-YYYYMMDD-HHMMSS.log.gz`（`scripts/diag-report.py` の `LOG_NAME` と同じ形）
    fn is_rotated_name(name: &str) -> bool {
        let Some(stamp) = name
            .strip_prefix(ROTATED_PREFIX)
            .and_then(|n| n.strip_suffix(".log.gz"))
        else {
            return false;
        };
        let b = stamp.as_bytes();
        b.len() == 15
            && b[8] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 8 || c.is_ascii_digit())
    }

    fn gunzip(path: &Path) -> String {
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(path).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn log_dir_and_file_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private");
        let log = EngineLog::open(&dir);
        log.record("convert", 1, "ready", None);
        log.flush_for_test();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join(LOG_DIR_NAME)), 0o700);
        assert_eq!(mode(&log_path(&dir)), 0o600);
        drop(log);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_one_line_per_record() {
        let dir = temp_dir("lines");
        let log = EngineLog::open(&dir);
        log.record("convert", 120, "ready", None);
        log.record("commit", 5, "ready", Some("unknown_conversion"));
        log.flush_for_test();
        let text = fs::read_to_string(log_path(&dir)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].contains(" convert 120us lm=ready err=-"),
            "{}",
            lines[0]
        );
        assert!(lines[1].ends_with("err=unknown_conversion"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn admin_lines_carry_the_request_kind_and_the_issue_name_only() {
        let dir = temp_dir("admin");
        let log = EngineLog::open(&dir);
        log.record("import_data", 4200, "disabled", Some("SCHEMA_UNSUPPORTED"));
        log.record("add_user_dict", 80, "disabled", None);
        log.flush_for_test();
        let text = fs::read_to_string(log_path(&dir)).unwrap();
        let fields: Vec<Vec<&str>> = text
            .lines()
            .map(|l| l.split_whitespace().collect())
            .collect();
        // 時刻、種類、所要時間、LM の状態、エラーの 5 つだけ
        assert!(fields.iter().all(|f| f.len() == 5), "{text}");
        assert_eq!(fields[0][1], "import_data");
        assert_eq!(fields[0][4], "err=SCHEMA_UNSUPPORTED");
        assert_eq!(fields[1][4], "err=-");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shutdown_writes_everything_that_was_queued() {
        let dir = temp_dir("shutdown");
        let log = EngineLog::open(&dir);
        for _ in 0..1000 {
            log.record_note("prefetch", 1, "ready", None, Some("ctx_rev=1"));
        }
        drop(log);
        let text = fs::read_to_string(log_path(&dir)).unwrap();
        assert_eq!(text.lines().count(), 1000);
        assert!(text.lines().all(|l| l.ends_with(" err=- ctx_rev=1")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rotates_by_size_into_gzip_generations_without_losing_lines() {
        let dir = temp_dir("rotate");
        let policy = Policy {
            max_bytes: 4096,
            ..POLICY
        };
        let log = EngineLog::open_with(&dir, policy, QUEUE_CAPACITY);
        let total = 1000;
        for _ in 0..total {
            log.record("prefetch", 1, "ready", None);
        }
        log.flush_for_test();
        let names = rotated(&dir);
        assert!(names.len() >= 2, "{names:?}");
        // 同じ秒に何度回しても `engine-YYYYMMDD-HHMMSS.log.gz` の形のまま（集計が読む形）
        assert!(names.iter().all(|n| is_rotated_name(n)), "{names:?}");
        let logs = dir.join(LOG_DIR_NAME);
        let mut lines = fs::read_to_string(logs.join(LOG_FILE_NAME))
            .unwrap()
            .lines()
            .count();
        for name in &names {
            let text = gunzip(&logs.join(name));
            assert!(text.len() as u64 <= policy.max_bytes, "{name}");
            assert!(
                text.lines()
                    .all(|l| l.contains(" prefetch 1us lm=ready err=-"))
            );
            lines += text.lines().count();
        }
        assert_eq!(lines, total);
        assert!(fs::metadata(logs.join(LOG_FILE_NAME)).unwrap().len() <= policy.max_bytes);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rotates_when_the_local_date_changes() {
        let dir = temp_dir("rotate-day");
        let logs = dir.join(LOG_DIR_NAME);
        fs::create_dir_all(&logs).unwrap();
        let mut w = Writer::open(&logs, POLICY).unwrap();
        let today = now_millis();
        w.write(&line(today, "convert"), 0);
        w.write(&line(today, "commit"), 0);
        // 次の日の最初の行で回す
        let tomorrow = next_local_midnight(today) + 1;
        w.write(&line(tomorrow, "convert"), 0);
        w.flush();
        let names = rotated(&dir);
        assert_eq!(names.len(), 1, "{names:?}");
        assert_eq!(names[0], format!("engine-{}.log.gz", local_stamp(tomorrow)));
        let old = gunzip(&logs.join(&names[0]));
        assert_eq!(old.lines().count(), 2, "{old}");
        let current = fs::read_to_string(logs.join(LOG_FILE_NAME)).unwrap();
        assert_eq!(current.lines().count(), 1);
        assert!(current.starts_with(&format!("{tomorrow} convert ")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn full_queue_drops_and_the_next_write_reports_the_count() {
        let dir = temp_dir("dropped");
        let logs = dir.join(LOG_DIR_NAME);
        fs::create_dir_all(&logs).unwrap();
        // 書き込みのスレッドをまだ動かさずにキューを満たす
        let (tx, rx) = sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut log = EngineLog {
            inner: Some(Shared {
                tx,
                dropped: Arc::clone(&dropped),
                worker: Mutex::new(None),
            }),
        };
        for _ in 0..5 {
            log.record("convert", 1, "ready", None);
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 3);
        let writer = Writer::open(&logs, POLICY).unwrap();
        let counter = Arc::clone(&dropped);
        let handle = std::thread::spawn(move || run(writer, &rx, &counter));
        if let Some(shared) = &mut log.inner {
            *shared.worker.lock().unwrap() = Some(handle);
        }
        // キューが空くのを待ってから次の行を入れる（満杯のままだと、これも捨てられる）
        log.flush_for_test();
        log.record("commit", 1, "ready", None);
        drop(log);
        let text = fs::read_to_string(logs.join(LOG_FILE_NAME)).unwrap();
        let kinds: Vec<&str> = text
            .lines()
            .map(|l| l.split_whitespace().nth(1).unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["log_dropped", "convert", "convert", "commit"],
            "{text}"
        );
        assert!(
            text.lines()
                .next()
                .unwrap()
                .ends_with(" 0us lm=ready err=- n=3")
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        let _ = fs::remove_dir_all(dir);
    }

    fn touch(path: &Path, bytes: usize, age_days: u64) {
        fs::write(path, vec![b'x'; bytes]).unwrap();
        let t = SystemTime::now() - Duration::from_millis(age_days * MILLIS_PER_DAY);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn retention_drops_generations_past_the_days_and_then_the_oldest_over_the_total() {
        let dir = temp_dir("retain");
        let logs = dir.join(LOG_DIR_NAME);
        fs::create_dir_all(&logs).unwrap();
        let policy = Policy {
            max_bytes: 4096,
            keep_days: 180,
            keep_total_bytes: 3000,
        };
        touch(&logs.join("engine-20260101-000000.log.gz"), 10, 200);
        touch(&logs.join("engine-20260601-000000.log.gz"), 1000, 100);
        touch(&logs.join("engine-20260701-000000.log.gz"), 1000, 90);
        touch(&logs.join("engine-20260801-000000.log.gz"), 1000, 80);
        touch(&logs.join("engine-20260901-000000.log.gz"), 500, 10);
        // 回した世代でないものは消さない
        touch(&logs.join("notes.txt"), 5000, 400);
        let w = Writer::open(&logs, policy).unwrap();
        w.retain(now_millis());
        assert_eq!(
            rotated(&dir),
            [
                "engine-20260701-000000.log.gz",
                "engine-20260801-000000.log.gz",
                "engine-20260901-000000.log.gz",
                "notes.txt",
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_compresses_leftovers_and_the_legacy_generations() {
        let dir = temp_dir("tidy");
        let logs = dir.join(LOG_DIR_NAME);
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("engine-20261001-120000.log"),
            "1 convert 1us lm=ready err=-\n",
        )
        .unwrap();
        fs::write(logs.join("engine-20261001-110000.log.gz.tmp"), "partial").unwrap();
        fs::write(logs.join("engine.log.1"), "2 commit 1us lm=ready err=-\n").unwrap();
        let log = EngineLog::open(&dir);
        log.flush_for_test();
        let names = rotated(&dir);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"engine-20261001-120000.log.gz".to_owned()));
        let legacy = names
            .iter()
            .find(|n| n.as_str() != "engine-20261001-120000.log.gz")
            .unwrap();
        assert!(is_rotated_name(legacy), "{legacy}");
        assert_eq!(gunzip(&logs.join(legacy)), "2 commit 1us lm=ready err=-\n");
        drop(log);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disabled_log_is_a_no_op() {
        EngineLog::disabled().record("convert", 1, "ready", None);
        EngineLog::disabled().shutdown();
        // 開けないディレクトリでも落ちない（普通のファイルの下は、root でも作れない）
        let file = tempfile::NamedTempFile::new().unwrap();
        EngineLog::open(&file.path().join("ikari-dir")).record("convert", 1, "ready", None);
    }
}
