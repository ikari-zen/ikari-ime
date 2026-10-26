//! セッション（開始、終了、確定履歴、左文脈の組み立て）。

use super::*;

pub(crate) struct Session {
    pub(crate) secure: bool,
    /// ほかのアプリと文脈を貸し借りしてよいか（`BeginSession.share_context`。D6）
    pub(crate) share: bool,
    pub(crate) history: Vec<ikari_core::Commit>,
    pub(crate) last_used: Instant,
    /// `EndSession` を受けた時刻。Some の間は休止中で、要求には答えず、同じ ID の `BeginSession` で再開する（D1）
    pub(crate) paused_at: Option<Instant>,
}

/// 全アプリの確定の 1 件（「最近の確定」。D5）。セッションの履歴とは別に、時刻順に持つ。
pub(crate) struct RecentCommit {
    pub(crate) session_id: u64,
    pub(crate) at: Instant,
    pub(crate) surface: String,
}

fn history_chars(history: &[ikari_core::Commit]) -> usize {
    history.iter().map(|c| c.surface.chars().count()).sum()
}

impl Inner {
    pub(crate) fn hello(&self) -> envelope::Body {
        self.ensure_warm();
        envelope::Body::HelloReply(pb::HelloReply {
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            dict_label: self.stack().meta.dict_label.clone(),
            lm: self.lm_state() as i32,
        })
    }

