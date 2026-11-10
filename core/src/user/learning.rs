//! 文脈付きの学習の保存先（ADR 0002 §8.2, §8.3）。
//!
//! 保存するのは (読み, 表記, 文脈の特徴量のハッシュ) ごとの回数だけで、左文脈の生テキストは持たない。
//! 文として残りうるのは、利用者が 1 回の変換で確定した候補（`phrase`、読みが
//! [`MAX_READING_CHARS`] 文字以下のものだけ）と、そのセグメントに限る。
//! 取り込み（`initial` 層）は 1 語ずつの `word` 記録だけを作り、文は持たない（NFR-19）。
//! 特徴量のプライバシー上の性質は [`super::features`] のモジュール doc を参照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use ikari_dict_format::MAX_READING_CHARS;
use ikari_dict_format::reading::normalize_key;

use super::features::extract_named;
use super::util::{
    check_schema, fnv1a64, fnv1a64_continue, has_cell_breaker, parse_error, parse_header,
    read_optional, write_atomic,
};
use super::{ContextFeatures, RecentChoices, RecentClock, UserDataError};
use crate::candidate::{CandidateOrigin, Segment};
use crate::converter::CommitEvent;
use crate::dict::Grammar;
use crate::pos::is_content_word;
use crate::vectors::WordVectors;

pub const LEARNING_SCHEMA: u32 = 1;
/// 保存する行数の上限。超えたら `last_day` の古いものから捨てる。
pub const MAX_LEARNING_ROWS: usize = 200_000;
/// Rewriter の候補は表記ではなく書式を学習する（`@rw:<name>:<format>`）。
pub const REWRITER_KEY_PREFIX: &str = "@rw:";

const HEADER_NAME: &str = "#ikari-learning";
const COLUMNS: &str = "layer\tsource\tunit\treading\tsurface\tsegments\tfeature\tcount\tlast_day";
const COLUMN_COUNT: usize = 9;
const NO_SOURCE: &str = "-";
const USAGE_SOURCE: u32 = 0;
const SEGMENT_JOINER: &str = " + ";
const SEGMENT_FIELD_SEPARATOR: &str = "/";
const UNKNOWN_POS_NAME: &str = "*";
const SALT_HEX_DIGITS: usize = 16;

/// 学習の層
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerSel {
    Usage,
    Initial,
    All,
}

/// 記録の層（`LayerSel::All` は選択だけで、記録の層ではない）
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Usage,
    Initial,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Usage => "usage",
            Layer::Initial => "initial",
        }
    }
}

/// 記録の単位。`phrase` は確定した候補全体、`word` はセグメント。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    Phrase,
    Word,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Phrase => "phrase",
            Unit::Word => "word",
        }
    }
}

/// 保存するセグメント（語IDは保存しない。文字列で持つ）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentKey {
    pub surface: String,
    pub reading: String,
    pub pos_name: String,
}

/// 学習の 1 行（読み出し用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningRecord {
    pub layer: Layer,
    /// `initial` の取り込み元の番号。`usage` は None
    pub source: Option<u32>,
    pub unit: Unit,
    pub reading: String,
    pub surface: String,
    pub segments: String,
    pub feature: u64,
    pub count: u32,
    pub last_day: u32,
}

/// 並び順は読みが先頭（読みで範囲検索するため）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    reading: String,
    unit: Unit,
    feature: u64,
    layer: Layer,
    source: u32,
    surface: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stat {
    segments: String,
    count: u32,
    last_day: u32,
}

type Rows = BTreeMap<Key, Stat>;

/// 確定を記録するときの重み（ADR 0057）。ファイルの形式は変えず、`count` を何回分増やすかだけを決める。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordPolicy {
    /// 1 位以外を選んだ確定（人間の意図が明らか）の回数の重み
    pub explicit_weight: u32,
    /// 1 位のまま確定したもの（慌てて確定したかもしれない）の回数の重み。0 なら記録しない
    pub top_weight: u32,
    /// 1 位の確定で、記録がすでにあるときに `last_day` だけを更新する（`top_weight` が 0 のときの忘却の起点）
    pub top_refresh: bool,
}

impl Default for RecordPolicy {
    fn default() -> Self {
        Self {
            explicit_weight: 4,
            top_weight: 1,
            top_refresh: true,
        }
    }
}

/// 学習の記録。メモリに全件を持ち、`save` で TSV に書く（一時ファイル + rename）。
pub struct LearningStore {
    policy: RwLock<RecordPolicy>,
    path: PathBuf,
    salt: u64,
    rows: RwLock<Rows>,
    grammar: OnceLock<Arc<dyn Grammar>>,
    /// 同じ読みで直前に選んだ表記（保存しない。ADR 0045）
    recent: RecentChoices,
}

