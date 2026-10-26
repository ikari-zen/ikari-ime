//! 先行計算のキャッシュ、確定待ちの変換の表、先行計算の待ち行列（ADR 0004 §7.2）。

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ikari_core::Context;

use crate::segments::SegEntry;

/// セッションごとに持つ件数の上限
pub const PER_SESSION_LIMIT: usize = 8;
/// 全体の件数の上限
pub const TOTAL_LIMIT: usize = 64;
/// 使われなかったら捨てるまでの時間
pub const IDLE_LIMIT: Duration = Duration::from_secs(60);

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // ロックを持ったまま panic しても、保持しているのは単純な値なのでそのまま使う
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 変換1つ（読み全体の変換と、文節の区切り）。ページの生成で `&mut` が要るのでロックで包む。
pub type SharedConversion = Arc<Mutex<SegEntry>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub session: u64,
    pub reading: String,
    pub context_rev: u64,
}

/// キャッシュに当たらなかった理由（ログ用。同じセッションのキャッシュと見比べる）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissReason {
    /// 文脈の番号は同じで、読みだけが違うものがある
    Reading,
    /// 読みは同じで、文脈の番号だけが違うものがある
    Context,
    /// 近いものがない
    None,
}

/// `Convert` がキャッシュをどう使ったか（ログ用）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheOutcome {
    /// 先行計算が Space の前に終わっていた
    Hit,
    /// 計算中で、待って間に合った
    WaitHit,
    /// 計算中で、待ったが間に合わなかった（または捨てられた）
    WaitMiss,
    Miss(MissReason),
}

impl CacheOutcome {
    /// ログの注記（`cache=hit lm_applied=1`、外れなら `cache=miss lm_applied=0 miss=reading`）。固定の語だけ（NFR-19）
    pub fn log_note(self, lm_applied: bool) -> String {
        let cache = match self {
            CacheOutcome::Hit => "hit",
            CacheOutcome::WaitHit => "wait_hit",
            CacheOutcome::WaitMiss => "wait_miss",
            CacheOutcome::Miss(_) => "miss",
        };
        let lm = u8::from(lm_applied);
        match self {
            CacheOutcome::Miss(reason) => {
                let reason = match reason {
                    MissReason::Reading => "reading",
                    MissReason::Context => "context",
                    MissReason::None => "none",
                };
                format!("cache={cache} lm_applied={lm} miss={reason}")
            }
            _ => format!("cache={cache} lm_applied={lm}"),
        }
    }
}

enum EntryState {
    Computing,
    Ready {
        conv: SharedConversion,
        lm_applied: bool,
    },
    /// 計算に失敗した、または捨てられた
    Abandoned,
}

/// 先行計算の1件。計算中は `Convert` が締め切りまで待てる。
pub struct CacheEntry {
    pub key: CacheKey,
    state: Mutex<EntryState>,
    ready: Condvar,
}

impl CacheEntry {
    pub fn new(key: CacheKey) -> Arc<CacheEntry> {
        Arc::new(CacheEntry {
            key,
            state: Mutex::new(EntryState::Computing),
            ready: Condvar::new(),
        })
    }

    pub fn set_ready(&self, conv: SharedConversion, lm_applied: bool) {
        let mut state = lock(&self.state);
        if matches!(*state, EntryState::Computing) {
            *state = EntryState::Ready { conv, lm_applied };
        }
        self.ready.notify_all();
    }

    /// 失敗、または捨てる。待っている `Convert` を起こす。計算済みのものは変えない。
    pub fn abandon(&self) {
        let mut state = lock(&self.state);
        if matches!(*state, EntryState::Computing) {
            *state = EntryState::Abandoned;
        }
        self.ready.notify_all();
    }

    pub fn is_computing(&self) -> bool {
        matches!(*lock(&self.state), EntryState::Computing)
    }

