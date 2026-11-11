//! 編集セッション（`ITfEditSession`）と、その中で行うこと: `TextAccess`（周辺テキストと選択）、`TsfOp` の実行。
//!
//! TSF では、文書を読み書きできるのは編集セッションの中（`DoEditSession` が受けた `ec` を使う間）だけ。
//! キーの処理は同期のセッション（`TF_ES_SYNC`。キーイベントの通知の中でだけ取れる）で、`Shell::dispatch` と
//! `TsfOp` の実行をまとめて 1 回のセッションで行う。
//!
//! 判断は持たない。`TsfOp` の意味（`tsf_op` の doc）を TSF の呼び出しに写すだけ。

use std::cell::RefCell;
use std::mem::ManuallyDrop;

use ikari_shell_core::{SelectedText, SurroundingText, TextAccess, TimerId, Utf16Len, Utf16Range};
use windows::Win32::Foundation::{E_UNEXPECTED, RECT};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfContext, ITfContextComposition, ITfEditSession,
    ITfEditSession_Impl, ITfInsertAtSelection, ITfRange, ITfRangeACP, TF_AE_NONE, TF_ANCHOR_END,
    TF_ANCHOR_START, TF_CONTEXT_EDIT_CONTEXT_FLAGS, TF_DEFAULT_SELECTION, TF_IAS_QUERYONLY,
    TF_SELECTION, TF_SELECTIONSTYLE,
};
use windows_core::{BOOL, HRESULT, Interface, implement};

use super::dll::DllRef;
use super::guard::guard;
use super::log;
use crate::tsf_op::TsfOp;

/// 周辺テキストの左と右に読む長さ（UTF-16）。macOS の `ClientAccess.leftContextLimit`・`rightContextLimit` と同じ値
const LEFT_CONTEXT_LIMIT: i32 = 2000;
const RIGHT_CONTEXT_LIMIT: i32 = 200;
/// 選択の長さの上限（UTF-16）。アプリが壊れた長さを返したときに、大きな領域を取らないため。動作に効かない上限
const SELECTION_SANITY_LIMIT: i32 = 1 << 20;
/// 取り残された合成が空でないかを見るのに読む長さ（1 文字あれば足りる）
const STRAY_PROBE_LEN: i32 = 1;
/// `ITfRange::GetText` の旗（なし。見えない文字も含めた素の文字列）
const GET_TEXT_PLAIN: u32 = 0;
/// `ITfRange::SetText` の旗（なし）
const SET_TEXT_PLAIN: u32 = 0;

// ---------------------------------------------------------------- 編集セッションの口

type Body = Box<dyn FnOnce(u32)>;

/// 1 回だけ `body` を呼ぶ編集セッション。
#[implement(ITfEditSession)]
struct EditSession {
    body: RefCell<Option<Body>>,
    _dll: DllRef,
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> windows_core::Result<()> {
        // `body` の中の panic は `body` 自身が止める（service）。ここは最後の備え
        guard("panic in DoEditSession", Err(E_UNEXPECTED.into()), || {
            let body = self
                .body
                .try_borrow_mut()
                .ok()
                .and_then(|mut body| body.take());
            if let Some(body) = body {
                body(ec);
            }
            Ok(())
        })
    }
}

/// 編集セッションを頼む。同期（`TF_ES_SYNC`）なら、戻る前に `body` が呼ばれている。
/// `Err` は頼めなかった（同期で取れない、読み取り専用など。`body` は呼ばれない）。
pub fn request(
    context: &ITfContext,
    client_id: u32,
    flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS,
    body: impl FnOnce(u32) + 'static,
) -> windows_core::Result<HRESULT> {
    let session: ITfEditSession = EditSession {
        body: RefCell::new(Some(Box::new(body))),
        _dll: DllRef::new(),
    }
    .into();
    // SAFETY: `session` は呼び出しの間生きている（TSF が非同期に呼ぶときは TSF が AddRef して持つ）
    let hr = unsafe { context.RequestEditSession(client_id, &session, flags) }?;
    // `RequestEditSession` の戻り値は、頼むことの成否。セッションの結果（同期のとき）は `hr`
    hr.ok()?;
    Ok(hr)
}