    pub(crate) fn begin_session(&self, b: pb::BeginSession) -> Result<envelope::Body, Fail> {
        // client_bundle_id は保存しない（アプリごとのプロファイルは将来の候補）。ログには、この要求の行の
        // `app=` にだけ書く（ADR 0034 §7・§9-1。ユーザー決定 2026-10-25。`diag::begin_session_note`。
        // 文字の種類と長さを確かめ、secure では書かない）。ほかの行には書かない
        let mut sessions = lock_mutex(&self.sessions);
        if !sessions.contains_key(&b.session_id) && sessions.len() >= SESSION_LIMIT {
            let oldest = sessions
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(id, _)| *id);
            if let Some(id) = oldest {
                sessions.remove(&id);
                drop(sessions);
                self.drop_session_state(id);
                sessions = lock_mutex(&self.sessions);
            }
        }
        let now = Instant::now();
        match sessions.get_mut(&b.session_id) {
            // 同じ ID で secure が同じなら、履歴を残して再開する（D1）
            Some(s) if s.secure == b.secure => {
                s.share = b.share_context;
                s.paused_at = None;
                s.last_used = now;
            }
            // secure が変わったら履歴をつながない（パスワード欄の前後）
            _ => {
                sessions.insert(
                    b.session_id,
                    Session {
                        secure: b.secure,
                        share: b.share_context,
                        history: Vec::new(),
                        last_used: now,
                        paused_at: None,
                    },
                );
            }
        }
        Ok(envelope::Body::Ack(pb::Ack {}))
    }

    pub(crate) fn end_session(&self, e: pb::EndSession) -> envelope::Body {
        // セッションは消さず休止にする。履歴は `paused_session_ttl` のあいだ残る（D1）
        if let Some(s) = lock_mutex(&self.sessions).get_mut(&e.session_id) {
            let now = Instant::now();
            s.paused_at = Some(now);
            s.last_used = now;
        }
        self.drop_session_state(e.session_id);
        self.save_now();
        envelope::Body::Ack(pb::Ack {})
    }

    pub(crate) fn drop_session_state(&self, session: u64) {
        if let Some(entry) = self.queue.forget_session(session) {
            entry.abandon();
        }
        self.lookahead.forget_session(session);
        lock_mutex(&self.cache).remove_session(session);
        lock_mutex(&self.ctx_trace).remove_session(session);
        lock_mutex(&self.conversions).remove_session(session);
    }

    /// (secure, 確定履歴)。セッションがなければ `UNKNOWN_SESSION`。
    pub(crate) fn session_info(&self, id: u64) -> Result<(bool, Vec<ikari_core::Commit>), Fail> {
        let mut sessions = lock_mutex(&self.sessions);
        let session = sessions
            .get_mut(&id)
            .filter(|s| s.paused_at.is_none())
            .ok_or(UNKNOWN_SESSION)?;
        session.last_used = Instant::now();
        Ok((session.secure, session.history.clone()))
    }

    pub(crate) fn push_history(&self, session: u64, reading: String, surface: String) {
        let limits = self.limits();
        let shared = {
            let mut sessions = lock_mutex(&self.sessions);
            let Some(s) = sessions
                .get_mut(&session)
                .filter(|s| !s.secure && s.paused_at.is_none())
            else {
                return;
            };
            let shared = s.share.then(|| surface.clone());
            s.history.push(ikari_core::Commit { reading, surface });
            // 表記の合計が上限を超えたら古い方から捨てる（最後の 1 件は残す。D4）
            let mut total = history_chars(&s.history);
            let mut drop_count = 0;
            while total > limits.history_max_chars && drop_count + 1 < s.history.len() {
                total -= s.history[drop_count].surface.chars().count();
                drop_count += 1;
            }
            s.history.drain(..drop_count);
            shared
        };
        if let Some(surface) = shared {
            self.push_recent(session, surface, &limits);
        }
    }

    /// 「最近の確定」に足し、古いものと上限を超えた分を捨てる（D5）
    fn push_recent(&self, session_id: u64, surface: String, limits: &ContextLimits) {
        let now = Instant::now();
        let mut recent = lock_mutex(&self.recent);
        recent.push(RecentCommit {
            session_id,
            at: now,
            surface,
        });
        Self::trim_recent(&mut recent, now, limits);
    }

    fn trim_recent(recent: &mut Vec<RecentCommit>, now: Instant, limits: &ContextLimits) {
        recent.retain(|r| now.saturating_duration_since(r.at) < limits.recent_commits_ttl);
        let mut total: usize = recent.iter().map(|r| r.surface.chars().count()).sum();
        let mut drop_count = 0;
        while total > limits.recent_commits_max_chars && drop_count + 1 < recent.len() {
            total -= recent[drop_count].surface.chars().count();
            drop_count += 1;
        }
        recent.drain(..drop_count);
    }

    /// 休止から `paused_session_ttl` たったセッションと、古い「最近の確定」を捨てる（見回りから）
    pub(crate) fn expire_context(&self, now: Instant) {
        let limits = self.limits();
        let expired: Vec<u64> = {
            let mut sessions = lock_mutex(&self.sessions);
            let ids: Vec<u64> = sessions
                .iter()
                .filter(|(_, s)| {
                    s.paused_at.is_some_and(|at| {
                        now.saturating_duration_since(at) >= limits.paused_session_ttl
                    })
                })
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                sessions.remove(id);
            }
            ids
        };
        for id in expired {
            self.drop_session_state(id);
        }
        Self::trim_recent(&mut lock_mutex(&self.recent), now, &limits);
    }

    /// X5: 同じ入力欄に文脈が何もないとき、直近 `borrow_window` 以内のほかのアプリの確定の末尾を返す。
    /// 貸し借りしないセッション（`share_context=false`）と secure には借りない・貸さない（D6）
    fn borrowed_left(&self, session: u64) -> Option<String> {
        let limits = self.limits();
        let shares = lock_mutex(&self.sessions)
            .get(&session)
            .is_some_and(|s| s.share && !s.secure);
        if !shares {
            return None;
        }
        let now = Instant::now();
        let joined: String = lock_mutex(&self.recent)
            .iter()
            .filter(|r| {
                r.session_id != session
                    && now.saturating_duration_since(r.at)
                        < limits.borrow_window.min(limits.recent_commits_ttl)
            })
            .map(|r| r.surface.as_str())
            .collect();
        (!joined.is_empty()).then(|| tail_chars(&joined, MAX_CONTEXT_CHARS).to_owned())
    }

    /// 変換の文脈を作る（セッションの状態から、ほかのアプリの確定を借りる判断までを含む）
    pub(crate) fn context_for(
        &self,
        stack: &Stack,
        session: u64,
        secure: bool,
        history: Vec<ikari_core::Commit>,
        surrounding: Option<&pb::SurroundingText>,
    ) -> Context {
        let nothing_here =
            !secure && history.is_empty() && !surrounding.is_some_and(|s| s.available);
        let borrowed = if nothing_here {
            self.borrowed_left(session)
        } else {
            None
        };
        Self::build_context_borrowing(stack, secure, history, surrounding, borrowed)
    }

    /// 変換の文脈を作る。secure は何も入れない。周辺テキストが取れないときは確定履歴を左文脈にする。
    pub(crate) fn build_context(
        stack: &Stack,
        secure: bool,
        history: Vec<ikari_core::Commit>,
        surrounding: Option<&pb::SurroundingText>,
    ) -> Context {
        Self::build_context_borrowing(stack, secure, history, surrounding, None)
    }

    /// `borrowed` は、同じ入力欄に文脈がないときだけ使う、ほかのアプリの確定（左文脈になり、`left_borrowed` が付く）
    pub(crate) fn build_context_borrowing(
        stack: &Stack,
        secure: bool,
        history: Vec<ikari_core::Commit>,
        surrounding: Option<&pb::SurroundingText>,
        borrowed: Option<String>,
    ) -> Context {
        if secure {
            return Context::new().with_secure(true);
        }
        let mut ctx = Context::new().with_profile(stack.profile.clone());
        match surrounding.filter(|s| s.available) {
            Some(s) => {
                ctx.left_text = tail_chars(&s.left, MAX_CONTEXT_CHARS).to_owned();
                ctx.right_text = head_chars(&s.right, MAX_CONTEXT_CHARS).to_owned();
            }
            None if history.is_empty() && borrowed.is_some() => {
                let left = borrowed.unwrap_or_default();
                ctx.left_text = tail_chars(&left, MAX_CONTEXT_CHARS).to_owned();
                ctx.left_borrowed = true;
            }
            None => {
                let joined: String = history.iter().map(|c| c.surface.as_str()).collect();
                ctx.left_text = tail_chars(&joined, MAX_CONTEXT_CHARS).to_owned();
            }
        }
        ctx.with_history(history)
    }
}