    /// `until` まで、計算が終わるのを待つ。間に合わない、または失敗なら None。
    pub fn wait_ready(&self, until: Instant) -> Option<(SharedConversion, bool)> {
        let mut state = lock(&self.state);
        loop {
            match &*state {
                EntryState::Ready { conv, lm_applied } => {
                    return Some((Arc::clone(conv), *lm_applied));
                }
                EntryState::Abandoned => return None,
                EntryState::Computing => {}
            }
            let now = Instant::now();
            if now >= until {
                return None;
            }
            state = self
                .ready
                .wait_timeout(state, until - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

struct CacheItem {
    entry: Arc<CacheEntry>,
    last_used: Instant,
}

/// 先行計算の結果。セッションごとに最新の 8 件、全体で 64 件まで。
#[derive(Default)]
pub struct PrefetchCache {
    items: Vec<CacheItem>,
}

impl PrefetchCache {
    pub fn find(&mut self, key: &CacheKey) -> Option<Arc<CacheEntry>> {
        let item = self.items.iter_mut().find(|i| i.entry.key == *key)?;
        item.last_used = Instant::now();
        Some(Arc::clone(&item.entry))
    }

    /// `key` に当たるものがなかった理由。同じセッションの中で、読みだけ／文脈の番号だけが違うものを探す
    pub fn miss_reason(&self, key: &CacheKey) -> MissReason {
        let same_session = || {
            self.items
                .iter()
                .filter(|i| i.entry.key.session == key.session)
        };
        if same_session().any(|i| i.entry.key.reading == key.reading) {
            MissReason::Context
        } else if same_session().any(|i| i.entry.key.context_rev == key.context_rev) {
            MissReason::Reading
        } else {
            MissReason::None
        }
    }

    /// 入れる。上限を超えた古いものは捨てる（計算中なら中止扱いにして待ちを解く）。
    pub fn insert(&mut self, entry: Arc<CacheEntry>) {
        self.items.push(CacheItem {
            entry: Arc::clone(&entry),
            last_used: Instant::now(),
        });
        let session = entry.key.session;
        while self
            .items
            .iter()
            .filter(|i| i.entry.key.session == session)
            .count()
            > PER_SESSION_LIMIT
        {
            self.evict_oldest(Some(session));
        }
        while self.items.len() > TOTAL_LIMIT {
            self.evict_oldest(None);
        }
    }

    fn evict_oldest(&mut self, session: Option<u64>) {
        let oldest = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| session.is_none_or(|s| i.entry.key.session == s))
            .min_by_key(|(_, i)| i.last_used)
            .map(|(idx, _)| idx);
        if let Some(idx) = oldest {
            self.items.remove(idx).entry.abandon();
        }
    }

    pub fn remove_session(&mut self, session: u64) {
        self.items.retain(|i| {
            let keep = i.entry.key.session != session;
            if !keep {
                i.entry.abandon();
            }
            keep
        });
    }

    pub fn remove(&mut self, entry: &Arc<CacheEntry>) {
        self.items.retain(|i| {
            let keep = !Arc::ptr_eq(&i.entry, entry);
            if !keep {
                i.entry.abandon();
            }
            keep
        });
    }

    pub fn clear(&mut self) {
        for item in self.items.drain(..) {
            item.entry.abandon();
        }
    }

    pub fn expire(&mut self, now: Instant) {
        self.items.retain(|i| {
            let keep = now.duration_since(i.last_used) < IDLE_LIMIT;
            if !keep {
                i.entry.abandon();
            }
            keep
        });
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn any_computing(&self) -> bool {
        self.items.iter().any(|i| i.entry.is_computing())
    }
}

/// 殻へ返した変換。`Commit` / `PageRequest` / `Cancel` が `conversion_id` で引く。
pub struct ConversionItem {
    pub id: u64,
    pub session: u64,
    pub conv: SharedConversion,
    pub page_size: usize,
    last_used: Instant,
}

#[derive(Default)]
pub struct ConversionTable {
    items: Vec<ConversionItem>,
}

impl ConversionTable {
    pub fn insert(&mut self, id: u64, session: u64, conv: SharedConversion, page_size: usize) {
        self.items.push(ConversionItem {
            id,
            session,
            conv,
            page_size,
            last_used: Instant::now(),
        });
        while self.items.iter().filter(|i| i.session == session).count() > PER_SESSION_LIMIT {
            self.evict_oldest(Some(session));
        }
        while self.items.len() > TOTAL_LIMIT {
            self.evict_oldest(None);
        }
    }

    fn evict_oldest(&mut self, session: Option<u64>) {
        let oldest = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| session.is_none_or(|s| i.session == s))
            .min_by_key(|(_, i)| i.last_used)
            .map(|(idx, _)| idx);
        if let Some(idx) = oldest {
            self.items.remove(idx);
        }
    }

    /// 引いて、使った時刻を更新する。
    pub fn get(&mut self, id: u64) -> Option<(SharedConversion, usize, u64)> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        item.last_used = Instant::now();
        Some((Arc::clone(&item.conv), item.page_size, item.session))
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        self.items.len() != before
    }

    pub fn remove_session(&mut self, session: u64) {
        self.items.retain(|i| i.session != session);
    }

    pub fn expire(&mut self, now: Instant) {
        self.items
            .retain(|i| now.duration_since(i.last_used) < IDLE_LIMIT);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// 先行計算の依頼。
pub struct Job {
    pub entry: Arc<CacheEntry>,
    pub context: Context,
    seq: u64,
}

#[derive(Default)]
struct QueueState {
    /// セッションごとに最新の 1 件だけ
    jobs: HashMap<u64, Job>,
    seq: u64,
    closed: bool,
}

/// 先行計算の待ち行列。古い依頼は捨て、ワーカーは最後に出された依頼から処理する。
#[derive(Default)]
pub struct PrefetchQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}

impl PrefetchQueue {
    /// 依頼を置く。同じセッションの古い依頼は捨てる（その `entry` は中止扱いになる）。
    pub fn submit(&self, entry: Arc<CacheEntry>, context: Context) -> Option<Arc<CacheEntry>> {
        let mut state = lock(&self.state);
        state.seq += 1;
        let job = Job {
            entry: Arc::clone(&entry),
            context,
            seq: state.seq,
        };
        let replaced = state.jobs.insert(entry.key.session, job);
        self.wake.notify_one();
        replaced.map(|j| j.entry)
    }

    /// セッションの待ち依頼を捨てる。
    pub fn forget_session(&self, session: u64) -> Option<Arc<CacheEntry>> {
        lock(&self.state).jobs.remove(&session).map(|j| j.entry)
    }

    pub fn clear(&self) -> Vec<Arc<CacheEntry>> {
        lock(&self.state)
            .jobs
            .drain()
            .map(|(_, j)| j.entry)
            .collect()
    }

    /// 次の依頼を待つ。閉じられたら None。
    pub fn next(&self) -> Option<Job> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return None;
            }
            let newest = state
                .jobs
                .iter()
                .max_by_key(|(_, j)| j.seq)
                .map(|(session, _)| *session);
            if let Some(session) = newest {
                return state.jobs.remove(&session);
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        for (_, job) in state.jobs.drain() {
            job.entry.abandon();
        }
        self.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segments::SegmentSeam;

    fn key(session: u64, reading: &str, rev: u64) -> CacheKey {
        CacheKey {
            session,
            reading: reading.to_owned(),
            context_rev: rev,
        }
    }

    #[test]
    fn cache_keeps_the_latest_eight_per_session() {
        let mut cache = PrefetchCache::default();
        let mut entries = Vec::new();
        for i in 0..(PER_SESSION_LIMIT as u64 + 3) {
            let e = CacheEntry::new(key(1, "あ", i));
            cache.insert(Arc::clone(&e));
            entries.push(e);
        }
        assert_eq!(cache.len(), PER_SESSION_LIMIT);
        assert!(cache.find(&key(1, "あ", 0)).is_none());
        assert!(
            !entries[0].is_computing(),
            "evicted entries release waiters"
        );
        assert!(
            cache
                .find(&key(1, "あ", PER_SESSION_LIMIT as u64 + 2))
                .is_some()
        );
    }

    #[test]
    fn cache_total_is_capped() {
        let mut cache = PrefetchCache::default();
        for session in 0..(TOTAL_LIMIT as u64 + 10) {
            cache.insert(CacheEntry::new(key(session, "あ", 0)));
        }
        assert_eq!(cache.len(), TOTAL_LIMIT);
    }

    #[test]
    fn removing_a_session_and_expiring_drop_entries() {
        let mut cache = PrefetchCache::default();
        let a = CacheEntry::new(key(1, "あ", 0));
        cache.insert(Arc::clone(&a));
        cache.insert(CacheEntry::new(key(2, "い", 0)));
        cache.remove_session(1);
        assert_eq!(cache.len(), 1);
        assert!(!a.is_computing());
        cache.expire(Instant::now() + IDLE_LIMIT + Duration::from_secs(1));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn waiting_for_an_entry_times_out_or_wakes() {
        let entry = CacheEntry::new(key(1, "あ", 0));
        let start = Instant::now();
        assert!(
            entry
                .wait_ready(start + Duration::from_millis(20))
                .is_none()
        );
        assert!(start.elapsed() >= Duration::from_millis(20));

        let waiter = {
            let entry = Arc::clone(&entry);
            std::thread::spawn(move || entry.wait_ready(Instant::now() + Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(10));
        entry.abandon();
        assert!(waiter.join().unwrap().is_none());
        assert!(!entry.is_computing());
    }

    #[test]
    fn miss_reason_tells_reading_from_context() {
        let mut cache = PrefetchCache::default();
        cache.insert(CacheEntry::new(key(1, "あい", 5)));
        assert_eq!(cache.miss_reason(&key(1, "あ", 5)), MissReason::Reading);
        assert_eq!(cache.miss_reason(&key(1, "あい", 6)), MissReason::Context);
        assert_eq!(cache.miss_reason(&key(1, "あ", 6)), MissReason::None);
        // ほかのセッションのものは見ない
        assert_eq!(cache.miss_reason(&key(2, "あい", 5)), MissReason::None);
    }

    #[test]
    fn outcome_notes_are_fixed_words() {
        assert_eq!(CacheOutcome::Hit.log_note(true), "cache=hit lm_applied=1");
        assert_eq!(
            CacheOutcome::WaitHit.log_note(true),
            "cache=wait_hit lm_applied=1"
        );
        assert_eq!(
            CacheOutcome::WaitMiss.log_note(false),
            "cache=wait_miss lm_applied=0"
        );
        assert_eq!(
            CacheOutcome::Miss(MissReason::Context).log_note(false),
            "cache=miss lm_applied=0 miss=context"
        );
    }

    #[test]
    fn queue_keeps_only_the_latest_job_per_session() {
        let queue = PrefetchQueue::default();
        let first = CacheEntry::new(key(1, "あ", 1));
        let second = CacheEntry::new(key(1, "あい", 2));
        assert!(queue.submit(Arc::clone(&first), Context::new()).is_none());
        let replaced = queue.submit(Arc::clone(&second), Context::new()).unwrap();
        assert!(Arc::ptr_eq(&replaced, &first));
        let job = queue.next().unwrap();
        assert!(Arc::ptr_eq(&job.entry, &second));
    }

    #[test]
    fn queue_serves_the_newest_job_first_and_closes() {
        let queue = Arc::new(PrefetchQueue::default());
        let a = CacheEntry::new(key(1, "あ", 1));
        let b = CacheEntry::new(key(2, "い", 1));
        queue.submit(Arc::clone(&a), Context::new());
        queue.submit(Arc::clone(&b), Context::new());
        assert!(Arc::ptr_eq(&queue.next().unwrap().entry, &b));
        assert!(Arc::ptr_eq(&queue.next().unwrap().entry, &a));
        let waiter = {
            let queue = Arc::clone(&queue);
            std::thread::spawn(move || queue.next().is_none())
        };
        std::thread::sleep(Duration::from_millis(10));
        queue.close();
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn conversion_table_limits_and_removes() {
        let mut table = ConversionTable::default();
        let conv = |_: u64| -> SharedConversion {
            // 中身は使わない。ダミーの変換を作る
            Arc::new(Mutex::new(dummy_entry()))
        };
        for id in 1..=(PER_SESSION_LIMIT as u64 + 2) {
            table.insert(id, 7, conv(id), 9);
        }
        assert_eq!(table.len(), PER_SESSION_LIMIT);
        assert!(table.get(1).is_none());
        assert!(table.get(PER_SESSION_LIMIT as u64 + 2).is_some());
        assert!(table.remove(PER_SESSION_LIMIT as u64 + 2));
        assert!(!table.remove(PER_SESSION_LIMIT as u64 + 2));
        table.remove_session(7);
        assert_eq!(table.len(), 0);
    }

    fn dummy_entry() -> SegEntry {
        use ikari_core::{
            Converter, DenseGrammar, Dictionary, Grammar, InMemoryDictionary, UnknownParams,
        };
        let g = DenseGrammar::new(
            2,
            2,
            0,
            UnknownParams {
                left_id: 1,
                right_id: 1,
                pos_id: 0,
                cost_per_char: 5000,
            },
        );
        let d = InMemoryDictionary::new(g.grammar_id());
        let (d, g): (Arc<dyn Dictionary>, Arc<dyn Grammar>) = (Arc::new(d), Arc::new(g));
        let converter = Arc::new(Converter::builder(d, g).build().unwrap());
        let conv = converter.convert("あ", &Context::new());
        SegEntry::new(crate::mock_seam::MockSeam::new(converter).segment(conv))
    }
}
