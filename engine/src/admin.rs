//! 管理の要求（ADR 0005 §8）: ユーザー辞書、学習、エクスポート／インポート、属性プロファイル。
//!
//! - 管理の要求は `Inner::admin` の Mutex で 1 つずつ処理する。変換の経路（`convert`、`prefetch`）はこの
//!   Mutex を取らないので、管理の処理中も変換は止まらない。
//! - ファイルを書いたら、その場で Stack を組み立て直す（`DataWatch` の 1 秒を待たない）。自分で書いた
//!   ファイルの状態は `DataWatch` に記録して、二重に組み立て直さない。
//! - 失敗は、返事の中の `DataIssue` で返す（種類と、画面に出す説明）。`detail` と一覧には利用者のデータ
//!   （読み、表記、問題の行）が入るので、ログには `DataIssueKind` の名前だけを書く（NFR-19）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ikari_core::user::{
    EXPORT_SCHEMA, ExportIssue, ExportIssueKind, ImportSummary, LayerSel, MAX_LEARNING_ROWS,
    Profile, ProfileWeight, UserDataError, UserDictEntry, UserDictionary, UserDir, check_export,
    export_text, format_utc, import_text, pos_options, validate_export, verb_conj_types,
};
use ikari_proto::v1 as pb;
use ikari_proto::v1::{DataIssueKind, ErrorCode, envelope};
use ikari_stack::Reuse;

use crate::cache::{ConversionTable, lock as lock_mutex};
use crate::{DataWatch, Fail, Inner, MAX_ADMIN_REQUEST_BYTES};

/// データのディレクトリがない（`from_parts` で渡されていない）とき
const NO_DATA_DIR: Fail = Fail::new(ErrorCode::EngineNotReady, "no data directory");
/// ユーザー辞書のセクション名（エクスポートのセクションと同じ）
const USER_DICT_SECTION: &str = "user_dict";
/// バックアップを置くディレクトリ（データのディレクトリの下）
const BACKUP_DIR_NAME: &str = "backups";
/// バックアップのファイル名: `before-{import,reset}-<UTC>[-<n>].tsv`（`n` は同じ秒に重なったとき）
const BACKUP_PREFIX: &str = "before-";
const BACKUP_SUFFIX: &str = ".tsv";
/// 残すバックアップの数（新しいものから）。ただし中身のある最新の 1 つは、これより古くても消さない
const KEPT_BACKUPS: usize = 3;
/// 画面に出す固定の説明
const DETAIL_RELOAD_FAILED: &str = "saved, but reloading the dictionary failed";

/// バックアップを取れなかったとき（取れなければ、消す操作もしない）
const BACKUP_FAILED: Fail = Fail::new(ErrorCode::Internal, "backup failed");
// `ExportReply` と `ProfileReply` には失敗（`DataIssue`）を入れる場所がないので、`ErrorReply` で返す
const EXPORT_FAILED: Fail = Fail::new(ErrorCode::Internal, "export failed");
const EXPORT_TOO_LARGE: Fail = Fail::new(ErrorCode::Internal, "export too large");
const PROFILE_UNREADABLE: Fail = Fail::new(ErrorCode::Internal, "profile unreadable");
const PROFILE_WRITE_FAILED: Fail = Fail::new(ErrorCode::Internal, "profile write failed");

/// バックアップを取る理由（ファイル名に入る）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackupKind {
    Import,
    Reset,
}

impl BackupKind {
    const fn name(self) -> &'static str {
        match self {
            BackupKind::Import => "import",
            BackupKind::Reset => "reset",
        }
    }
}

/// `20261012T153045Z`。固定の幅なので、文字列の順が時刻の順になる。
pub(crate) fn utc_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // "YYYY-MM-DD HH:MM:SS" から区切りを除く
    let digits: String = format_utc(secs)
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    format!("{}T{}Z", &digits[..8], &digits[8..])
}

/// 同じ秒に 2 つ取っても、先のバックアップを上書きしない。通し番号は、その秒の最大の番号の次にする
/// （古いものが消えたあとに番号が戻ると、新しいものが古い順に並んで消されてしまう）。
fn unique_backup_path(backups: &Path, kind: BackupKind, stamp: &str) -> PathBuf {
    let next = list_backups(backups)
        .into_iter()
        .filter(|((s, _), _)| s == stamp)
        .map(|((_, n), _)| n + 1)
        .max()
        .unwrap_or(1);
    let counter = if next == 1 {
        String::new()
    } else {
        format!("-{next}")
    };
    backups.join(format!(
        "{BACKUP_PREFIX}{}-{stamp}{counter}{BACKUP_SUFFIX}",
        kind.name()
    ))
}

