//! 変換の要求（先行計算、変換、ページ、確定、取り消し、終了）。

use super::*;

/// Space（Convert）で、計算中の先行計算を待つ最大の時間の既定（NFR-04。Space から数える。ADR 0021 L1）。
/// `engine.convert_wait_ms` で変えられる
#[cfg(test)]
pub(crate) const DEFAULT_CONVERT_WAIT: Duration =
    Duration::from_millis(settings::DEFAULT_CONVERT_WAIT_MS as u64);

pub(crate) fn candidate_kind(origin: CandidateOrigin) -> pb::CandidateKind {
    match origin {
        CandidateOrigin::Hiragana => pb::CandidateKind::Hiragana,
        CandidateOrigin::Katakana => pb::CandidateKind::Katakana,
        CandidateOrigin::Rewriter { .. } => pb::CandidateKind::Rewriter,
        CandidateOrigin::Literal => pb::CandidateKind::Literal,
        _ => pb::CandidateKind::Dictionary,
    }
}

pub(crate) fn check_reading(reading: &str) -> Result<(), Fail> {
    if reading.is_empty() {
        return Err(Fail::bad("empty reading"));
    }
    if char_count_exceeds(reading, MAX_READING_CHARS) {
        return Err(Fail::bad("reading too long"));
    }
    Ok(())
}

/// `index` ページ目（1 ページ `page_size` 件）の候補と、次のページがあるか。
/// 変換の内部のページの大きさと独立に、生成済みの列を切り出す。
pub(crate) fn page_candidates(
    conv: &SharedConversion,
    index: usize,
    page_size: usize,
    deadline: Option<Instant>,
) -> Result<(Vec<pb::Candidate>, bool), Fail> {
    let start = index
        .checked_mul(page_size)
        .ok_or(Fail::bad("page index out of range"))?;
    let end = start + page_size;
    let mut entry = lock_mutex(conv);
    let conv = entry.whole_mut();
    // 次のページがあるかを正確に返すため、end の次の 1 件まで生成する
    let mut chunk = 0;
    while conv.generated().len() <= end {
        if conv.page_until(chunk, deadline).is_none() {
            break;
        }
        chunk += 1;
    }
    let generated = conv.generated();
    if start >= generated.len() {
        return Err(Fail::bad("page out of range"));
    }
    let candidates = generated[start..end.min(generated.len())]
        .iter()
        .map(|c| pb::Candidate {
            surface: c.surface.clone(),
            annotation: String::new(),
            kind: candidate_kind(c.origin) as i32,
        })
        .collect();
    Ok((candidates, generated.len() > end))
}

/// 確信度の閾値: LM なしの変換で、1 位と 2 位の `Candidate.score` の差がこれ以上なら confident（#69）。
/// 暫定。評価セットの測定（保守的の案）で決めた値で、診断ログの `gap=` を見て見直す。
pub(crate) const CONFIDENT_GAP_WITHOUT_LM: i32 = 5000;
/// 確信度の閾値: LM ありの変換（返事の `lm_applied`）。LM で差の目盛りが大きくなるため別にする（#69）。暫定
pub(crate) const CONFIDENT_GAP_WITH_LM: i32 = 10000;

/// 1 位と 2 位のスコアの差が閾値以上か（#69）
pub(crate) fn is_confident(gap: i32, lm_applied: bool) -> bool {
    let threshold = if lm_applied {
        CONFIDENT_GAP_WITH_LM
    } else {
        CONFIDENT_GAP_WITHOUT_LM
    };
    gap >= threshold
}

/// 読み全体の候補の、1 位と 2 位のスコアの差（`page_candidates` のあとに呼ぶ）。
/// 候補が 1 件だけ（次もない）なら None（一覧を出す意味がないので confident）
fn top_gap(conv: &SharedConversion, has_next: bool) -> Gap {
    let mut entry = lock_mutex(conv);
    match entry.whole_mut().generated() {
        [first, second, ..] => Gap::Known(second.score.saturating_sub(first.score)),
        [_] if !has_next => Gap::Single,
        _ => Gap::Unknown,
    }
}