// ---------------------------------------------------------------- 範囲の小さな道具

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// 既定の選択（キャレット）の範囲。
fn default_selection(ec: u32, context: &ITfContext) -> Option<ITfRange> {
    let mut selection = [TF_SELECTION::default()];
    let mut fetched = 0u32;
    // SAFETY: `selection` は 1 要素の書ける配列、`fetched` は書ける場所。`ec` はこのセッションのもの
    unsafe { context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched) }.ok()?;
    if fetched == 0 {
        return None;
    }
    // SAFETY: TSF が AddRef して書いた参照を、ここで 1 回だけ取り出す（以降 `selection` の range は使わない）
    unsafe { ManuallyDrop::take(&mut selection[0].range) }
}

/// キャレットを `range` の終わりに置く。
fn put_caret_at_end(ec: u32, context: &ITfContext, range: &ITfRange) {
    let result = (|| -> windows_core::Result<()> {
        // SAFETY: `ec` はこのセッションのもの。`range` は生きている
        let caret = unsafe { range.Clone() }?;
        // SAFETY: 同上
        unsafe { caret.Collapse(ec, TF_ANCHOR_END) }?;
        let selection = [TF_SELECTION {
            range: ManuallyDrop::new(Some(caret)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: BOOL::from(false),
            },
        }];
        // SAFETY: `selection` は呼び出しの間生きている。TSF は必要なら AddRef する
        let result = unsafe { context.SetSelection(ec, &selection) };
        // `ManuallyDrop` に入れた参照を自分で落とす
        let [mut selection] = selection;
        // SAFETY: 1 回だけ落とす
        unsafe { ManuallyDrop::drop(&mut selection.range) };
        result
    })();
    if let Err(e) = result {
        log::error("cannot move the caret", &e);
    }
}

/// `range` の文字列（`limit` 文字まで）。
fn range_text(ec: u32, range: &ITfRange, limit: i32) -> Option<String> {
    let len = usize::try_from(limit.max(0)).ok()?;
    let mut buffer = vec![0u16; len];
    let mut fetched = 0u32;
    // SAFETY: `buffer` は書ける長さを渡す。`fetched` は書ける場所
    unsafe { range.GetText(ec, GET_TEXT_PLAIN, &mut buffer, &mut fetched) }.ok()?;
    let fetched = usize::try_from(fetched).ok()?.min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..fetched]))
}

/// 文書の先頭からの位置（UTF-16）と長さ。アプリが ACP（文字の位置）で表せる文書のときだけ取れる。
fn acp_extent(range: &ITfRange) -> Option<(i32, i32)> {
    let acp: ITfRangeACP = range.cast().ok()?;
    let (mut start, mut len) = (0i32, 0i32);
    // SAFETY: 書ける場所を 2 つ渡す
    unsafe { acp.GetExtent(&mut start, &mut len) }.ok()?;
    (start >= 0 && len >= 0).then_some((start, len))
}

/// 文書の先頭からの範囲（UTF-16）を `ITfRange` にする。ACP で表せない文書では `None`。
fn acp_range(ec: u32, context: &ITfContext, range: Utf16Range) -> Option<ITfRange> {
    // SAFETY: `ec` はこのセッションのもの
    let start = unsafe { context.GetStart(ec) }.ok()?;
    let acp: ITfRangeACP = start.cast().ok()?;
    let location = i32::try_from(range.location.get()).ok()?;
    let length = i32::try_from(range.length.get()).ok()?;
    // SAFETY: 値を渡すだけ。範囲の外なら TSF が誤りを返す
    unsafe { acp.SetExtent(location, length) }.ok()?;
    Some(start)
}

fn to_len(value: i32) -> Utf16Len {
    Utf16Len(u32::try_from(value).unwrap_or(0))
}

/// 範囲の端を 1 点に縮めた複製。
fn collapsed(
    ec: u32,
    range: &ITfRange,
    anchor: windows::Win32::UI::TextServices::TfAnchor,
) -> Option<ITfRange> {
    // SAFETY: `ec` はこのセッションのもの。`range` は生きている
    let copy = unsafe { range.Clone() }.ok()?;
    // SAFETY: 同上
    unsafe { copy.Collapse(ec, anchor) }.ok()?;
    Some(copy)
}

// ---------------------------------------------------------------- TextAccess