/// バックアップのファイル名から並べ替えの鍵（時刻、同じ秒の通し番号）を取る。バックアップでなければ None。
pub(crate) fn backup_key(file_name: &str) -> Option<(String, u32)> {
    let body = file_name
        .strip_prefix(BACKUP_PREFIX)?
        .strip_suffix(BACKUP_SUFFIX)?;
    let (kind, rest) = body.split_once('-')?;
    if ![BackupKind::Import, BackupKind::Reset]
        .iter()
        .any(|k| k.name() == kind)
    {
        return None;
    }
    let key = match rest.split_once('-') {
        Some((stamp, n)) => (stamp.to_owned(), n.parse().ok()?),
        None => (rest.to_owned(), 1),
    };
    (!key.0.is_empty()).then_some(key)
}

/// 鍵の昇順に並べたバックアップ。
fn list_backups(backups: &Path) -> Vec<((String, u32), PathBuf)> {
    let Ok(entries) = fs::read_dir(backups) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let key = backup_key(&e.file_name().to_string_lossy())?;
            Some((key, e.path()))
        })
        .collect();
    found.sort();
    found
}

fn newest_backup(dir: &UserDir) -> Option<PathBuf> {
    list_backups(&dir.root().join(BACKUP_DIR_NAME))
        .pop()
        .map(|(_, path)| path)
}

/// エクスポートの文章に中身がないか。分野・ユーザー辞書・学習・取り込み元の件数が、すべて 0 のとき空とする。
/// 読めない（検証に通らない）ものは、失いたくないので空とは見なさない。
fn is_empty_export(text: &str) -> bool {
    validate_export(text).is_ok_and(|s| {
        s.profile_domains + s.user_dict_entries + s.learning_rows + s.ingest_sources == 0
    })
}

/// 新しい `KEPT_BACKUPS` 個を残して消す。今書いた `keep` は、時計が戻っていても消さない。
/// 中身のある最新のバックアップも、数からあふれても消さない（空のバックアップに押し出されないため）。
fn prune_backups(backups: &Path, keep: &Path) {
    let others: Vec<PathBuf> = list_backups(backups)
        .into_iter()
        .map(|(_, path)| path)
        .filter(|path| path != keep)
        .collect();
    let newest_filled = others
        .iter()
        .rev()
        .find(|path| fs::read_to_string(path).map_or(true, |text| !is_empty_export(&text)))
        .cloned();
    let excess = others.len().saturating_sub(KEPT_BACKUPS - 1);
    for old in &others[..excess] {
        if Some(old) != newest_filled.as_ref() {
            let _ = fs::remove_file(old);
        }
    }
}

/// `DataIssue` の種類の名前（ログ用）。問題がなければ None。
pub(crate) fn issue_label(reply: &envelope::Body) -> Option<&'static str> {
    let issue = match reply {
        envelope::Body::UserDictListReply(r) => r.issue.as_ref(),
        envelope::Body::ImportReply(r) => r.issue.as_ref(),
        _ => None,
    }?;
    match DataIssueKind::try_from(issue.kind) {
        Ok(DataIssueKind::DataIssueNone) | Err(_) => None,
        Ok(kind) => Some(kind.as_str_name()),
    }
}

fn issue_kind(kind: ExportIssueKind) -> DataIssueKind {
    match kind {
        ExportIssueKind::NotIkariExport => DataIssueKind::NotIkariExport,
        ExportIssueKind::SchemaUnsupported => DataIssueKind::SchemaUnsupported,
        ExportIssueKind::Parse => DataIssueKind::Parse,
        ExportIssueKind::InvalidEntry => DataIssueKind::InvalidEntry,
        ExportIssueKind::Io => DataIssueKind::Io,
    }
}

/// core の分類を `DataIssue` にする。
fn data_issue(issue: ExportIssue) -> pb::DataIssue {
    pb::DataIssue {
        kind: issue_kind(issue.kind) as i32,
        section: issue.section.unwrap_or_default().to_owned(),
        line: u32::try_from(issue.line).unwrap_or(0),
        found_schema: issue.found_schema,
        supported_schema: issue.supported_schema,
        detail: issue.detail,
    }
}

