//! 方式 C: `ITfCandidateListUIElement`。単独では窓が出ないので、B と組み合わせる口として作る。
//!
//! このスパイクでできること: 固定の候補を返す UI 要素のオブジェクトを作り、自分のスレッドの `ITfUIElementMgr` に
//! `BeginUIElement` で渡し、`pbshow`（TIP が自前の窓を出してよいか）を記録する。
//! できないこと（未確認）: 他のアプリ（Notepad など）が UI 要素を受け取って自前で描くかどうか。
//! それには TIP として他のアプリのスレッドに入る必要があり（W2-b）、単独の exe からは見えない。

use windows::core::{implement, Result as WinResult, BOOL, BSTR, GUID};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, FALSE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCandidateListUIElement, ITfCandidateListUIElement_Impl, ITfDocumentMgr,
    ITfThreadMgr, ITfUIElement, ITfUIElementMgr, ITfUIElement_Impl, TF_CLUIE_COUNT,
    TF_CLUIE_CURRENTPAGE, TF_CLUIE_PAGEINDEX, TF_CLUIE_SELECTION, TF_CLUIE_STRING,
};

use crate::model::{Panel, PAGE_SIZE};
use std::cell::Cell;

/// UI 要素の GUID（スパイク用の固定値）
const ELEMENT_GUID: GUID = GUID::from_u128(0x7d1b_3c2e_5a40_4f6b_9e11_2c8f_0a53_b9d1);

#[implement(ITfUIElement, ITfCandidateListUIElement)]
struct CandidateElement {
    panel: Panel,
    shown: Cell<bool>,
}

impl ITfUIElement_Impl for CandidateElement_Impl {
    fn GetDescription(&self) -> WinResult<BSTR> {
        Ok(BSTR::from("ikari spike candidate list"))
    }
    fn GetGUID(&self) -> WinResult<GUID> {
        Ok(ELEMENT_GUID)
    }
    fn Show(&self, bshow: BOOL) -> WinResult<()> {
        self.shown.set(bshow.as_bool());
        println!("C: ITfUIElement::Show({})", bshow.as_bool());
        Ok(())
    }
    fn IsShown(&self) -> WinResult<BOOL> {
        Ok(self.shown.get().into())
    }
}

impl ITfCandidateListUIElement_Impl for CandidateElement_Impl {
    fn GetUpdatedFlags(&self) -> WinResult<u32> {
        Ok(TF_CLUIE_COUNT
            | TF_CLUIE_SELECTION
            | TF_CLUIE_STRING
            | TF_CLUIE_PAGEINDEX
            | TF_CLUIE_CURRENTPAGE)
    }
    fn GetDocumentMgr(&self) -> WinResult<ITfDocumentMgr> {
        Err(E_NOTIMPL.into()) // スパイクは TSF の文脈を持たない
    }
    fn GetCount(&self) -> WinResult<u32> {
        Ok(self.panel.candidates.len() as u32)
    }
    fn GetSelection(&self) -> WinResult<u32> {
        Ok(self.panel.selected as u32)
    }
    fn GetString(&self, uindex: u32) -> WinResult<BSTR> {
        self.panel
            .candidates
            .get(uindex as usize)
            .map(|c| BSTR::from(c.surface))
            .ok_or_else(|| E_INVALIDARG.into())
    }
    fn GetPageIndex(&self, pindex: *mut u32, usize_: u32, pupagecnt: *mut u32) -> WinResult<()> {
        let pages = self.panel.page_count() as u32;
        // SAFETY: TSF 側が渡す配列（usize_ 個）と出力先
        unsafe {
            if !pupagecnt.is_null() {
                *pupagecnt = pages;
            }
            if !pindex.is_null() {
                for p in 0..pages.min(usize_) {
                    *pindex.add(p as usize) = p * PAGE_SIZE as u32;
                }
            }
        }
        Ok(())
    }
    fn SetPageIndex(&self, _pindex: *const u32, _upagecnt: u32) -> WinResult<()> {
        Ok(())
    }
    fn GetCurrentPage(&self) -> WinResult<u32> {
        Ok(self.panel.page() as u32)
    }
}

/// UI 要素を作って `BeginUIElement` に渡し、`show` を返す。`Ok(None)` は UI 要素の管理者が取れなかった
pub fn begin_ui_element() -> Result<Option<bool>, String> {
    // SAFETY: COM の初期化と後始末は対にする。オブジェクトはこの関数の間だけ使う
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let result = (|| -> Result<Option<bool>, String> {
            let mgr: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("ThreadMgr: {e}"))?;
            let Ok(ui_mgr) = windows::core::Interface::cast::<ITfUIElementMgr>(&mgr) else {
                return Ok(None);
            };
            let element: ITfUIElement = CandidateElement {
                panel: Panel::fixed(),
                shown: Cell::new(false),
            }
            .into();
            let mut show: BOOL = FALSE;
            let mut id = 0u32;
            ui_mgr
                .BeginUIElement(&element, &mut show, &mut id)
                .map_err(|e| format!("BeginUIElement: {e}"))?;
            println!("C: BeginUIElement ok id={id} show={}", show.as_bool());
            let _ = ui_mgr.EndUIElement(id);
            Ok(Some(show.as_bool()))
        })();
        CoUninitialize();
        result
    }
}

/// 方式 C は B と組み合わせて動かす。`show == false`（アプリが自前で描く）なら窓を出さないのが正しい動き。
/// このスパイクでは常に B も出して、`show` の値だけを記録する
pub fn run(args: &crate::Args) -> Result<(), String> {
    match begin_ui_element() {
        Ok(Some(show)) => println!(
            "RESULT C ui_element show={show} (standalone process: not the target app's TSF thread)"
        ),
        Ok(None) => println!("RESULT C ITfUIElementMgr not available"),
        Err(e) => println!("RESULT C failed: {e}"),
    }
    println!("C: running B alongside (C alone shows no window)");
    super::b_gdi::run(args)
}