impl LearningStore {
    /// ファイルがなければ空で作る（ソルトを生成する。ファイルは `save` で作られる）。
    /// ファイルが不正なら何も返さずエラー（既存のファイルは変更しない）。
    pub fn open(path: &Path) -> Result<Self, UserDataError> {
        Self::open_with_recent_clock(path, RecentClock::System)
    }

    /// `open` と同じで、直前の選択の時計を差し替える（eval とテスト用）。
    pub fn open_with_recent_clock(path: &Path, clock: RecentClock) -> Result<Self, UserDataError> {
        let (salt, rows) = match read_optional(path)? {
            Some(text) => parse(&text, path)?,
            None => (new_salt(), Rows::new()),
        };
        Ok(Self {
            policy: RwLock::new(RecordPolicy::default()),
            path: path.to_path_buf(),
            salt,
            rows: RwLock::new(rows),
            grammar: OnceLock::new(),
            recent: RecentChoices::new(clock),
        })
    }

    /// `open` と同じで、ファイルが読めない・不正なときは、そのファイルを `<名前>.bad`（あれば `.bad.1`、`.2` …）へ
    /// 移し（rename。中身も権限も変えない）、空の学習で開く。壊れた学習ファイルで日本語が打てなくなるのを避けるため。
    /// 移した先を 2 つ目の値で返す（正常なら None）。移せなかったときは、元のエラーを返して何も変えない（データを失わない側）。
    pub fn open_or_quarantine(path: &Path) -> Result<(Self, Option<PathBuf>), UserDataError> {
        match Self::open(path) {
            Ok(store) => Ok((store, None)),
            Err(err) => {
                let moved = quarantine(path).map_err(|_| err)?;
                Ok((Self::open(path)?, Some(moved)))
            }
        }
    }

    /// 学習ファイルの中身を検証する（エクスポートのインポートの事前検証用）。
    pub fn validate_text(text: &str) -> Result<(), UserDataError> {
        parse(text, Path::new(LEARNING_SECTION_NAME)).map(|_| ())
    }

    /// 品詞名を引く文法を結びつける。結びつけると、特徴量の `Prev` が品詞1（`名詞` など）で作られ、
    /// 保存するセグメントに品詞名が入り、`word` 記録が内容語に限られる。
    /// 辞書を作り直しても学習が壊れないように、本番では必ず呼ぶ。最初の 1 回だけ有効。
    pub fn attach_grammar(&self, grammar: Arc<dyn Grammar>) {
        let _ = self.grammar.set(grammar);
    }

    /// 結びつけた文法。`LearningRanker` も同じものを使い、記録と加点で特徴量が食い違わないようにする。
    pub fn grammar(&self) -> Option<&dyn Grammar> {
        self.grammar.get().map(|g| &**g)
    }