/// 1 位と 2 位の差の求まり方
enum Gap {
    Known(i32),
    /// 候補が 1 件だけ
    Single,
    /// 候補がない（または締め切りで 2 件目まで届かなかった）
    Unknown,
}

impl Inner {
    /// 文脈の番号と周辺テキストを記録し、番号が進んでいたら `ctx_change` の行を書く。
    /// 返すのは要求の行に添える `ctx_rev=N`（固定の語と数だけ。NFR-19）。secure では呼ばない
    fn trace_context(
        &self,
        session: u64,
        rev: u64,
        surrounding: Option<&pb::SurroundingText>,
        req: &str,
        reading: &str,
    ) -> String {
        let reading = reading.chars().count();
        let change = lock_mutex(&self.ctx_trace).observe(session, rev, surrounding, req, reading);
        if let Some(note) = change {
            self.log
                .record_note("ctx_change", 0, self.lm_state_name(), None, Some(&note));
        }
        // 周辺テキストが取れなかった要求だけ `ctx=0` を足す（回数の集計用。ADR 0034 §4.3）
        if surrounding.is_some_and(|s| s.available) {
            format!("ctx_rev={rev}")
        } else {
            format!("ctx_rev={rev} ctx=0")
        }
    }

    /// 返事と、ログの注記（`ctx_rev`。secure と空の読みでは None）
    pub(crate) fn prefetch(
        &self,
        p: pb::Prefetch,
    ) -> Result<(envelope::Body, Option<String>), Fail> {
        let ack = Ok((envelope::Body::Ack(pb::Ack {}), None));
        if char_count_exceeds(&p.reading, MAX_READING_CHARS) {
            return Err(Fail::bad("reading too long"));
        }
        let (secure, history) = self.session_info(p.session_id)?;
        // 読みが変わった。前の変換の文節の先読みは要らない
        self.lookahead.forget_session(p.session_id);
        // 秘密の入力は、計算も保存もしない（NFR-18）。読みが空になったら古い依頼を捨てる
        if secure || p.reading.is_empty() {
            if let Some(old) = self.queue.forget_session(p.session_id) {
                lock_mutex(&self.cache).remove(&old);
            }
            return ack;
        }
        let trace = self.trace_context(
            p.session_id,
            p.context_rev,
            p.context.as_ref(),
            "prefetch",
            &p.reading,
        );
        let ack = Ok((envelope::Body::Ack(pb::Ack {}), Some(trace)));
        // アイドル解放のあとの最初の Prefetch で、読み込みを別スレッドで始める（読み込み済みなら何もしない）
        self.ensure_warm();
        let key = CacheKey {
            session: p.session_id,
            reading: p.reading,
            context_rev: p.context_rev,
        };
        if lock_mutex(&self.cache).find(&key).is_some() {
            return ack;
        }
        let stack = self.stack();
        let ctx = self.context_for(&stack, p.session_id, false, history, p.context.as_ref());
        let entry = CacheEntry::new(key);
        lock_mutex(&self.cache).insert(Arc::clone(&entry));
        if let Some(replaced) = self.queue.submit(entry, ctx) {
            lock_mutex(&self.cache).remove(&replaced);
        }
        ack
    }

