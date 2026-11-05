//! 文節ごとの削除（ADR 0050 §3）。`clause.delete`（すぐ消す）と `clause.delete_preview`（対象を見てから消す）。
//!
//! 状態ごと（読みの入力中、変換中、確定後）に対象を決める。単位の規則は `crate::clause`（純粋な関数）。
//! プレビューは状態（`InputState`）を増やさず、`delete_preview` で持つ（確定後は `Idle` のまま）。

use std::ops::Range;

use super::*;
use crate::clause::{self, Dir, Sentence};

/// 確定後の削除の対象。プレビューの間は、単位を左右へ足していく。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommittedPreview {
    sentence: Sentence,
    bounds: Vec<usize>,
    /// 作った単位（昇順で隣り合う）。`target` が今の対象
    units: Vec<Range<usize>>,
    target: usize,
    min: usize,
    /// 削除の前の文書の長さ（ホストが返した値。0 = 取れていない）。取り消しの検査の予想に使う
    document_length: Utf16Len,
}

/// 文節削除の 1 段（ADR 0053 §3.1）。外には出さない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UndoStep {
    /// 確定後。`removed` を `start` に書き戻す
    Committed {
        /// 消した文字（表示の文字列そのまま）
        removed: String,
        /// 消した範囲の始まり（UTF-16。消したあとのキャレットの予想）
        start: Utf16Len,
        /// 消したあとの、`start` の左の文字（切り出した文の中だけ）
        left: String,
        /// 消したあとの、`start` の右の文字（同上）
        right: String,
        /// 消したあとの文書の長さの予想（消す前が取れたときだけ）
        document_length: Option<Utf16Len>,
    },
    /// 読みの入力中。消す前の合成器の中身に戻す
    Composing {
        kana: String,
        pending: String,
        /// 消したあとの合成器の中身
        after: (String, String),
    },
    /// 変換中。消す前の変換の読み全体を、読みの入力中として戻す
    Converting {
        reading: String,
        after: ConvertingAfter,
    },
}

/// 変換中の削除のあとの状態（取り消しの検査に使う）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConvertingAfter {
    /// 全部消えて idle になった
    Idle,
    /// 文節が残っている
    Segments {
        conversion_id: u64,
        layout_rev: u32,
        reading: String,
    },
}

/// 確定後の段の書き戻しの検査（ADR 0053 §3.2）。キャレットと前後の文字と文書の長さが、消した直後のままか。
pub(super) fn committed_step_holds(step: &UndoStep, surrounding: &SurroundingText) -> bool {
    let UndoStep::Committed {
        start,
        left,
        right,
        document_length,
        ..
    } = step
    else {
        return false;
    };
    let Some(caret) = surrounding.caret else {
        return false;
    };
    if caret.length != Utf16Len::ZERO || caret.location != *start {
        return false;
    }
    // ホストの左右は短いことがある（文書の端、窓の切り出し）。重なる部分で比べる
    let left_ok = if surrounding.left.len() >= left.len() {
        surrounding.left.ends_with(left.as_str())
    } else {
        left.ends_with(surrounding.left.as_str())
    };
    let right_ok = if surrounding.right.len() >= right.len() {
        surrounding.right.starts_with(right.as_str())
    } else {
        right.starts_with(surrounding.right.as_str())
    };
    if !left_ok || !right_ok {
        return false;
    }
    match document_length {
        Some(expected) if surrounding.document_length != Utf16Len::ZERO => {
            surrounding.document_length == *expected
        }
        _ => true,
    }
}

impl CommittedPreview {
    /// 今の対象を消したあとの取り消しの 1 段
    fn undo_step(&self) -> UndoStep {
        let range = self.target_range();
        let removed = self.sentence.slice(&range);
        let removed_length = Utf16Len::of(&removed);
        let total = grapheme_count(&self.sentence.text);
        UndoStep::Committed {
            start: self.utf16_range().location,
            left: clause::grapheme_slice(&self.sentence.text, 0, range.start),
            right: clause::grapheme_slice(&self.sentence.text, range.end, total),
            document_length: (self.document_length != Utf16Len::ZERO).then(|| {
                Utf16Len(
                    self.document_length
                        .get()
                        .saturating_sub(removed_length.get()),
                )
            }),
            removed,
        }
    }
}

