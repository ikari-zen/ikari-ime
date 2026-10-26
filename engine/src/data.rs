//! ユーザーデータの更新の検出、Stack の組み立て直し、学習の保存。

use super::*;

use crate::dispatch::{emoji_tsv_path_in, postal_tsv_path_in};
use ikari_core::EmojiTable;

pub(crate) struct SaveState {
    pub(crate) dirty: bool,
    pub(crate) last_save: Instant,
}

pub(crate) type Stamp = Option<(SystemTime, u64)>;

/// ユーザー辞書とプロファイルの更新の検出（mtime と大きさ）。
pub(crate) struct DataWatch {
    pub(crate) paths: [PathBuf; 2],
    pub(crate) known: [Stamp; 2],
    /// `emoji.tsv`。辞書の組み立て直しとは別に、表だけを読み直す
    pub(crate) emoji_path: PathBuf,
    pub(crate) emoji_known: Stamp,
    /// `postal.tsv`。変わったら Stack を組み立て直す（表は Rewriter が持つ）
    postal_path: PathBuf,
    postal_known: Stamp,
    pub(crate) last_check: Instant,
}

impl DataWatch {
    pub(crate) fn new(data_dir: &Path) -> DataWatch {
        let user = UserDir::new(data_dir);
        let paths = [user.user_dict_tsv(), user.profile_tsv()];
        let known = [stamp(&paths[0]), stamp(&paths[1])];
        let emoji_path = emoji_tsv_path_in(data_dir);
        let postal_path = postal_tsv_path_in(data_dir);
        DataWatch {
            paths,
            known,
            emoji_known: stamp(&emoji_path),
            emoji_path,
            postal_known: stamp(&postal_path),
            postal_path,
            last_check: Instant::now(),
        }
    }

    /// 今のファイルの状態を「確認済み」にする（管理の要求が自分で書いたとき、二重に組み立て直さない）。
    pub(crate) fn resync(&mut self) {
        self.known = [stamp(&self.paths[0]), stamp(&self.paths[1])];
        self.last_check = Instant::now();
    }

    /// 前回と違えば true（確認した値を覚える）。
    pub(crate) fn changed(&mut self) -> bool {
        let now = [stamp(&self.paths[0]), stamp(&self.paths[1])];
        let changed = now != self.known;
        self.known = now;
        changed
    }

    /// 郵便番号の表が前回と違えば true（確認した値を覚える）。
    pub(crate) fn postal_changed(&mut self) -> bool {
        let now = stamp(&self.postal_path);
        let changed = now != self.postal_known;
        self.postal_known = now;
        changed
    }

    /// 絵文字の表が前回と違えば true（確認した値を覚える）。
    pub(crate) fn emoji_changed(&mut self) -> bool {
        let now = stamp(&self.emoji_path);
        let changed = now != self.emoji_known;
        self.emoji_known = now;
        changed
    }
}

/// `emoji.tsv` を読んで返事にする。ファイルがない、壊れている、読めないときは空の表
/// （ショートコードが使えなくなるだけで、変換には影響しない）。
pub(crate) fn load_emoji_reply(path: Option<&Path>) -> pb::EmojiTableReply {
    let Some(path) = path else {
        return pb::EmojiTableReply::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return pb::EmojiTableReply::default();
    };
    let Ok(table) = EmojiTable::from_tsv(&text) else {
        return pb::EmojiTableReply::default();
    };
    if table.shortcodes().is_empty() {
        return pb::EmojiTableReply::default();
    }
    pb::EmojiTableReply {
        entries: table
            .shortcodes()
            .iter()
            .map(|(name, emoji)| pb::EmojiEntry {
                name: name.clone(),
                emoji: emoji.clone(),
            })
            .collect(),
        data_label: emoji_data_label(&text),
    }
}