    /// 返事と、ログの注記（キャッシュの使われ方と LM の有無。固定の語だけ）を返す
    pub(crate) fn convert(&self, c: pb::Convert) -> Result<(envelope::Body, Option<String>), Fail> {
        let started = Instant::now();
        check_reading(&c.reading)?;
        if c.page_size > MAX_PAGE_SIZE {
            return Err(Fail::bad("page_size out of range"));
        }
        let page_size = if c.page_size == 0 {
            MAX_PAGE_SIZE
        } else {
            c.page_size
        } as usize;
        let (secure, history) = self.session_info(c.session_id)?;
        let stack = self.stack();
        let trace = (!secure).then(|| {
            self.trace_context(
                c.session_id,
                c.context_rev,
                c.context.as_ref(),
                "convert",
                &c.reading,
            )
        });

        let (cached, outcome) = if secure {
            (None, CacheOutcome::Miss(MissReason::None))
        } else {
            let key = CacheKey {
                session: c.session_id,
                reading: c.reading.clone(),
                context_rev: c.context_rev,
            };
            let (entry, reason) = {
                let mut cache = lock_mutex(&self.cache);
                match cache.find(&key) {
                    Some(e) => (Some(e), MissReason::None),
                    None => (None, cache.miss_reason(&key)),
                }
            };
            match entry {
                None => (None, CacheOutcome::Miss(reason)),
                Some(e) => {
                    // 計算中なら、Space から `convert_wait` まで待つ
                    let computing = e.is_computing();
                    let hit = e.wait_ready(Instant::now() + self.convert_wait);
                    let outcome = match (computing, hit.is_some()) {
                        (false, true) => CacheOutcome::Hit,
                        (true, true) => CacheOutcome::WaitHit,
                        (true, false) => CacheOutcome::WaitMiss,
                        // 計算に失敗した、または捨てられたもの
                        (false, false) => CacheOutcome::Miss(MissReason::None),
                    };
                    (hit, outcome)
                }
            }
        };
        let (conv, lm_applied) = match cached {
            Some(hit) => hit,
            None => {
                // 先行計算がない、または間に合わなかった。LM を使わずにその場で変換する
                let ctx =
                    self.context_for(&stack, c.session_id, secure, history, c.context.as_ref());
                let conv = stack
                    .converter
                    .convert_until(&c.reading, &ctx, Some(Instant::now()));
                let seg = self.seam(&stack).segment(conv);
                (Arc::new(Mutex::new(SegEntry::new(seg))), false)
            }
        };

        let (candidates, has_next) = page_candidates(&conv, 0, page_size, Some(Instant::now()))?;
        let gap = top_gap(&conv, has_next);
        let (confident, gap_note) = match gap {
            Gap::Known(g) => (is_confident(g, lm_applied), g.to_string()),
            Gap::Single => (true, "na".to_string()),
            Gap::Unknown => (false, "na".to_string()),
        };
        let conversion_id = self.next_conversion.fetch_add(1, Ordering::Relaxed);
        let layout = {
            let mut entry = lock_mutex(&conv);
            let layout = entry.layout().clone();
            // 1 文節なら、文節 0 の列は読み全体の列そのもの（ADR 0006 §2.3）
            let (page0, page0_next) = if layout.spans.len() == 1 {
                (candidates.clone(), has_next)
            } else {
                entry.segment_page(0, 0, page_size, &[], Some(Instant::now()))?
            };
            layout_reply(
                conversion_id,
                &layout,
                0,
                Some(pb::SegmentPageReply {
                    segment: 0,
                    page_index: 0,
                    candidates: page0,
                    has_next: page0_next,
                    lm_applied,
                    engine_micros: micros_u32(started),
                }),
                micros_u32(started),
            )
        };
        // 前の変換の先読みは捨てる。2 文節以上なら、返事のあとに文節 1 以降の 1 ページ目を裏で作る
        // （秘密の入力は先読みしない。ADR 0006 §3.4、§5.3）
        self.lookahead.forget_session(c.session_id);
        if !secure && layout.segments.len() >= 2 {
            self.lookahead.submit(LookJob::new(
                c.session_id,
                Arc::clone(&conv),
                page_size,
                layout.layout_rev,
            ));
        }
        lock_mutex(&self.conversions).insert(conversion_id, c.session_id, conv, page_size);
        let reply = envelope::Body::ConvertReply(pb::ConvertReply {
            conversion_id,
            page_index: 0,
            candidates,
            has_next,
            lm_applied,
            confident,
            engine_micros: micros_u32(started),
            layout: Some(layout),
        });
        let note = format!(
            "{} gap={gap_note} confident={}",
            outcome.log_note(lm_applied),
            u8::from(confident)
        );
        Ok((
            reply,
            Some(match trace {
                Some(trace) => format!("{note} {trace}"),
                None => note,
            }),
        ))
    }