impl CommittedPreview {
    fn target_range(&self) -> Range<usize> {
        self.units[self.target].clone()
    }

    fn utf16_range(&self) -> Utf16Range {
        self.sentence.utf16_range(&self.target_range())
    }

    fn marked(&self) -> ShellAction {
        ShellAction::SetMarkedText(MarkedText {
            replacement_range: Some(self.utf16_range()),
            delete_target: true,
            ..MarkedText::plain(self.sentence.slice(&self.target_range()))
        })
    }

    /// 今の対象を元の表記に戻す（`Restore`）
    fn restore(&self) -> ShellAction {
        ShellAction::Restore {
            text: self.sentence.slice(&self.target_range()),
            replacing: self.utf16_range(),
        }
    }

    /// 対象を左右へ動かす。端では止まる（動けなければ `None`）
    fn step(&mut self, dir: Dir) -> Option<()> {
        match dir {
            Dir::Left if self.target > 0 => self.target -= 1,
            Dir::Right if self.target + 1 < self.units.len() => self.target += 1,
            Dir::Left => {
                let edge = self.units.first()?.start;
                let unit = clause::unit_range(
                    self.sentence_len(),
                    &self.bounds,
                    edge,
                    self.min,
                    Dir::Left,
                )?;
                self.units.insert(0, unit);
            }
            Dir::Right => {
                let edge = self.units.last()?.end;
                let unit = clause::unit_range(
                    self.sentence_len(),
                    &self.bounds,
                    edge,
                    self.min,
                    Dir::Right,
                )?;
                self.units.push(unit);
                self.target = self.units.len() - 1;
            }
        }
        Some(())
    }

    fn sentence_len(&self) -> usize {
        grapheme_count(&self.sentence.text)
    }
}

/// 削除のプレビューのモード（ADR 0050 §3.4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeletePreview {
    /// 読みの入力中。`units` は表示（かな + 未確定のローマ字）の書記素の範囲で、全体を覆う
    Composing {
        units: Vec<Range<usize>>,
        target: usize,
    },
    /// 変換中。対象はフォーカスしている文節
    Converting,
    /// 確定後
    Committed(CommittedPreview),
}

impl InputStateMachine {
    /// `clause.delete`・`clause.delete_preview` のキー。扱わない状態（asciiRun など）は `None`（今までの扱いに任せる）
    pub(super) fn start_clause_delete(
        &mut self,
        action: KeymapAction,
        text: &mut dyn TextAccess,
    ) -> Option<Vec<ShellAction>> {
        let preview = action == KeymapAction::ClauseDeletePreview;
        match &self.state {
            InputState::Idle => Some(self.idle_delete(preview, text)),
            InputState::Composing { .. } => Some(self.composing_delete(preview)),
            InputState::Converting(_) => Some(if preview {
                self.converting_preview()
            } else {
                self.converting_delete()
            }),
            _ => None,
        }
    }

    fn min_chars(&self) -> usize {
        to_usize(self.settings.clause_min_chars).max(1)
    }

    // MARK: - 確定後