/// `Shell::dispatch` に渡す、周辺テキストと選択の読み手（編集セッションの間だけ使う）。
pub struct TsfText<'a> {
    pub ec: u32,
    pub context: &'a ITfContext,
    /// 合成中の範囲（文脈から外す）
    pub composition: Option<&'a ITfComposition>,
}

impl TsfText<'_> {
    fn composition_range(&self) -> Option<ITfRange> {
        // SAFETY: 合成は生きている（呼ぶ側が持つ）
        self.composition
            .and_then(|composition| unsafe { composition.GetRange() }.ok())
    }

    fn read_surrounding(&self) -> Option<SurroundingText> {
        let selection = default_selection(self.ec, self.context)?;
        // 変換中の下線つきの文字列は文脈に含めない（macOS の `ClientAccess.surroundingText` と同じ）
        let around = self
            .composition_range()
            .unwrap_or_else(|| selection.clone());

        let left = collapsed(self.ec, &around, TF_ANCHOR_START).and_then(|range| {
            let mut moved = 0i32;
            // SAFETY: `moved` は書ける場所。止める条件はなし（文書の先頭で止まる）
            unsafe { range.ShiftStart(self.ec, -LEFT_CONTEXT_LIMIT, &mut moved, std::ptr::null()) }
                .ok()?;
            range_text(self.ec, &range, LEFT_CONTEXT_LIMIT)
        });
        let right = collapsed(self.ec, &around, TF_ANCHOR_END).and_then(|range| {
            let mut moved = 0i32;
            // SAFETY: 同上（文書の終わりで止まる）
            unsafe { range.ShiftEnd(self.ec, RIGHT_CONTEXT_LIMIT, &mut moved, std::ptr::null()) }
                .ok()?;
            range_text(self.ec, &range, RIGHT_CONTEXT_LIMIT)
        });
        let caret = acp_extent(&selection).map(|(start, len)| Utf16Range {
            location: to_len(start),
            length: to_len(len),
        });
        // 文書の長さ = 終わりの位置。ACP で表せない文書では 0（返さないアプリと同じ扱い）
        // SAFETY: `ec` はこのセッションのもの
        let document_length = unsafe { self.context.GetEnd(self.ec) }
            .ok()
            .and_then(|end| acp_extent(&end))
            .map_or(Utf16Len::ZERO, |(start, _)| to_len(start));
        Some(SurroundingText {
            left: left.unwrap_or_default(),
            right: right.unwrap_or_default(),
            document_length,
            caret,
        })
    }

    fn read_selection(&self) -> Option<SelectedText> {
        let selection = default_selection(self.ec, self.context)?;
        let (start, len) = acp_extent(&selection)?;
        if len == 0 || len > SELECTION_SANITY_LIMIT {
            return None;
        }
        let text = range_text(self.ec, &selection, len)?;
        // 読めた長さが選択と違う（隠れた文字、埋め込みなど）なら、取れないとする（macOS の actualRange の照合と同じ）
        if Utf16Len::of(&text) != to_len(len) || text.is_empty() {
            return None;
        }
        Some(SelectedText {
            range: Utf16Range {
                location: to_len(start),
                length: to_len(len),
            },
            text,
        })
    }
}

impl TextAccess for TsfText<'_> {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        self.read_surrounding()
    }

    fn selection(&mut self) -> Option<SelectedText> {
        self.read_selection()
    }
}

// ---------------------------------------------------------------- TsfOp の実行

/// タイマーの予約と取り消し（`timer` に渡す。テキストサービスが実装する）。
pub trait Timers {
    fn schedule(&self, id: TimerId, after_ms: u32);
    fn cancel(&self, id: TimerId);
}

/// 文書に触れる口（編集セッションの中だけ）。
pub struct Doc<'a> {
    pub ec: u32,
    pub context: &'a ITfContext,
    pub sink: &'a ITfCompositionSink,
}

/// `TsfOp` の列を順に実行する。
pub struct Exec<'a> {
    /// 文書に触れられないとき（文書のない通知、タイマーで文書がないとき）は `None`。文書の操作は飛ばす
    pub doc: Option<Doc<'a>>,
    /// この TIP の合成（なければ `None`）。実行に合わせて更新する
    pub composition: &'a mut Option<ITfComposition>,
    pub timers: &'a dyn Timers,
}