    /// 記録の重み（既定は `RecordPolicy::default`）。
    pub fn record_policy(&self) -> RecordPolicy {
        *self.policy.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 記録の重みを変える（設定から。`stack` が組み立てのときに呼ぶ）。
    pub fn set_record_policy(&self, policy: RecordPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// 確定を `usage` 層に記録する。secure の確定は `Conversion::commit` が None を返すので、ここには来ない。
    pub fn record(&self, ev: &CommitEvent, vectors: Option<&dyn WordVectors>, today: u32) {
        let feats = extract_named(&ev.analysis, self.grammar(), vectors, self.salt);
        let reading = normalize_key(&ev.reading);
        if reading.is_empty() {
            return;
        }
        let cand = &ev.candidate;
        let policy = self.record_policy();
        let (weight, refresh) = if ev.explicit {
            (policy.explicit_weight, false)
        } else {
            (policy.top_weight, policy.top_refresh)
        };
        if weight == 0 && !refresh {
            return;
        }
        let mut rows = self.write();

        let (surface_key, segments_text) = match cand.origin {
            CandidateOrigin::Rewriter { name, format } => {
                let key = rewriter_key(name, format);
                (key.clone(), key)
            }
            _ => (cand.surface.clone(), self.segments_text(&cand.segments)),
        };
        if has_cell_breaker(&surface_key) || has_cell_breaker(&segments_text) {
            return;
        }
        if reading.chars().count() <= MAX_READING_CHARS {
            self.recent.note(&reading, &surface_key);
            for f in &feats.features {
                bump(
                    &mut rows,
                    Key {
                        reading: reading.clone(),
                        unit: Unit::Phrase,
                        feature: f.hash,
                        layer: Layer::Usage,
                        source: USAGE_SOURCE,
                        surface: surface_key.clone(),
                    },
                    &segments_text,
                    today,
                    weight,
                    refresh,
                );
            }
        }

        let is_rewriter = matches!(cand.origin, CandidateOrigin::Rewriter { .. });
        if !is_rewriter && cand.segments.len() > 1 {
            for seg in &cand.segments {
                if !self.is_recordable_word(seg) {
                    continue;
                }
                let seg_reading = normalize_key(&seg.reading);
                let text = self.segments_text(std::slice::from_ref(seg));
                if seg_reading.is_empty()
                    || has_cell_breaker(&seg.surface)
                    || has_cell_breaker(&text)
                {
                    continue;
                }
                for f in &feats.features {
                    bump(
                        &mut rows,
                        Key {
                            reading: seg_reading.clone(),
                            unit: Unit::Word,
                            feature: f.hash,
                            layer: Layer::Usage,
                            source: USAGE_SOURCE,
                            surface: seg.surface.clone(),
                        },
                        &text,
                        today,
                        weight,
                        refresh,
                    );
                }
            }
        }
    }

    /// 取り込みなどからの `initial` 層の記録。`segs` が 1 つなら `word`、複数なら連結した `phrase`。
    /// 特徴量ごとに 1 行を作る。表記や読みにタブ・改行があるものは無視する。
    pub fn add_initial(
        &self,
        source: u32,
        reading: &str,
        segs: &[SegmentKey],
        feats: &ContextFeatures,
        today: u32,
    ) {
        let reading = normalize_key(reading);
        if reading.is_empty() || segs.is_empty() {
            return;
        }
        let (unit, surface) = if segs.len() == 1 {
            (Unit::Word, segs[0].surface.clone())
        } else {
            (
                Unit::Phrase,
                segs.iter().map(|s| s.surface.as_str()).collect::<String>(),
            )
        };
        let text = segs
            .iter()
            .map(segment_text)
            .collect::<Vec<_>>()
            .join(SEGMENT_JOINER);
        if surface.is_empty()
            || has_cell_breaker(&reading)
            || has_cell_breaker(&surface)
            || has_cell_breaker(&text)
        {
            return;
        }
        let mut rows = self.write();
        for f in &feats.features {
            bump(
                &mut rows,
                Key {
                    reading: reading.clone(),
                    unit,
                    feature: f.hash,
                    layer: Layer::Initial,
                    source,
                    surface: surface.clone(),
                },
                &text,
                today,
                1,
                false,
            );
        }
    }

    pub fn reset(&self, layer: LayerSel) {
        if matches!(layer, LayerSel::All | LayerSel::Usage) {
            self.recent.clear();
        }
        let mut rows = self.write();
        match layer {
            LayerSel::All => rows.clear(),
            LayerSel::Usage => rows.retain(|k, _| k.layer != Layer::Usage),
            LayerSel::Initial => rows.retain(|k, _| k.layer != Layer::Initial),
        }
    }

    /// 取り込み元の `initial` 記録を消す（NFR-26）。
    pub fn remove_source(&self, source: u32) {
        self.write()
            .retain(|k, _| !(k.layer == Layer::Initial && k.source == source));
    }

    /// 行数の上限を守って保存する（一時ファイル + rename）。
    pub fn save(&self) -> Result<(), UserDataError> {
        let text = {
            let mut rows = self.write();
            prune(&mut rows, MAX_LEARNING_ROWS);
            serialize(self.salt, &rows)
        };
        write_atomic(&self.path, &text)
    }

    pub fn salt(&self) -> u64 {
        self.salt
    }

    /// ファイルの中身（ヘッダ付き）。
    pub fn to_text(&self) -> String {
        serialize(self.salt, &self.read())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 同じ読みで直前に選んだ表記（ADR 0045）
    pub fn recent(&self) -> &RecentChoices {
        &self.recent
    }

    /// 読みが一致する行（加点の計算用）。
    pub fn lookup(&self, reading: &str) -> Vec<LearningRecord> {
        let rows = self.read();
        let lower = Key {
            reading: reading.to_string(),
            unit: Unit::Phrase,
            feature: 0,
            layer: Layer::Usage,
            source: 0,
            surface: String::new(),
        };
        rows.range(lower..)
            .take_while(|(k, _)| k.reading == reading)
            .map(|(k, v)| record_of(k, v))
            .collect()
    }

    /// 全行（`learn show` 用。読みの順）。
    pub fn records(&self) -> Vec<LearningRecord> {
        self.read().iter().map(|(k, v)| record_of(k, v)).collect()
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// (usage の行数, initial の行数)
    pub fn layer_counts(&self) -> (usize, usize) {
        let rows = self.read();
        let usage = rows.keys().filter(|k| k.layer == Layer::Usage).count();
        (usage, rows.len() - usage)
    }

    fn read(&self) -> RwLockReadGuard<'_, Rows> {
        self.rows
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Rows> {
        self.rows
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn pos_name(&self, pos_id: u16) -> String {
        self.grammar()
            .and_then(|g| g.pos_name(pos_id))
            .unwrap_or(UNKNOWN_POS_NAME)
            .to_string()
    }

    fn segments_text(&self, segs: &[Segment]) -> String {
        segs.iter()
            .map(|s| {
                segment_text(&SegmentKey {
                    surface: s.surface.clone(),
                    reading: s.reading.clone(),
                    pos_name: self.pos_name(s.pos_id),
                })
            })
            .collect::<Vec<_>>()
            .join(SEGMENT_JOINER)
    }

    /// 文法があれば内容語だけ。なければ（品詞が分からないので）未知語でないセグメントを全部。
    fn is_recordable_word(&self, seg: &Segment) -> bool {
        match self.grammar().and_then(|g| g.pos_name(seg.pos_id)) {
            Some(name) => is_content_word(name),
            None => seg.word.is_some(),
        }
    }
}

pub const LEARNING_SECTION_NAME: &str = "learning";

/// Rewriter の候補の保存用キー。
pub fn rewriter_key(name: &str, format: u16) -> String {
    format!("{REWRITER_KEY_PREFIX}{name}:{format}")
}

fn segment_text(s: &SegmentKey) -> String {
    [s.surface.as_str(), s.reading.as_str(), s.pos_name.as_str()].join(SEGMENT_FIELD_SEPARATOR)
}

/// `weight` 回分の確定を数える。`weight` が 0 のときは回数を増やさず、`refresh` なら既にある行の
/// `last_day` だけを更新する（忘却の起点を新しくする。行は作らない）
fn bump(rows: &mut Rows, key: Key, segments: &str, today: u32, weight: u32, refresh: bool) {
    if weight == 0 {
        if refresh && let Some(stat) = rows.get_mut(&key) {
            stat.last_day = stat.last_day.max(today);
        }
        return;
    }
    let stat = rows.entry(key).or_insert_with(|| Stat {
        segments: segments.to_string(),
        count: 0,
        last_day: today,
    });
    stat.count = stat.count.saturating_add(weight);
    stat.last_day = stat.last_day.max(today);
}

/// `last_day` の古い行から捨てて `max` 行にする（同じ日ならキーの順）。
fn prune(rows: &mut Rows, max: usize) {
    if rows.len() <= max {
        return;
    }
    let mut order: Vec<(u32, &Key)> = rows.iter().map(|(k, v)| (v.last_day, k)).collect();
    order.sort();
    let drop: Vec<Key> = order[..rows.len() - max]
        .iter()
        .map(|(_, k)| (*k).clone())
        .collect();
    for k in drop {
        rows.remove(&k);
    }
}

fn record_of(k: &Key, v: &Stat) -> LearningRecord {
    LearningRecord {
        layer: k.layer,
        source: (k.layer == Layer::Initial).then_some(k.source),
        unit: k.unit,
        reading: k.reading.clone(),
        surface: k.surface.clone(),
        segments: v.segments.clone(),
        feature: k.feature,
        count: v.count,
        last_day: v.last_day,
    }
}

fn serialize(salt: u64, rows: &Rows) -> String {
    let mut out = format!("{HEADER_NAME}\tschema={LEARNING_SCHEMA}\tsalt={salt:016x}\n{COLUMNS}\n");
    for (k, v) in rows {
        let source = match k.layer {
            Layer::Usage => NO_SOURCE.to_string(),
            Layer::Initial => k.source.to_string(),
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\n",
            k.layer.as_str(),
            source,
            k.unit.as_str(),
            k.reading,
            k.surface,
            v.segments,
            k.feature,
            v.count,
            v.last_day
        ));
    }
    out
}

/// 退避先の拡張子（`learning.tsv` → `learning.tsv.bad`）
const QUARANTINE_SUFFIX: &str = "bad";

/// `path` を同じフォルダの `.bad`（あれば `.bad.1`、`.2` …。上限なし。上書きしない）へ移す。移した先を返す。
fn quarantine(path: &Path) -> std::io::Result<PathBuf> {
    let base = path.as_os_str().to_owned();
    let with = |suffix: String| {
        let mut s = base.clone();
        s.push(".");
        s.push(suffix);
        PathBuf::from(s)
    };
    let mut candidate = with(QUARANTINE_SUFFIX.to_owned());
    let mut n: u64 = 0;
    while std::fs::symlink_metadata(&candidate).is_ok() {
        n += 1;
        candidate = with(format!("{QUARANTINE_SUFFIX}.{n}"));
    }
    std::fs::rename(path, &candidate)?;
    Ok(candidate)
}

fn parse(text: &str, file: &Path) -> Result<(u64, Rows), UserDataError> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| parse_error(file, 1, "空のファイルです"))?;
    let kv = parse_header(header, HEADER_NAME)
        .ok_or_else(|| parse_error(file, 1, format!("ヘッダが `{HEADER_NAME}` ではありません")))?;
    check_schema(file, &kv, LEARNING_SCHEMA)?;
    let salt_text = kv
        .iter()
        .find(|(k, _)| *k == "salt")
        .map(|(_, v)| *v)
        .ok_or_else(|| parse_error(file, 1, "ヘッダに salt=<16桁の16進> がありません"))?;
    let salt = parse_hex64(salt_text).ok_or_else(|| parse_error(file, 1, "salt が不正です"))?;
    if lines.next() != Some(COLUMNS) {
        return Err(parse_error(file, 2, "列の見出し行が不正です"));
    }
    let mut rows = Rows::new();
    for (i, line) in lines.enumerate() {
        let n = i + 3;
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != COLUMN_COUNT {
            return Err(parse_error(
                file,
                n,
                format!("列数が {COLUMN_COUNT} ではありません ({})", f.len()),
            ));
        }
        let layer = match f[0] {
            "usage" => Layer::Usage,
            "initial" => Layer::Initial,
            other => return Err(parse_error(file, n, format!("未知の層: {other}"))),
        };
        let source = match (layer, f[1]) {
            (Layer::Usage, NO_SOURCE) => USAGE_SOURCE,
            (Layer::Initial, s) if s != NO_SOURCE => s
                .parse::<u32>()
                .map_err(|_| parse_error(file, n, "source が数値ではありません"))?,
            _ => return Err(parse_error(file, n, "層と source の組が不正です")),
        };
        let unit = match f[2] {
            "phrase" => Unit::Phrase,
            "word" => Unit::Word,
            other => return Err(parse_error(file, n, format!("未知の単位: {other}"))),
        };
        if f[3].is_empty() || f[4].is_empty() {
            return Err(parse_error(file, n, "読みか表記が空です"));
        }
        let feature = parse_hex64(f[6])
            .ok_or_else(|| parse_error(file, n, "feature が16進の数ではありません"))?;
        let count = f[7]
            .parse::<u32>()
            .ok()
            .filter(|c| *c >= 1)
            .ok_or_else(|| parse_error(file, n, "count は 1 以上の整数です"))?;
        let last_day = f[8]
            .parse::<u32>()
            .map_err(|_| parse_error(file, n, "last_day が整数ではありません"))?;
        let key = Key {
            reading: f[3].to_string(),
            unit,
            feature,
            layer,
            source,
            surface: f[4].to_string(),
        };
        let stat = Stat {
            segments: f[5].to_string(),
            count,
            last_day,
        };
        if rows.insert(key, stat).is_some() {
            return Err(parse_error(file, n, "同じキーの行が重複しています"));
        }
    }
    Ok((salt, rows))
}

fn parse_hex64(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > SALT_HEX_DIGITS || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(s, 16).ok()
}

/// ソルトは秘密である必要はない（ファイルに保存する値）。時刻、PID、アドレスから作る。
fn new_salt() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let local = 0u8;
    let addr = std::ptr::from_ref(&local) as usize;
    let mut h = fnv1a64(&nanos.to_le_bytes());
    h = fnv1a64_continue(h, &std::process::id().to_le_bytes());
    fnv1a64_continue(h, &addr.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::super::features::extract;
    use super::super::features::test_support::{analysis, token};
    use super::*;
    use crate::analysis::AnalyzedContext;
    use crate::candidate::Candidate;

    pub(crate) fn seg(surface: &str, reading: &str, word: bool) -> Segment {
        Segment {
            reading: reading.to_string(),
            surface: surface.to_string(),
            word: word.then_some(crate::dict::WordId { dict: 0, index: 0 }),
            left_id: 1,
            right_id: 1,
            pos_id: 0,
        }
    }

    pub(crate) fn commit(reading: &str, segs: Vec<Segment>, ctx: AnalyzedContext) -> CommitEvent {
        let surface: String = segs.iter().map(|s| s.surface.as_str()).collect();
        CommitEvent {
            reading: reading.to_string(),
            candidate: Candidate {
                surface,
                segments: segs,
                base_cost: 0,
                score: 0,
                origin: CandidateOrigin::Dictionary,
            },
            analysis: Arc::new(ctx),
            explicit: true,
        }
    }

    fn open_tmp() -> (tempfile::TempDir, LearningStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LearningStore::open(&dir.path().join("learning.tsv")).unwrap();
        (dir, store)
    }

    fn tech() -> AnalyzedContext {
        analysis(vec![
            token("仕様", true, 1, None),
            token("を", false, 2, None),
        ])
    }

    #[test]
    fn missing_file_opens_empty_and_save_creates_it() {
        let (dir, store) = open_tmp();
        assert!(store.is_empty());
        store.save().unwrap();
        let text = std::fs::read_to_string(dir.path().join("learning.tsv")).unwrap();
        assert!(text.starts_with("#ikari-learning\tschema=1\tsalt="));
        let again = LearningStore::open(&dir.path().join("learning.tsv")).unwrap();
        assert_eq!(again.salt(), store.salt());
    }

    #[test]
    fn roundtrip_preserves_every_row() {
        let (dir, store) = open_tmp();
        let ev = commit(
            "はじめてしった",
            vec![
                seg("初めて", "はじめて", true),
                seg("知った", "しった", true),
            ],
            tech(),
        );
        store.record(&ev, None, 100);
        store.record(&ev, None, 101);
        let feats = extract(&tech(), None, store.salt());
        store.add_initial(
            3,
            "ひんし",
            &[SegmentKey {
                surface: "品詞".into(),
                reading: "ひんし".into(),
                pos_name: "名詞,普通名詞,一般,*,*,*".into(),
            }],
            &feats,
            100,
        );
        store.save().unwrap();
        let reopened = LearningStore::open(store.path()).unwrap();
        assert_eq!(reopened.records(), store.records());
        assert_eq!(reopened.salt(), store.salt());
        assert!(!store.is_empty());
        drop(dir);
    }

    #[test]
    fn record_counts_and_updates_last_day() {
        let (_d, store) = open_tmp();
        let ev = commit("ひんし", vec![seg("品詞", "ひんし", true)], tech());
        store.record(&ev, None, 10);
        store.record(&ev, None, 12);
        let hits = store.lookup("ひんし");
        assert!(!hits.is_empty());
        let w = store.record_policy().explicit_weight;
        assert!(hits.iter().all(|h| h.count == 2 * w && h.last_day == 12));
        assert!(hits.iter().all(|h| h.unit == Unit::Phrase));
        assert!(hits.iter().all(|h| h.layer == Layer::Usage));
    }

    #[test]
    fn top_choice_adds_no_count_but_refreshes_last_day() {
        // ADR 0057: 1 位のままの確定は回数を増やさない。すでにある記録の last_day だけを更新する
        let (_d, store) = open_tmp();
        // 1 位の確定を 0 回扱いにする設定（既定は弱い重み 1。ADR 0057）
        store.set_record_policy(RecordPolicy {
            top_weight: 0,
            ..store.record_policy()
        });
        let mut ev = commit("ひんし", vec![seg("品詞", "ひんし", true)], tech());
        // 記録がないうちの 1 位の確定は何も作らない
        ev.explicit = false;
        store.record(&ev, None, 10);
        assert!(store.is_empty());
        // 1 位以外を選んだ確定は、重みの分だけ数える
        ev.explicit = true;
        store.record(&ev, None, 10);
        let w = store.record_policy().explicit_weight;
        assert!(
            store
                .lookup("ひんし")
                .iter()
                .all(|h| h.count == w && h.last_day == 10)
        );
        // 1 位の確定: 回数はそのまま、last_day だけ新しくなる
        ev.explicit = false;
        store.record(&ev, None, 50);
        assert!(
            store
                .lookup("ひんし")
                .iter()
                .all(|h| h.count == w && h.last_day == 50)
        );
        // 更新をやめる設定なら last_day も動かない
        store.set_record_policy(RecordPolicy {
            top_refresh: false,
            ..store.record_policy()
        });
        store.record(&ev, None, 90);
        assert!(store.lookup("ひんし").iter().all(|h| h.last_day == 50));
        // 1 位の確定に重みを付ければ数える
        store.set_record_policy(RecordPolicy {
            top_weight: 1,
            ..store.record_policy()
        });
        store.record(&ev, None, 90);
        assert!(
            store
                .lookup("ひんし")
                .iter()
                .all(|h| h.count == w + 1 && h.last_day == 90)
        );
    }

    #[test]
    fn one_row_per_feature_and_words_for_multi_segment_commits() {
        let (_d, store) = open_tmp();
        let ctx = tech();
        let n_features = extract(&ctx, None, store.salt()).features.len();
        let ev = commit(
            "はじめてしった",
            vec![
                seg("初めて", "はじめて", true),
                seg("知った", "しった", true),
            ],
            ctx,
        );
        store.record(&ev, None, 1);
        assert_eq!(
            store
                .lookup("はじめてしった")
                .iter()
                .filter(|r| r.unit == Unit::Phrase)
                .count(),
            n_features
        );
        let words = store.lookup("はじめて");
        assert_eq!(words.len(), n_features);
        assert!(words.iter().all(|r| r.unit == Unit::Word));
        // 1 セグメントの確定は phrase だけ（二重に数えない）
        let single = commit("ひんし", vec![seg("品詞", "ひんし", true)], tech());
        store.record(&single, None, 1);
        assert!(
            store
                .lookup("ひんし")
                .iter()
                .all(|r| r.unit == Unit::Phrase)
        );
    }

    #[test]
    fn long_readings_are_not_recorded_as_phrases() {
        let (_d, store) = open_tmp();
        let long = "あ".repeat(MAX_READING_CHARS + 1);
        let ev = commit(&long, vec![seg("亜", &long, true)], tech());
        store.record(&ev, None, 1);
        assert!(store.is_empty());
        assert!(store.recent().latest(&long).is_none());
    }

    #[test]
    fn record_notes_recent_choice_and_reset_clears_it() {
        let (_d, store) = open_tmp();
        store.record(
            &commit("ひんし", vec![seg("品詞", "ひんし", true)], tech()),
            None,
            1,
        );
        assert_eq!(
            store.recent().latest("ひんし").map(|(s, _)| s),
            Some("品詞".into())
        );
        store.reset(LayerSel::Initial);
        assert!(store.recent().latest("ひんし").is_some());
        store.reset(LayerSel::Usage);
        assert!(store.recent().latest("ひんし").is_none());
    }

    #[test]
    fn rewriter_candidates_are_stored_by_format_not_text() {
        let (_d, store) = open_tmp();
        let mut ev = commit("きょう", vec![seg("2026/10/11", "きょう", false)], tech());
        ev.candidate.origin = CandidateOrigin::Rewriter {
            name: "date",
            format: 1,
        };
        store.record(&ev, None, 1);
        let text = serialize(store.salt(), &store.read());
        assert!(text.contains("@rw:date:1"));
        assert!(!text.contains("2026/10/11"));
    }

    #[test]
    fn reset_by_layer_and_remove_source() {
        let (_d, store) = open_tmp();
        let feats = extract(&tech(), None, store.salt());
        let one = |s: &str| {
            vec![SegmentKey {
                surface: s.into(),
                reading: "ひんし".into(),
                pos_name: "名詞".into(),
            }]
        };
        store.add_initial(1, "ひんし", &one("品詞"), &feats, 1);
        store.add_initial(2, "ひんし", &one("瀕死"), &feats, 1);
        store.record(
            &commit("ひんし", vec![seg("品詞", "ひんし", true)], tech()),
            None,
            1,
        );
        let (usage, initial) = store.layer_counts();
        assert!(usage > 0 && initial == usage * 2);

        store.remove_source(1);
        let (u2, i2) = store.layer_counts();
        assert_eq!((u2, i2), (usage, usage));
        assert!(
            store
                .records()
                .iter()
                .all(|r| r.source != Some(1) && (r.layer == Layer::Usage || r.surface == "瀕死"))
        );

        store.reset(LayerSel::Initial);
        assert_eq!(store.layer_counts(), (usage, 0));
        store.add_initial(1, "ひんし", &one("品詞"), &feats, 1);
        store.reset(LayerSel::Usage);
        assert_eq!(store.layer_counts(), (0, usage));
        store.reset(LayerSel::All);
        assert!(store.is_empty());
    }

    #[test]
    fn prune_drops_oldest_rows_first() {
        let mut rows = Rows::new();
        for (i, day) in [5u32, 1, 9, 3].into_iter().enumerate() {
            rows.insert(
                Key {
                    reading: format!("r{i}"),
                    unit: Unit::Word,
                    feature: 0,
                    layer: Layer::Usage,
                    source: 0,
                    surface: "x".into(),
                },
                Stat {
                    segments: String::new(),
                    count: 1,
                    last_day: day,
                },
            );
        }
        prune(&mut rows, 2);
        let mut days: Vec<u32> = rows.values().map(|s| s.last_day).collect();
        days.sort();
        assert_eq!(days, vec![5, 9]);
    }

    #[test]
    fn invalid_files_are_rejected_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("learning.tsv");
        let good = "#ikari-learning\tschema=1\tsalt=00000000000000ff\n";
        let cases = [
            "".to_string(),
            "#other\tschema=1\tsalt=00ff\n".to_string(),
            "#ikari-learning\tschema=2\tsalt=00ff\n".to_string(),
            "#ikari-learning\tschema=1\n".to_string(),
            format!("{good}wrong columns\n"),
            format!("{good}{COLUMNS}\nusage\t-\tword\tあ\t亜\t\tzz\t1\t1\n"),
            format!("{good}{COLUMNS}\nusage\t-\tword\tあ\t亜\t\t00ff\t0\t1\n"),
            format!("{good}{COLUMNS}\nusage\t3\tword\tあ\t亜\t\t00ff\t1\t1\n"),
            format!("{good}{COLUMNS}\ninitial\t-\tword\tあ\t亜\t\t00ff\t1\t1\n"),
            format!("{good}{COLUMNS}\nusage\t-\tword\tあ\n"),
        ];
        for (i, text) in cases.iter().enumerate() {
            std::fs::write(&p, text).unwrap();
            assert!(LearningStore::open(&p).is_err(), "case {i} accepted");
            assert!(LearningStore::validate_text(text).is_err(), "case {i}");
            assert_eq!(&std::fs::read_to_string(&p).unwrap(), text);
        }
        let dup = format!(
            "{good}{COLUMNS}\nusage\t-\tword\tあ\t亜\t\t00ff\t1\t1\nusage\t-\tword\tあ\t亜\t\t00ff\t1\t1\n"
        );
        assert!(LearningStore::validate_text(&dup).is_err());
    }

    fn bad_texts() -> Vec<Vec<u8>> {
        let good = "#ikari-learning\tschema=1\tsalt=00000000000000ff\n";
        vec![
            format!("{good}wrong columns\n").into_bytes(),
            format!("{good}{COLUMNS}\nusage\t-\tword\tあ\t亜\t\t00ff\t0\t1\n").into_bytes(),
            format!(
                "{good}{COLUMNS}\nusage\t-\tword\tあ\t亜\t\t00ff\t1\t1\nusage\t-\tword\tあ\t亜\t\t00ff\t1\t1\n"
            )
            .into_bytes(),
            vec![0xff, 0xfe, 0x00, b'\n'],
        ]
    }

    #[test]
    fn invalid_file_is_moved_to_bad_and_opens_empty() {
        for (i, bytes) in bad_texts().iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("learning.tsv");
            std::fs::write(&p, bytes).unwrap();
            let (store, moved) = LearningStore::open_or_quarantine(&p).unwrap();
            let bad = dir.path().join("learning.tsv.bad");
            assert_eq!(moved.as_deref(), Some(bad.as_path()), "case {i}");
            assert_eq!(&std::fs::read(&bad).unwrap(), bytes, "case {i}");
            assert!(!p.exists(), "case {i}");
            assert!(store.is_empty(), "case {i}");
        }
    }