    /// 確定後の削除の対象を作る。断る条件（§7）に当たれば `None`（アプリに渡す）
    fn committed_target(&mut self, text: &mut dyn TextAccess) -> Option<CommittedPreview> {
        if clause::is_denied(&self.session.client_bundle_id) {
            return None;
        }
        let surrounding = text.surrounding()?;
        let caret = surrounding.caret?;
        if caret.length != Utf16Len::ZERO
            || !is_available(&surrounding.left, surrounding.document_length)
            || !clause::left_matches_caret(&surrounding, caret)
        {
            return None;
        }
        // 直前が英字なら、アプリの単語の削除に任せる（ADR 0053 §6.3。エンジンを呼ばない）
        if clause::ends_with_ascii(&surrounding.left) {
            return None;
        }
        let sentence = clause::cut_sentence(
            &surrounding.left,
            &surrounding.right,
            caret.location,
            MAX_RECONVERT_CHARACTERS,
        );
        if sentence.cursor == 0 || sentence.text.chars().all(char::is_control) {
            return None;
        }
        let before = grapheme_count(&surrounding.left).saturating_sub(sentence.cursor);
        let context = ContextText {
            left: prefix_graphemes(&surrounding.left, before),
            right: String::new(),
            available: true,
        }
        .truncated();
        let request = v1::Reconvert {
            session_id: self.session.session_id,
            surface: sentence.text.clone(),
            context: Some(context.to_proto()),
            page_size: ENGINE_PAGE_SIZE,
        };
        let timeout = timeouts::convert_timeout(grapheme_count(&sentence.text));
        let reply = self.engine.reconvert(request, timeout).ok()?;
        let layout = SegmentLayoutInfo::from_reply(&reply);
        // 区切りだけを読む。エンジンに変換の状態を残さない
        self.engine.cancel(layout.conversion_id);
        if layout.segments.is_empty() || layout.joined_surface() != sentence.text {
            return None;
        }
        let bounds = clause::boundaries(
            layout
                .segments
                .iter()
                .map(|segment| grapheme_count(&segment.surface)),
        );
        let min = self.min_chars();
        let unit = clause::unit_range(
            grapheme_count(&sentence.text),
            &bounds,
            sentence.cursor,
            min,
            Dir::Left,
        )?;
        Some(CommittedPreview {
            sentence,
            bounds,
            units: vec![unit],
            target: 0,
            min,
            document_length: surrounding.document_length,
        })
    }

    fn idle_delete(&mut self, preview: bool, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        let Some(target) = self.committed_target(text) else {
            return vec![ShellAction::PassThrough];
        };
        if preview {
            let marked = target.marked();
            self.delete_preview = Some(DeletePreview::Committed(target));
            return vec![marked];
        }
        self.clause_undo.push(target.undo_step());
        vec![ShellAction::DeleteText {
            range: target.utf16_range(),
        }]
    }

    // MARK: - 読みの入力中

    /// 読みの文節の境界。区切りを返さないエンジンは 1 文節とみなす。取れない（時間切れ、食い違い）ときは `None`
    fn composing_bounds(&mut self, kana: &str) -> Option<Vec<usize>> {
        let count = grapheme_count(kana);
        if count == 0 {
            return Some(vec![0]);
        }
        if count > MAX_READING_CHARACTERS {
            return None;
        }
        let request = v1::Convert {
            session_id: self.session.session_id,
            reading: kana.to_string(),
            context: Some(self.context_for_request().to_proto()),
            context_rev: self.context_rev,
            page_size: ENGINE_PAGE_SIZE,
        };
        let reply = self
            .engine
            .convert(request, timeouts::convert_timeout(count))
            .ok()?;
        // 区切りだけを読む
        self.engine.cancel(reply.conversion_id);
        let Some(layout) = reply.layout.as_ref().map(SegmentLayoutInfo::from_reply) else {
            return Some(vec![0, count]);
        };
        if layout.segments.is_empty() || layout.joined_reading() != kana {
            return None;
        }
        Some(clause::boundaries(
            layout
                .segments
                .iter()
                .map(|segment| grapheme_count(&segment.reading)),
        ))
    }

    /// 読みの入力中の単位（表示の全体を、右から左へ作って昇順にしたもの）
    fn composing_units(&mut self) -> Option<Vec<Range<usize>>> {
        let kana = self.composer.kana().to_string();
        let total = grapheme_count(&kana) + grapheme_count(self.composer.pending());
        let bounds = self.composing_bounds(&kana)?;
        let min = self.min_chars();
        let mut units = Vec::new();
        let mut edge = total;
        while let Some(unit) = clause::unit_range(total, &bounds, edge, min, Dir::Left) {
            edge = unit.start;
            units.push(unit);
        }
        units.reverse();
        Some(units)
    }

    fn composing_delete(&mut self, preview: bool) -> Vec<ShellAction> {
        let Some(units) = self.composing_units() else {
            return vec![ShellAction::Consume];
        };
        let Some(last) = units.len().checked_sub(1) else {
            return vec![ShellAction::Consume];
        };
        if preview {
            let marked = self.composing_preview_marked(&units, last);
            self.delete_preview = Some(DeletePreview::Composing {
                units,
                target: last,
            });
            return vec![marked];
        }
        self.remove_reading_unit(&units[last].clone())
    }