/// 表の版を表す文字列（内容の FNV-1a。殻は前回と違うときだけ表を取り直す）
fn emoji_data_label(text: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = text.bytes().fold(FNV_OFFSET, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(FNV_PRIME)
    });
    format!("emoji-{hash:016x}")
}

pub(crate) fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl Inner {
    /// ユーザー辞書とプロファイルの更新を見る（1 秒に 1 回まで）。変わっていれば Stack を組み立て直す。
    /// 組み立て直しは時間がかかるので、変換の経路（Convert、Prefetch、Page）からは呼ばない。
    /// 見回りのスレッド（`tick`）だけが呼び、変換は古い Stack のまま進んで、差し替えは `Arc` の入れ替えだけで済む。
    pub(crate) fn refresh_data(&self) {
        let Some(watch) = &self.watch else {
            return;
        };
        // 別のスレッドが確認や組み立て直しの最中なら、そちらに任せて古い Stack で進む
        let Ok(mut watch) = watch.try_lock() else {
            return;
        };
        if watch.last_check.elapsed() < DATA_CHECK_INTERVAL {
            return;
        }
        watch.last_check = Instant::now();
        // 絵文字の表は辞書と別に読み直す（Stack は組み立て直さない）
        if watch.emoji_changed() {
            let reply = load_emoji_reply(Some(&watch.emoji_path));
            *self.emoji.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(reply);
        }
        let Some(rebuild) = &self.rebuild else {
            return;
        };
        // 両方の確認を済ませてから判断する（短絡で片方の「確認済み」が遅れないように）
        let data_changed = watch.changed();
        let postal_changed = watch.postal_changed();
        if !data_changed && !postal_changed {
            return;
        }
        let mut reuse = Reuse::from_stack(&self.stack());
        if postal_changed {
            // 表のファイルが変わったときだけ読み直す（読み込みは数十 ms かかるので、ほかの理由の組み立て直しでは使い回す）
            reuse.postal = None;
        }
        let _ = self.rebuild_stack(rebuild, &reuse);
    }

    /// `reuse` を使い回して Stack を組み立て直し、差し替える。成功したら古い Stack の先行計算を捨てる。
    /// 管理の要求（`admin.rs`）も使う。
    pub(crate) fn rebuild_stack(&self, rebuild: &StackBuilder, reuse: &Reuse) -> bool {
        match rebuild(reuse) {
            Ok(next) => {
                let old = self.stack();
                self.note_stack_swap(&old, &next);
                *self.stack.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
                self.warm_dictionary();
                self.discard_prefetched();
                self.log.record("reload", 0, self.lm_state_name(), None);
                true
            }
            Err(_) => {
                self.log
                    .record("reload", 0, self.lm_state_name(), Some("reload_failed"));
                false
            }
        }
    }

    /// 先行計算を捨てる。古い Stack や、リセット前の学習で作ったものは使わない
    /// （返した変換は、それぞれの辞書を持っているので残す）。
    pub(crate) fn discard_prefetched(&self) {
        for entry in self.queue.clear() {
            entry.abandon();
        }
        lock_mutex(&self.cache).clear();
    }

    pub(crate) fn save_now(&self) {
        let stack = self.stack();
        let Some(store) = &stack.learning else {
            return;
        };
        // ファイルの書き込みは `save` を持たずにやる（Commit が dirty を立てるのを待たせない）。
        // 書き込み同士は `save_io` で 1 つずつにする
        let _io = lock_mutex(&self.save_io);
        {
            let mut state = lock_mutex(&self.save);
            if !state.dirty {
                return;
            }
            // 書いている間に増えた学習は、次の保存に回る
            state.dirty = false;
        }
        let result = store.save();
        let mut state = lock_mutex(&self.save);
        match result {
            Ok(()) => state.last_save = Instant::now(),
            Err(_) => {
                state.dirty = true;
                self.log
                    .record("save", 0, self.lm_state_name(), Some("save_failed"));
            }
        }
    }
}