fn simple_issue(kind: DataIssueKind, detail: &str) -> pb::DataIssue {
    pb::DataIssue {
        kind: kind as i32,
        detail: detail.to_owned(),
        ..pb::DataIssue::default()
    }
}

fn data_counts(summary: &ImportSummary) -> pb::DataCounts {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    pb::DataCounts {
        profile_domains: count(summary.profile_domains),
        user_dict_entries: count(summary.user_dict_entries),
        learning_rows: count(summary.learning_rows),
        ingest_sources: count(summary.ingest_sources),
    }
}

/// `profile.tsv` の `(分野, 重み)`。ファイルがなければ空。
fn read_profile_settings(dir: &UserDir) -> Result<Vec<(String, ProfileWeight)>, UserDataError> {
    match fs::read_to_string(dir.profile_tsv()) {
        Ok(text) => Profile::parse_settings(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn weight_kind(weight: ProfileWeight) -> pb::ProfileWeightKind {
    if weight == ProfileWeight::OFTEN {
        pb::ProfileWeightKind::ProfileWeightOften
    } else {
        pb::ProfileWeightKind::ProfileWeightSometimes
    }
}

fn pb_entry(e: &UserDictEntry) -> pb::UserDictEntry {
    pb::UserDictEntry {
        reading: e.reading.clone(),
        surface: e.surface.clone(),
        pos: e.pos.clone(),
        comment: e.comment.clone(),
    }
}

fn core_entry(e: pb::UserDictEntry) -> UserDictEntry {
    UserDictEntry {
        reading: e.reading,
        surface: e.surface,
        pos: e.pos,
        comment: e.comment,
    }
}

impl Inner {
    /// 管理の要求 1 件を処理する。
    pub(crate) fn admin(&self, body: envelope::Body) -> Result<envelope::Body, Fail> {
        let data_dir = self.data_dir.as_deref().ok_or(NO_DATA_DIR)?;
        let dir = UserDir::new(data_dir);
        let _serial = lock_mutex(&self.admin);
        match body {
            envelope::Body::ListUserDict(_) => Ok(self.list_user_dict(&dir)),
            envelope::Body::AddUserDict(add) => self.add_user_dict(&dir, add),
            envelope::Body::RemoveUserDict(remove) => Ok(self.remove_user_dict(&dir, &remove)),
            envelope::Body::LearningStatus(_) => Ok(self.learning_status(&dir, None)),
            envelope::Body::ResetLearning(reset) => self.reset_learning(&dir, &reset),
            envelope::Body::ExportData(_) => self.export_data(&dir, MAX_ADMIN_REQUEST_BYTES),
            envelope::Body::ImportData(import) => Ok(self.import_data(&dir, &import)),
            envelope::Body::GetProfile(_) => self.get_profile(&dir),
            envelope::Body::SetProfile(set) => self.set_profile(&dir, &set),
            // 管理でない種類は `dispatch` が渡さない
            _ => Err(Fail::bad("unexpected message")),
        }
    }

    // ---- ユーザー辞書 ----

    /// 辞書の一覧の返事。`entries` は変更後の一覧。
    fn user_dict_reply(
        &self,
        entries: &[UserDictEntry],
        issue: Option<pb::DataIssue>,
    ) -> envelope::Body {
        let stack = self.stack();
        envelope::Body::UserDictListReply(pb::UserDictListReply {
            entries: entries.iter().map(pb_entry).collect(),
            pos_options: pos_options()
                .into_iter()
                .map(|o| pb::PosOption {
                    name: o.name,
                    needs_conj_type: o.needs_conj_type,
                })
                .collect(),
            verb_conj_types: verb_conj_types(&*stack.grammar),
            issue,
        })
    }

    /// 読めない辞書ファイルの返事（一覧は空で、理由を issue に入れる）。
    fn user_dict_unreadable(&self, e: &UserDataError) -> envelope::Body {
        self.user_dict_failed(&UserDictionary::default(), e)
    }

    fn list_user_dict(&self, dir: &UserDir) -> envelope::Body {
        match UserDictionary::open(&dir.user_dict_tsv()) {
            Ok(dict) => self.user_dict_reply(dict.entries(), None),
            Err(e) => self.user_dict_unreadable(&e),
        }
    }

    fn add_user_dict(&self, dir: &UserDir, add: pb::UserDictAdd) -> Result<envelope::Body, Fail> {
        let entry = core_entry(add.entry.ok_or(Fail::bad("entry required"))?);
        let mut watch = self.lock_watch();
        let mut dict = match UserDictionary::open(&dir.user_dict_tsv()) {
            Ok(dict) => dict,
            Err(e) => return Ok(self.user_dict_unreadable(&e)),
        };
        // 読みの正規化と品詞の検査は 1 語だけで行い（正規化した形を得る）、grammar で展開できることも確かめる
        let mut probe = UserDictionary::default();
        let checked = probe
            .add(entry)
            .and_then(|()| probe.build(&*self.stack().grammar).map(|_| ()))
            .and_then(|()| dict.add(probe.entries()[0].clone()))
            .and_then(|()| dict.save(&dir.user_dict_tsv()));
        if let Err(e) = checked {
            // 書いていない（save が失敗したときは元のファイルのまま）。メモリ上の dict は捨てる
            let current = UserDictionary::open(&dir.user_dict_tsv()).unwrap_or_default();
            return Ok(self.user_dict_failed(&current, &e));
        }
        Ok(self.user_dict_changed(&dict, watch.as_deref_mut()))
    }

    fn remove_user_dict(&self, dir: &UserDir, remove: &pb::UserDictRemove) -> envelope::Body {
        let mut watch = self.lock_watch();
        let mut dict = match UserDictionary::open(&dir.user_dict_tsv()) {
            Ok(dict) => dict,
            Err(e) => return self.user_dict_unreadable(&e),
        };
        if !dict.remove(&remove.reading, &remove.surface) {
            let issue = simple_issue(DataIssueKind::NotFound, "");
            return self.user_dict_reply(dict.entries(), Some(issue));
        }
        if let Err(e) = dict.save(&dir.user_dict_tsv()) {
            let current = UserDictionary::open(&dir.user_dict_tsv()).unwrap_or_default();
            return self.user_dict_failed(&current, &e);
        }
        self.user_dict_changed(&dict, watch.as_deref_mut())
    }

    /// 追加や削除が失敗した返事。`current` はファイルに今ある辞書。
    fn user_dict_failed(&self, current: &UserDictionary, e: &UserDataError) -> envelope::Body {
        let issue = ExportIssue::from_error(Some(USER_DICT_SECTION), e);
        self.user_dict_reply(current.entries(), Some(data_issue(issue)))
    }

    /// 辞書ファイルを書いたあとの組み立て直しと、返事。
    fn user_dict_changed(
        &self,
        dict: &UserDictionary,
        watch: Option<&mut DataWatch>,
    ) -> envelope::Body {
        let reuse = Reuse::from_stack(&self.stack());
        let issue = (!self.reload_after_write(watch, &reuse))
            .then(|| simple_issue(DataIssueKind::Io, DETAIL_RELOAD_FAILED));
        self.user_dict_reply(dict.entries(), issue)
    }

    // ---- 学習 ----

    /// 学習の件数の返事。`backup` は今作ったバックアップ（なければ、今ある一番新しいもの）。
    fn learning_status(&self, dir: &UserDir, backup: Option<&Path>) -> envelope::Body {
        let (usage, initial) = self
            .stack()
            .learning
            .as_ref()
            .map_or((0, 0), |store| store.layer_counts());
        let backup = backup.map(Path::to_path_buf).or_else(|| newest_backup(dir));
        envelope::Body::LearningStatusReply(pb::LearningStatusReply {
            usage_rows: u32::try_from(usage).unwrap_or(u32::MAX),
            initial_rows: u32::try_from(initial).unwrap_or(u32::MAX),
            max_rows: u32::try_from(MAX_LEARNING_ROWS).unwrap_or(u32::MAX),
            backup_file: backup.map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
        })
    }

    /// 層ごとのリセット。先にバックアップを取り（取れなければ何も消さない）、消したらすぐ保存する。
    fn reset_learning(
        &self,
        dir: &UserDir,
        reset: &pb::LearningReset,
    ) -> Result<envelope::Body, Fail> {
        let layer = match pb::LearningLayer::try_from(reset.layer) {
            Ok(pb::LearningLayer::Usage) => LayerSel::Usage,
            Ok(pb::LearningLayer::Initial) => LayerSel::Initial,
            Ok(pb::LearningLayer::All) => LayerSel::All,
            _ => return Err(Fail::bad("learning layer required")),
        };
        let stack = self.stack();
        let Some(store) = &stack.learning else {
            return Ok(self.learning_status(dir, None));
        };
        let backup = self
            .write_backup(dir, BackupKind::Reset)
            .map_err(|_| BACKUP_FAILED)?;
        {
            // 保存の Mutex を持ったまま消して書く（タイマーの保存と重ならない）
            let mut save = lock_mutex(&self.save);
            store.reset(layer);
            match store.save() {
                Ok(()) => {
                    save.dirty = false;
                    save.last_save = Instant::now();
                }
                // メモリは消えている。次の保存で書き直す
                Err(_) => save.dirty = true,
            }
        }
        // リセット前の学習で並べた先行計算は使わない
        self.discard_prefetched();
        let mut status = self.learning_status(dir, backup.as_deref());
        if backup.is_none()
            && let envelope::Body::LearningStatusReply(r) = &mut status
        {
            // 取らなかったので、画面に「保存しました」と出さない
            r.backup_file.clear();
        }
        Ok(status)
    }

    // ---- エクスポートとインポート ----

    /// 学習を保存してから（メモリの最新を含めるため）、ユーザーデータ全体をバイト列にして返す。
    /// `limit` を超えるときは `ErrorReply`（`ExportReply` には失敗を入れる場所がない）。
    pub(crate) fn export_data(&self, dir: &UserDir, limit: usize) -> Result<envelope::Body, Fail> {
        self.save_now();
        let text = export_text(dir).map_err(|_| EXPORT_FAILED)?;
        if text.len() > limit {
            return Err(EXPORT_TOO_LARGE);
        }
        let summary = validate_export(&text).map_err(|_| EXPORT_FAILED)?;
        Ok(envelope::Body::ExportReply(pb::ExportReply {
            content: text.into_bytes(),
            schema: EXPORT_SCHEMA,
            counts: Some(data_counts(&summary)),
        }))
    }

    /// エクスポートのファイルで、ユーザーデータを上書きする（マージしない）。
    /// 検証に失敗したら何も変えない。`dry_run` は検証して件数を返すだけ。
    fn import_data(&self, dir: &UserDir, import: &pb::ImportData) -> envelope::Body {
        let rejected = |issue: pb::DataIssue| {
            envelope::Body::ImportReply(pb::ImportReply {
                issue: Some(issue),
                ..pb::ImportReply::default()
            })
        };
        let Ok(text) = std::str::from_utf8(&import.content) else {
            return rejected(simple_issue(DataIssueKind::Parse, "not valid UTF-8"));
        };
        let grammar = Arc::clone(&self.stack().grammar);
        let summary = match check_export(text, Some(&*grammar)) {
            Ok(summary) => summary,
            Err(issue) => return rejected(data_issue(issue)),
        };
        let counts = Some(data_counts(&summary));
        if import.dry_run {
            return envelope::Body::ImportReply(pb::ImportReply {
                counts,
                ..pb::ImportReply::default()
            });
        }

        let mut watch = self.lock_watch();
        // 戻せるように、先に今の状態を書く。書けなければ上書きしない
        let backup = match self.write_backup(dir, BackupKind::Import) {
            Ok(path) => path,
            Err(_) => return rejected(simple_issue(DataIssueKind::Io, "backup failed")),
        };
        // 取らなかったとき（空、または直前と同じ）は空の文字列（画面は保存先を出さない）
        let backup_file = backup.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
        // 保存の Mutex を持ったまま、ファイルの差し替えから学習の読み直しまでを行う。
        // 古いメモリが、差し替えたファイルを保存で上書きしないようにするため
        let mut save = lock_mutex(&self.save);
        if let Err(e) = import_text(dir, text) {
            return rejected(data_issue(ExportIssue::from_error(None, &e)));
        }
        let reuse = Reuse::without_learning(&self.stack());
        let reloaded = self.reload_after_write(watch.as_deref_mut(), &reuse);
        save.dirty = false;
        save.last_save = Instant::now();
        drop(save);
        // 読み直し前に始めた変換の確定は、学習されない（UNKNOWN_CONVERSION）
        *lock_mutex(&self.conversions) = ConversionTable::default();
        envelope::Body::ImportReply(pb::ImportReply {
            applied: true,
            counts,
            issue: (!reloaded).then(|| simple_issue(DataIssueKind::Io, DETAIL_RELOAD_FAILED)),
            backup_file,
        })
    }

    // ---- 属性プロファイル ----

    /// 同梱のプリセットすべてと、今の重み（`profile.tsv` にないものは OFF）。
    fn get_profile(&self, dir: &UserDir) -> Result<envelope::Body, Fail> {
        let current = read_profile_settings(dir).map_err(|_| PROFILE_UNREADABLE)?;
        let weight_of = |id: &str| {
            current
                .iter()
                .find(|(d, _)| d == id)
                .map_or(pb::ProfileWeightKind::ProfileWeightOff, |(_, w)| {
                    weight_kind(*w)
                })
        };
        Ok(envelope::Body::ProfileReply(pb::ProfileReply {
            domains: Profile::presets()
                .into_iter()
                .map(|d| pb::ProfileDomainState {
                    weight: weight_of(&d.id) as i32,
                    id: d.id,
                    name: d.name,
                })
                .collect(),
        }))
    }

    /// 分野の重みを全体で置き換える（OFF は書かない）。書いたらすぐ組み立て直す。
    fn set_profile(&self, dir: &UserDir, set: &pb::ProfileSet) -> Result<envelope::Body, Fail> {
        let mut settings: Vec<(String, ProfileWeight)> = Vec::new();
        for domain in &set.domains {
            let weight = match pb::ProfileWeightKind::try_from(domain.weight) {
                Ok(pb::ProfileWeightKind::ProfileWeightOff) => continue,
                Ok(pb::ProfileWeightKind::ProfileWeightSometimes) => ProfileWeight::SOMETIMES,
                Ok(pb::ProfileWeightKind::ProfileWeightOften) => ProfileWeight::OFTEN,
                _ => return Err(Fail::bad("profile weight required")),
            };
            if settings.iter().any(|(id, _)| *id == domain.id) {
                return Err(Fail::bad("duplicate profile domain"));
            }
            settings.push((domain.id.clone(), weight));
        }
        // 未知の分野は、ここで断る（書かない）
        let profile = Profile::from_settings(&settings).map_err(|_| Fail::bad("unknown domain"))?;
        let mut watch = self.lock_watch();
        profile.save(dir).map_err(|_| PROFILE_WRITE_FAILED)?;
        let reuse = Reuse::from_stack(&self.stack());
        // 組み立て直しに失敗しても、書いたものは残る（次の `refresh_data` がもう一度試す）
        self.reload_after_write(watch.as_deref_mut(), &reuse);
        self.get_profile(dir)
    }

    // ---- バックアップ ----

    /// 今のユーザーデータをエクスポートの形式で `backups/` に書き、古いものを消す（新しい 3 つと、中身のある最新の 1 つを残す）。
    /// メモリの最新を含めるため、先に学習を保存する。書いたファイルのパスを返す。
    /// 取る意味がないときは書かずに None: 中身が空（`is_empty_export`）のとき、または一番新しいバックアップと
    /// 内容が同じとき。消す操作を続けても、中身のあるバックアップが空のものに押し出されない。
    fn write_backup(
        &self,
        dir: &UserDir,
        kind: BackupKind,
    ) -> Result<Option<PathBuf>, UserDataError> {
        self.save_now();
        let text = export_text(dir)?;
        if is_empty_export(&text) {
            return Ok(None);
        }
        let backups = dir.root().join(BACKUP_DIR_NAME);
        if let Some(newest) = newest_backup(dir)
            && fs::read(&newest).is_ok_and(|prev| prev == text.as_bytes())
        {
            return Ok(None);
        }
        fs::create_dir_all(&backups)?;
        let path = unique_backup_path(&backups, kind, &utc_stamp());
        let tmp = path.with_extension("tsv.tmp");
        let written = fs::write(&tmp, text.as_bytes()).and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        prune_backups(&backups, &path);
        Ok(Some(path))
    }

    // ---- 共通 ----

    /// `DataWatch` を持つ。組み立て直しの確認（`refresh_data`）は待たずに諦めるが、管理の要求は待つ。
    /// 持っている間は、`refresh_data` が同じ変更で二重に組み立て直さない。
    fn lock_watch(&self) -> Option<MutexGuard<'_, DataWatch>> {
        self.watch.as_ref().map(lock_mutex)
    }

    /// ファイルを書いたあとに Stack を組み立て直し、成功したら `DataWatch` に自分で書いたことを記録する。
    /// 組み立て直しの仕組みがなければ（`rebuild == None`）何もしないで true。失敗なら false
    /// （記録しないので、次の `refresh_data` がもう一度試す）。
    fn reload_after_write(&self, watch: Option<&mut DataWatch>, reuse: &Reuse) -> bool {
        let Some(rebuild) = &self.rebuild else {
            return true;
        };
        let ok = self.rebuild_stack(rebuild, reuse);
        if ok && let Some(watch) = watch {
            watch.resync();
        }
        ok
    }
}