    pub(crate) fn page(&self, p: pb::PageRequest) -> Result<envelope::Body, Fail> {
        let started = Instant::now();
        let index = usize::try_from(p.index).unwrap_or(usize::MAX);
        if index > MAX_PAGE_INDEX {
            return Err(Fail::bad("page index out of range"));
        }
        let (conv, page_size, _) = lock_mutex(&self.conversions)
            .get(p.conversion_id)
            .ok_or(UNKNOWN_CONVERSION)?;
        let (budget, layout) = {
            let entry = lock_mutex(&conv);
            (
                self.lm_budget.for_reading(entry.whole().reading()),
                entry.layout().clone(),
            )
        };
        let before = self.lm_stats();
        // 次のページの生成には、そのときまでに届いた LM の結果を使ってよい
        let (candidates, has_next) =
            page_candidates(&conv, index, page_size, Some(Instant::now() + budget))?;
        Ok(envelope::Body::PageReply(pb::ConvertReply {
            conversion_id: p.conversion_id,
            page_index: p.index,
            candidates,
            has_next,
            lm_applied: self.lm_applied_since(before),
            confident: false,
            engine_micros: micros_u32(started),
            // 区切りだけを添える（文節ごとのページは SegmentPage で引く）
            layout: Some(layout_reply(
                p.conversion_id,
                &layout,
                0,
                None,
                micros_u32(started),
            )),
        }))
    }

    /// 返事と、ログの注記（確定した候補が Rewriter のものなら `rw=<名前>`。secure では None）
    pub(crate) fn commit(&self, c: pb::Commit) -> Result<(envelope::Body, Option<String>), Fail> {
        let (conv, _, session) = lock_mutex(&self.conversions)
            .get(c.conversion_id)
            .ok_or(UNKNOWN_CONVERSION)?;
        let index = usize::try_from(c.candidate_index).unwrap_or(usize::MAX);
        let (event, reading, surface) = {
            let entry = lock_mutex(&conv);
            let conv = entry.whole();
            let candidate = conv
                .generated()
                .get(index)
                .ok_or(Fail::bad("candidate index out of range"))?;
            (
                conv.commit(index),
                conv.reading().to_owned(),
                candidate.surface.clone(),
            )
        };
        lock_mutex(&self.conversions).remove(c.conversion_id);
        // secure の確定は `commit` が None を返す（学習も履歴もしない。NFR-18）
        let mut note = None;
        if let Some(event) = event {
            note = diag::rewriter_note(std::iter::once(&event.candidate.origin));
            let stack = self.stack();
            if let Some(store) = &stack.learning {
                store.record(&event, stack.vectors.as_deref(), today_unix_day());
                lock_mutex(&self.save).dirty = true;
            }
            lock_mutex(&self.ctx_trace).note_commit(session, surface.chars().count());
            self.push_history(session, reading, surface);
        }
        Ok((envelope::Body::Ack(pb::Ack {}), note))
    }

    pub(crate) fn commit_text(&self, c: pb::CommitText) -> Result<envelope::Body, Fail> {
        if char_count_exceeds(&c.reading, MAX_READING_CHARS)
            || char_count_exceeds(&c.surface, MAX_SURFACE_CHARS)
        {
            return Err(Fail::bad("text too long"));
        }
        let (secure, _) = self.session_info(c.session_id)?;
        if !secure && !c.surface.is_empty() {
            lock_mutex(&self.ctx_trace).note_commit(c.session_id, c.surface.chars().count());
            self.push_history(c.session_id, c.reading, c.surface);
        }
        Ok(envelope::Body::Ack(pb::Ack {}))
    }

    pub(crate) fn cancel(&self, c: pb::Cancel) -> envelope::Body {
        lock_mutex(&self.conversions).remove(c.conversion_id);
        envelope::Body::Ack(pb::Ack {})
    }

    /// 保存して Ack を返す。プロセスを終わらせるのは XPC サービスの役目（返事を送ってから exit する）。
    pub(crate) fn shutdown(&self) -> envelope::Body {
        self.save_now();
        envelope::Body::Ack(pb::Ack {})
    }
}
