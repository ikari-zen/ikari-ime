//! セッション（開始、終了、確定履歴、左文脈の組み立て）。

use ikari_core::{CarryParams, FieldLeft, LeftSources, compose_left};

use super::*;

pub(crate) struct Session {
    pub(crate) secure: bool,
    /// ログの `app=` に書く語（`diag::app_word` で確かめ済みの Bundle ID、`invalid`、`none`。メモリにだけ持つ。
    /// ADR 0034 §9-1 の 2026-10-30 の改め）。secure では None（NFR-18）
    pub(crate) app: Option<String>,
    pub(crate) history: Vec<ikari_core::Commit>,
    pub(crate) last_used: Instant,
    /// `EndSession` を受けた時刻。Some の間は休止中で、要求には答えず、同じ ID の `BeginSession` で再開する（D1）
    pub(crate) paused_at: Option<Instant>,
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
        // client_bundle_id そのものは保存しない（アプリごとのプロファイルは将来の候補）。ログの `app=` に
        // 書く語（文字の種類と長さを確かめたもの。secure では None）だけを、メモリ上のセッションに持つ。
        // 書く行は `begin_session` と `convert`（ADR 0034 §7・§9-1。2026-10-25、2026-10-30 の改め）
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
        let app = diag::session_app(&b);
        match sessions.get_mut(&b.session_id) {
            // 同じ ID で secure が同じなら、履歴を残して再開する（D1）
            Some(s) if s.secure == b.secure => {
                s.app = app;
                s.paused_at = None;
                s.last_used = now;
            }
            // secure が変わったら履歴をつながない（パスワード欄の前後）
            _ => {
                sessions.insert(
                    b.session_id,
                    Session {
                        secure: b.secure,
                        app,
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

    /// `convert` の行に足す `app=<語>`。secure、セッションがなければ None
    pub(crate) fn app_note(&self, id: u64) -> Option<String> {
        lock_mutex(&self.sessions)
            .get(&id)
            .and_then(|s| s.app.as_ref().map(|a| format!("app={a}")))
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
        let mut sessions = lock_mutex(&self.sessions);
        let Some(s) = sessions
            .get_mut(&session)
            .filter(|s| !s.secure && s.paused_at.is_none())
        else {
            return;
        };
        s.history.push(ikari_core::Commit { reading, surface });
        // 表記の合計が上限を超えたら古い方から捨てる（最後の 1 件は残す。D4）
        let mut total = history_chars(&s.history);
        let mut drop_count = 0;
        while total > limits.history_max_chars && drop_count + 1 < s.history.len() {
            total -= s.history[drop_count].surface.chars().count();
            drop_count += 1;
        }
        s.history.drain(..drop_count);
    }

    /// 休止から `paused_session_ttl` たったセッションを捨てる（見回りから）
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
    }

    /// 変換の文脈を作る（`build_context` に、今効いている `carry` の値を渡す）
    pub(crate) fn context_for(
        &self,
        stack: &Stack,
        secure: bool,
        history: Vec<ikari_core::Commit>,
        surrounding: Option<&pb::SurroundingText>,
    ) -> Context {
        Self::build_context(stack, secure, history, surrounding, &self.limits().carry)
    }

    /// 変換の文脈を作る。secure は何も入れない。左文脈は `compose_left`（ADR 0046）で、周辺テキストと
    /// 同じセッションの確定履歴から組み立てる。ほかのアプリ（ほかのセッション）の確定は入れない
    pub(crate) fn build_context(
        stack: &Stack,
        secure: bool,
        history: Vec<ikari_core::Commit>,
        surrounding: Option<&pb::SurroundingText>,
        carry: &CarryParams,
    ) -> Context {
        if secure {
            return Context::new().with_secure(true);
        }
        let mut ctx = Context::new().with_profile(stack.profile.clone());
        let joined: String = history.iter().map(|c| c.surface.as_str()).collect();
        let field = match surrounding.filter(|s| s.available) {
            Some(s) => {
                ctx.right_text = head_chars(&s.right, MAX_CONTEXT_CHARS).to_owned();
                FieldLeft::Available(tail_chars(&s.left, MAX_CONTEXT_CHARS))
            }
            None => FieldLeft::Unavailable,
        };
        let src = LeftSources {
            field,
            history: &joined,
        };
        ctx.left_text = compose_left(&src, carry, MAX_CONTEXT_CHARS).text;
        ctx.with_history(history)
    }
}