    fn composing_preview_marked(&self, units: &[Range<usize>], target: usize) -> ShellAction {
        let shown = self.composer_display();
        ShellAction::SetMarkedText(MarkedText {
            segments: units
                .iter()
                .map(|unit| Utf16Len::of(&clause::grapheme_slice(&shown, unit.start, unit.end)))
                .collect(),
            focused: Some(to_u32(target)),
            delete_target: true,
            ..MarkedText::plain(shown)
        })
    }

    /// 表示の `unit` を読みから取り除く。未確定のローマ字は、末尾の単位に含まれるときだけ一緒に消える。
    /// かな入力の合成器には読みを入れ直せないので、残りがあるときは何もしない
    fn remove_reading_unit(&mut self, unit: &Range<usize>) -> Vec<ShellAction> {
        let kana = self.composer.kana().to_string();
        let pending = self.composer.pending().to_string();
        let kana_count = grapheme_count(&kana);
        let total = kana_count + grapheme_count(&pending);
        let rest = format!(
            "{}{}",
            clause::grapheme_slice(&kana, 0, unit.start),
            clause::grapheme_slice(&kana, unit.end, kana_count)
        );
        let kept_pending = if unit.end < total {
            pending.clone()
        } else {
            String::new()
        };
        let unchanged_input = rest.is_empty() && kept_pending.is_empty();
        if !unchanged_input && self.settings.input_method == crate::types::InputMethodKind::Kana {
            return vec![ShellAction::Consume];
        }
        self.composer.clear();
        if !unchanged_input {
            self.feed(&rest);
            self.feed(&kept_pending);
        }
        // かな入力の合成器には読みを入れ直せないので、取り消しの段はローマ字入力のときだけ積む
        if self.settings.input_method == crate::types::InputMethodKind::Romaji {
            self.clause_undo.push(UndoStep::Composing {
                kana,
                pending,
                after: (
                    self.composer.kana().to_string(),
                    self.composer.pending().to_string(),
                ),
            });
        }
        self.refresh_composing()
    }

    // MARK: - 変換中

    /// 変換中の文節をすべて消して idle に戻る
    fn clear_converting(&mut self, current: &ConvertingState) -> Vec<ShellAction> {
        self.push_converting_undo(current, ConvertingAfter::Idle);
        if !current.is_fallback {
            self.engine.cancel(current.conversion_id);
        }
        let displayed = Utf16Len::of(&marked_text(current).text);
        self.reset_composition(None, None);
        let mut actions = vec![ShellAction::HideCandidates];
        match &current.origin {
            ConversionOrigin::Reconversion { replacing, .. } => {
                actions.push(ShellAction::Restore {
                    text: String::new(),
                    replacing: Utf16Range {
                        location: replacing.location,
                        length: displayed,
                    },
                });
            }
            ConversionOrigin::Composing | ConversionOrigin::Ascii => actions.push(clear_mark()),
        }
        actions
    }