impl Exec<'_> {
    /// 順に実行し、キーを食べたかを返す（`PassThrough` があれば食べない。取り残された合成がなければ食べない）。
    pub fn run(&mut self, ops: &[TsfOp]) -> bool {
        let mut eaten = crate::ops::key_consumed(ops);
        for op in ops {
            if !self.run_one(op) {
                eaten = false;
            }
        }
        eaten
    }

    /// 1 つ実行する。戻り値 false は「キーをアプリに渡す」（`ResolveStrayComposition` で合成がなかったとき）
    fn run_one(&mut self, op: &TsfOp) -> bool {
        match op {
            TsfOp::PassThrough => {}
            TsfOp::StartComposition => {
                if self.composition.is_none() {
                    self.start_composition();
                }
            }
            TsfOp::UpdateComposition(marked) => {
                if self.composition.is_none() {
                    self.start_composition();
                }
                self.update_composition(&marked.text, marked.replacement_range);
                // TODO(W2 の続き): 文節と強調（`segments`、`focused`、`highlighted_length`、`underlined`）の見た目。
                // 表示属性（`ITfDisplayAttributeProvider` と `GUID_PROP_ATTRIBUTE`）が要る
            }
            TsfOp::CommitComposition { text } => self.commit(text),
            TsfOp::EndComposition => self.end_composition(),
            TsfOp::ResolveStrayComposition => return self.resolve_stray(),
            TsfOp::ShowCandidates(_) => {
                // TODO(W3): 候補窓（方式は ADR 0040 §2.4）。ここでは位置が取れるかだけを見る
                let found = self.candidate_rect().is_some();
                log::event(if found {
                    "show candidates (position found)"
                } else {
                    "show candidates (no position)"
                });
            }
            TsfOp::MoveCandidates(_) => log::event("move candidates"),
            TsfOp::HideCandidates => log::event("hide candidates"),
            TsfOp::DeleteText { range } => {
                // 合成があるときは出ない約束（`tsf_op`）。あっても消さない
                if self.composition.is_none() {
                    self.replace_range(*range, "");
                }
            }
            TsfOp::RestoreText { text, replacing } => self.restore(text, *replacing),
            TsfOp::ScheduleTimer { id, after_ms } => self.timers.schedule(*id, *after_ms),
            TsfOp::CancelTimer { id } => self.timers.cancel(*id),
            // TODO(W4): 入力モードの compartment（GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION など）
            TsfOp::SelectMode { .. } => log::event("select mode (not yet)"),
            // TODO(W4-s): 設定画面
            TsfOp::OpenSettings { .. } => log::event("open settings (not yet)"),
            // Windows の設定画面は engine-host とつなぐ（ADR 0040 §2.5）。TIP には中継の口がない
            TsfOp::BridgeReply { .. } | TsfOp::BridgeClose { .. } => {
                log::event("bridge op ignored")
            }
            // TODO(W4): 通知の出し方
            TsfOp::ShowLearningQuarantineNotice => {
                log::event("learning quarantine notice (not yet)")
            }
        }
        true
    }

    fn start_composition(&mut self) {
        let Some(doc) = &self.doc else {
            log::event("start composition without a document");
            return;
        };
        let result = (|| -> windows_core::Result<ITfComposition> {
            let insert: ITfInsertAtSelection = doc.context.cast()?;
            // 選択の位置の空の範囲（文字は入れない）
            // SAFETY: `ec` はこのセッションのもの。空の文字列を渡す
            let range = unsafe { insert.InsertTextAtSelection(doc.ec, TF_IAS_QUERYONLY, &[]) }?;
            let compose: ITfContextComposition = doc.context.cast()?;
            // SAFETY: `range` と `sink` は生きている。TSF が AddRef して持つ
            unsafe { compose.StartComposition(doc.ec, &range, doc.sink) }
        })();
        match result {
            Ok(composition) => *self.composition = Some(composition),
            Err(e) => log::error("StartComposition failed", &e),
        }
    }

    /// 合成の範囲を `text` にし、キャレットを終わりに置く。`replacing` があれば、先に合成をその範囲に広げる
    fn update_composition(&mut self, text: &str, replacing: Option<Utf16Range>) {
        let (Some(doc), Some(composition)) = (&self.doc, self.composition.as_ref()) else {
            return;
        };
        let result = (|| -> windows_core::Result<()> {
            if let Some(target) = replacing.and_then(|range| acp_range(doc.ec, doc.context, range))
            {
                // 再変換の始まり: 選んでいた範囲を合成に取り込んでから置き換える
                // SAFETY: `ec` はこのセッションのもの。`target` は生きている
                unsafe { composition.ShiftStart(doc.ec, &target) }?;
                // SAFETY: 同上
                unsafe { composition.ShiftEnd(doc.ec, &target) }?;
            }
            // SAFETY: 合成は生きている
            let range = unsafe { composition.GetRange() }?;
            // SAFETY: `ec` はこのセッションのもの。文字列は呼び出しの間生きている
            unsafe { range.SetText(doc.ec, SET_TEXT_PLAIN, &utf16(text)) }?;
            put_caret_at_end(doc.ec, doc.context, &range);
            Ok(())
        })();
        if let Err(e) = result {
            log::error("update composition failed", &e);
        }
    }

    fn end_composition_keep_text(&mut self) {
        let Some(composition) = self.composition.take() else {
            return;
        };
        let Some(doc) = &self.doc else {
            log::event("end composition without a document");
            return;
        };
        // SAFETY: `ec` はこのセッションのもの。合成は生きている
        if let Err(e) = unsafe { composition.EndComposition(doc.ec) } {
            log::error("EndComposition failed", &e);
        }
    }

    fn commit(&mut self, text: &str) {
        // 合成がなくても、合成を作ってすぐ確定する（`ops` の約束）
        if self.composition.is_none() {
            self.start_composition();
        }
        self.update_composition(text, None);
        self.end_composition_keep_text();
    }

    fn end_composition(&mut self) {
        if self.composition.is_some() {
            self.update_composition("", None);
        }
        self.end_composition_keep_text();
    }

    /// 取り残された合成があれば、その文字列のまま確定する（キーは食べる）。なければキーはアプリへ
    fn resolve_stray(&mut self) -> bool {
        let text = match (&self.doc, self.composition.as_ref()) {
            // SAFETY: 合成は生きている
            (Some(doc), Some(composition)) => unsafe { composition.GetRange() }
                .ok()
                .and_then(|range| range_text(doc.ec, &range, STRAY_PROBE_LEN)),
            _ => None,
        };
        match text {
            Some(text) if !text.is_empty() => {
                self.end_composition_keep_text();
                true
            }
            _ => {
                // 空の合成が残っていれば、静かに終える
                self.end_composition_keep_text();
                false
            }
        }
    }

    fn restore(&mut self, text: &str, replacing: Utf16Range) {
        if self.composition.is_some() {
            // 合成の範囲（= 置き換える範囲）を元の文字列にして終える。1 回の書き込みにする
            self.update_composition(text, None);
            self.end_composition_keep_text();
        } else {
            self.replace_range(replacing, text);
        }
    }

    /// 確定済みの文字列の範囲を `text` に置き換える（合成がないときだけ使う）
    fn replace_range(&mut self, range: Utf16Range, text: &str) {
        let Some(doc) = &self.doc else {
            log::event("replace text without a document");
            return;
        };
        let Some(target) = acp_range(doc.ec, doc.context, range) else {
            log::event("replace text: the range is not available");
            return;
        };
        // SAFETY: `ec` はこのセッションのもの。文字列は呼び出しの間生きている
        match unsafe { target.SetText(doc.ec, SET_TEXT_PLAIN, &utf16(text)) } {
            Ok(()) => put_caret_at_end(doc.ec, doc.context, &target),
            Err(e) => log::error("replace text failed", &e),
        }
    }

    /// 候補窓を置く位置（合成、なければキャレットの画面上の矩形）。W3 が使う
    fn candidate_rect(&self) -> Option<RECT> {
        let doc = self.doc.as_ref()?;
        let range = match self.composition.as_ref() {
            // SAFETY: 合成は生きている
            Some(composition) => unsafe { composition.GetRange() }.ok()?,
            None => default_selection(doc.ec, doc.context)?,
        };
        // SAFETY: 文書は生きている
        let view = unsafe { doc.context.GetActiveView() }.ok()?;
        let mut rect = RECT::default();
        let mut clipped = BOOL::default();
        // SAFETY: `rect` と `clipped` は書ける場所。`ec` はこのセッションのもの
        unsafe { view.GetTextExt(doc.ec, &range, &mut rect, &mut clipped) }.ok()?;
        Some(rect)
    }
}