    #[test]
    fn quarantine_keeps_mode_and_never_overwrites() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("learning.tsv");
        let mut expected = Vec::new();
        for n in 0..3u8 {
            let body = vec![b'x', n];
            std::fs::write(&p, &body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            let (_, moved) = LearningStore::open_or_quarantine(&p).unwrap();
            let moved = moved.unwrap();
            let mode = std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            expected.push((moved, body));
        }
        let names: Vec<String> = expected
            .iter()
            .map(|(m, _)| m.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "learning.tsv.bad",
                "learning.tsv.bad.1",
                "learning.tsv.bad.2"
            ]
        );
        for (m, body) in &expected {
            assert_eq!(&std::fs::read(m).unwrap(), body);
        }
    }

    #[test]
    fn valid_or_missing_file_is_not_moved() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("learning.tsv");
        let (store, moved) = LearningStore::open_or_quarantine(&p).unwrap();
        assert!(moved.is_none());
        store.save().unwrap();
        let before = std::fs::read(&p).unwrap();
        let (_, moved) = LearningStore::open_or_quarantine(&p).unwrap();
        assert!(moved.is_none());
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(!dir.path().join("learning.tsv.bad").exists());
    }

    #[test]
    fn unmovable_invalid_file_stays_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("learning.tsv");
        std::fs::write(&p, b"broken\n").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = LearningStore::open_or_quarantine(&p);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        // root はフォルダの権限を無視して移せてしまうので、その場合は確かめない
        if let Err(e) = result {
            assert!(matches!(
                e,
                UserDataError::Parse { .. } | UserDataError::Invalid(_)
            ));
            assert_eq!(std::fs::read(&p).unwrap(), b"broken\n");
        }
    }

    #[test]
    fn schema_mismatch_is_reported_as_schema_error() {
        let err = LearningStore::validate_text("#ikari-learning\tschema=9\tsalt=ff\n").unwrap_err();
        assert!(matches!(
            err,
            UserDataError::Schema {
                found: 9,
                expected: 1
            }
        ));
    }

    #[test]
    fn learning_file_has_no_left_context_text() {
        // 左文脈の語（lemma）は特徴量のハッシュとしてしか現れない
        let (_d, store) = open_tmp();
        let ctx = analysis(vec![
            token("極秘プロジェクト", true, 1, None),
            token("の", false, 2, None),
        ]);
        store.record(
            &commit("ひんし", vec![seg("品詞", "ひんし", true)], ctx),
            None,
            1,
        );
        let text = serialize(store.salt(), &store.read());
        assert!(!text.contains("極秘"));
        assert!(store.records().iter().any(|r| r.feature != 0));
    }
}