    /// 文節のモードの `[first, first + count)` を取り除く。全部なら idle に戻る。失敗は `Consume`（状態は変えない）
    fn remove_converting_segments(&mut self, first: usize, count: usize) -> Vec<ShellAction> {
        let Some(current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        if count == 0 || first + count > current.segments.len() {
            return vec![ShellAction::Consume];
        }
        if count == current.segments.len() {
            return self.clear_converting(&current);
        }
        let request = v1::RemoveSegments {
            conversion_id: current.conversion_id,
            layout_rev: current.layout_rev,
            first: to_u32(first),
            count: to_u32(count),
        };
        let timeout = timeouts::convert_timeout(grapheme_count(&current.reading));
        let Ok(reply) = self.engine.remove_segments(request, timeout) else {
            return vec![ShellAction::Consume];
        };
        let layout = SegmentLayoutInfo::from_reply(&reply);
        let remaining: Vec<ConvertingSegment> = current
            .segments
            .iter()
            .enumerate()
            .filter(|(index, _)| *index < first || *index >= first + count)
            .map(|(_, segment)| segment.clone())
            .collect();
        // 残りの文節は変わっていないはず（変わっていたら信用しない）
        let same_readings = layout.segments.len() == remaining.len()
            && layout
                .segments
                .iter()
                .zip(&remaining)
                .all(|(new, old)| new.reading == old.reading);
        let Some((built, focused)) =
            converting_segments(&layout, None, false).filter(|_| same_readings)
        else {
            return vec![ShellAction::Consume];
        };
        let mut next = current.clone();
        next.segments = remaining;
        // 取り除いたあとの文節の選択は保つ。まだ候補を持たない文節（新しいフォーカス）は、返事の 1 ページ目を使う
        for (segment, new) in next.segments.iter_mut().zip(built) {
            if segment.pages.is_empty() {
                segment.pages = new.pages;
            }
        }
        next.focused = focused;
        next.layout_rev = layout.layout_rev;
        next.conversion_id = layout.conversion_id;
        next.reading = layout.joined_reading();
        next.whole_layout = None;
        next.candidates_hidden = false;
        if next.segments[focused].pages.is_empty() {
            match self.fetch_page(&next, focused, 0) {
                Some(page) => next.segments[focused].pages = vec![page],
                None => return vec![ShellAction::Consume],
            }
        }
        next.sync_focused();
        self.push_converting_undo(
            &current,
            ConvertingAfter::Segments {
                conversion_id: next.conversion_id,
                layout_rev: next.layout_rev,
                reading: next.reading.clone(),
            },
        );
        self.mode.use_segment_api = true;
        self.mode.layout_changed = true;
        self.state = InputState::Converting(next.clone());
        vec![
            ShellAction::SetMarkedText(marked_text(&next)),
            ShellAction::ShowCandidates(self.panel_model(&next)),
        ]
    }

    fn converting_delete(&mut self) -> Vec<ShellAction> {
        let Some(current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        // 区切りのない変換（殻だけの候補、古いエンジン）は、全体が 1 つの単位
        if current.is_fallback || !self.mode.can_segment {
            return self.clear_converting(&current);
        }
        let Some(layout) = current.whole_layout.clone() else {
            // 文節のモード: フォーカスしている文節そのもの（最小文字数で広げない）
            return self.remove_converting_segments(current.focused, 1);
        };
        // 全体の候補を出している間: 文節のモードに入って、末尾の単位を取り除く
        let previous_mode = self.mode;
        self.mode.use_segment_api = true;
        let Some(next) = self.segment_mode_state(&current, &layout, true) else {
            return self.restore_segment_mode(previous_mode, current);
        };
        let bounds = clause::boundaries(
            next.segments
                .iter()
                .map(|segment| grapheme_count(segment.displayed_surface())),
        );
        let total = bounds.last().copied().unwrap_or(0);
        let first = clause::unit_range(total, &bounds, total, self.min_chars(), Dir::Left)
            .and_then(|unit| bounds.iter().position(|bound| *bound == unit.start));
        let Some(first) = first else {
            return self.restore_segment_mode(previous_mode, current);
        };
        let count = next.segments.len() - first;
        self.state = InputState::Converting(next);
        let actions = self.remove_converting_segments(first, count);
        if actions == [ShellAction::Consume] {
            return self.restore_segment_mode(previous_mode, current);
        }
        actions
    }

    fn converting_preview(&mut self) -> Vec<ShellAction> {
        let Some(current) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        if !current.is_fallback
            && self.mode.can_segment
            && let Some(layout) = current.whole_layout.clone()
        {
            // 全体の候補を出している間は、← と同じ手順で文節のモードに入り、末尾の文節を対象にする
            let entered = self.enter_segment_mode(KeymapAction::SegmentFocusLeft, current, &layout);
            if entered == [ShellAction::Consume] {
                return entered;
            }
        }
        let Some(now) = self.converting().cloned() else {
            return vec![ShellAction::Consume];
        };
        self.delete_preview = Some(DeletePreview::Converting);
        vec![
            ShellAction::SetMarkedText(delete_target_marked(&now)),
            ShellAction::HideCandidates,
        ]
    }

    // MARK: - 元に戻す（ADR 0053）

    /// 変換中の削除の取り消しの段を積む。再変換とアスキーの変換は、Esc で戻せるので積まない。
    /// かな入力の合成器には読みを入れ直せないので、ローマ字入力のときだけ積む
    fn push_converting_undo(&mut self, current: &ConvertingState, after: ConvertingAfter) {
        if !matches!(current.origin, ConversionOrigin::Composing)
            || self.settings.input_method != crate::types::InputMethodKind::Romaji
        {
            return;
        }
        self.clause_undo.push(UndoStep::Converting {
            reading: current.reading.clone(),
            after,
        });
    }

    /// 取り消しの覚えを捨てる
    pub fn forget_clause_undo(&mut self) {
        self.clause_undo.clear();
    }

    /// `clause.undo` のキー。覚えがない、または状態が消した直後と食い違うときは `None`（割り当てがないのと同じ扱い）。
    /// 確定後の段で検査に落ちたときは、覚えを全部捨てて `PassThrough`
    pub(super) fn clause_undo_key(
        &mut self,
        text: &mut dyn TextAccess,
    ) -> Option<Vec<ShellAction>> {
        let step = self.clause_undo.last()?.clone();
        let actions = match &step {
            UndoStep::Committed { removed, .. } => {
                if self.state != InputState::Idle {
                    None
                } else if text
                    .surrounding()
                    .is_some_and(|surrounding| committed_step_holds(&step, &surrounding))
                {
                    // 学習・確定履歴には送らない（削除と書き戻しは変換の証拠ではない）
                    Some(vec![ShellAction::Commit {
                        text: removed.clone(),
                    }])
                } else {
                    self.clause_undo.clear();
                    return Some(vec![ShellAction::PassThrough]);
                }
            }
            UndoStep::Composing {
                kana,
                pending,
                after,
            } => self
                .composing_matches(after)
                .then(|| self.restore_composing(kana, pending, text)),
            UndoStep::Converting { reading, after } => self
                .converting_matches(after)
                .then(|| self.restore_converting(reading, text)),
        };
        match actions {
            Some(actions) => {
                self.clause_undo.pop();
                Some(actions)
            }
            None => {
                self.clause_undo.clear();
                None
            }
        }
    }

    /// 今の状態が、読みの削除のあと（`after`）のままか
    fn composing_matches(&self, after: &(String, String)) -> bool {
        match &self.state {
            InputState::Idle => after.0.is_empty() && after.1.is_empty(),
            InputState::Composing { kana, pending } => *kana == after.0 && *pending == after.1,
            _ => false,
        }
    }

    /// 今の状態が、変換中の削除のあとのままか。直前の取り消しで読みの入力中に戻したあとも含む
    fn converting_matches(&self, after: &ConvertingAfter) -> bool {
        match (after, &self.state) {
            (ConvertingAfter::Idle, InputState::Idle) => true,
            (
                ConvertingAfter::Segments {
                    conversion_id,
                    layout_rev,
                    ..
                },
                InputState::Converting(current),
            ) => current.conversion_id == *conversion_id && current.layout_rev == *layout_rev,
            (
                ConvertingAfter::Segments { reading, .. },
                InputState::Composing { kana, pending },
            ) => kana == reading && pending.is_empty(),
            _ => false,
        }
    }

    /// 合成器を `kana` と `pending` に入れ直して、読みの入力中にする
    fn restore_composing(
        &mut self,
        kana: &str,
        pending: &str,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        self.composer.clear();
        self.feed(kana);
        self.feed(pending);
        self.composing_started(text)
    }

    /// 変換を取りやめて、その読み全体を読みの入力中にする
    fn restore_converting(&mut self, reading: &str, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        let was_converting = match self.converting().cloned() {
            Some(current) => {
                if !current.is_fallback {
                    self.engine.cancel(current.conversion_id);
                }
                self.reset_composition(None, None);
                true
            }
            None => false,
        };
        let mut actions = Vec::new();
        if was_converting {
            actions.push(ShellAction::HideCandidates);
        }
        actions.extend(self.restore_composing(reading, "", text));
        actions
    }

    // MARK: - プレビューのモードの中のキー

    /// プレビューのモードの中のキー（ADR 0050 §3.4）。← → Backspace Esc 以外は、抜けてからふつうに扱う
    pub(super) fn handle_delete_preview(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Vec<ShellAction> {
        let Some(preview) = self.delete_preview.clone() else {
            return self.dispatch(input, text);
        };
        match &input.key {
            KeyKind::Left => self.preview_step(preview, Dir::Left),
            KeyKind::Right => self.preview_step(preview, Dir::Right),
            KeyKind::Backspace => self.preview_delete(preview),
            KeyKind::Escape => self.leave_preview(preview),
            _ => {
                let mut actions = self.leave_preview(preview);
                actions.extend(self.dispatch(input, text));
                actions
            }
        }
    }

    /// 何もせずに抜ける。表示を元の状態に戻す動作を返す
    fn leave_preview(&mut self, preview: DeletePreview) -> Vec<ShellAction> {
        self.delete_preview = None;
        match preview {
            DeletePreview::Committed(target) => vec![target.restore()],
            DeletePreview::Composing { .. } => self.refresh_composing(),
            DeletePreview::Converting => match self.converting().cloned() {
                Some(current) => vec![
                    ShellAction::SetMarkedText(marked_text(&current)),
                    ShellAction::ShowCandidates(self.panel_model(&current)),
                ],
                None => vec![ShellAction::Consume],
            },
        }
    }

    fn preview_step(&mut self, preview: DeletePreview, dir: Dir) -> Vec<ShellAction> {
        match preview {
            DeletePreview::Committed(mut target) => {
                let before = target.restore();
                if target.step(dir).is_none() {
                    return vec![ShellAction::Consume];
                }
                let marked = target.marked();
                self.delete_preview = Some(DeletePreview::Committed(target));
                vec![before, marked]
            }
            DeletePreview::Composing { units, target } => {
                let moved = match dir {
                    Dir::Left => target.checked_sub(1),
                    Dir::Right => Some(target + 1).filter(|next| *next < units.len()),
                };
                let Some(moved) = moved else {
                    return vec![ShellAction::Consume];
                };
                let marked = self.composing_preview_marked(&units, moved);
                self.delete_preview = Some(DeletePreview::Composing {
                    units,
                    target: moved,
                });
                vec![marked]
            }
            DeletePreview::Converting => {
                let Some(current) = self.converting() else {
                    return vec![ShellAction::Consume];
                };
                let in_range = match dir {
                    Dir::Left => current.focused > 0,
                    Dir::Right => current.focused + 1 < current.segments.len(),
                };
                let delta = if dir == Dir::Left { -1 } else { 1 };
                // 端で止まる（ふだんの ←/→ のように反対の端へ回らない）
                if !in_range || self.move_focus(delta).is_none() {
                    return vec![ShellAction::Consume];
                }
                match self.converting() {
                    Some(now) => vec![ShellAction::SetMarkedText(delete_target_marked(now))],
                    None => vec![ShellAction::Consume],
                }
            }
        }
    }

    fn preview_delete(&mut self, preview: DeletePreview) -> Vec<ShellAction> {
        match preview {
            DeletePreview::Committed(target) => {
                self.delete_preview = None;
                self.clause_undo.push(target.undo_step());
                vec![ShellAction::Restore {
                    text: String::new(),
                    replacing: target.utf16_range(),
                }]
            }
            DeletePreview::Composing { units, target } => {
                let actions = self.remove_reading_unit(&units[target].clone());
                if actions != [ShellAction::Consume] {
                    self.delete_preview = None;
                }
                actions
            }
            DeletePreview::Converting => {
                let Some(focused) = self.converting().map(|current| current.focused) else {
                    return vec![ShellAction::Consume];
                };
                let actions = self.remove_converting_segments(focused, 1);
                if actions != [ShellAction::Consume] {
                    self.delete_preview = None;
                }
                actions
            }
        }
    }
}

/// フォーカスしている文節を削除の対象として見せる変換中の文字列
fn delete_target_marked(state: &ConvertingState) -> MarkedText {
    MarkedText {
        delete_target: true,
        ..marked_text(state)
    }
}
