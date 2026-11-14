//! 入力モードの表示（W4。`TsfOp::ShowInputMode` の実行。`mode` の表を TSF の API に写すだけ）。
//!
//! - compartment（`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` と `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`。
//!   スレッドの管理者のもの）への書き込み。OS が「IME のオン・オフ」「入力モード」を読むときの正規の場所。
//!   値は `mode::values_of`。
//! - 言語バーのボタン（`ITfLangBarItemButton`。`GUID_LBI_INPUTMODE`）。タスクバーの入力インジケーターが、
//!   この種類のボタンの文字を入力モードとして出す、というのが Microsoft の文書の説明（実機で確かめる。
//!   `docs/windows-real-device-check.md` の項目 34）。文字は `mode::label`。クリックは、OPENCLOSE の compartment を
//!   反転するだけにする（判断は持たない。反転は下の通知で `shell-core` に届く）。
//! - compartment の変更の通知（`ITfCompartmentEventSink`）の登録。OS 側の変更（言語バーのクリック、IME のオン・オフ）を
//!   `mode::from_open`・`from_conversion` で読み、`ShellEvent::InputModeChanged` にするのは `service`。
//!
//! 自分の書き込みでも通知が来る。`service` が書き込みの間だけ旗を立てて、自分の通知を読み捨てる。
//!
//! **まだ入れていないもの**: ボタンのアイコン（DLL に資源がない。`GetIcon` は空のアイコンを返す）、ボタンのメニュー。

use std::cell::{Cell, RefCell};

use ikari_shell_core::english::InputMode;
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED, POINT, RECT};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::TextServices::{
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_LBI_INPUTMODE, ITfCompartmentEventSink, ITfCompartmentMgr, ITfLangBarItem,
    ITfLangBarItem_Impl, ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfLangBarItemMgr,
    ITfLangBarItemSink, ITfMenu, ITfSource, ITfSource_Impl, ITfThreadMgr, TF_LANGBARITEMINFO,
    TF_LBI_ICON, TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_SHOWNINTRAY, TF_LBI_TEXT, TF_LBI_TOOLTIP,
    TfLBIClick,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows_core::{BOOL, BSTR, ComObject, GUID, HRESULT, IUnknown, Interface, Ref, implement};

use super::dll::DllRef;
use super::guard::guard_result;
use super::guids::CLSID_TEXT_SERVICE;
use super::log;
use crate::mode;

/// ボタンの順序（言語バーの中の並び。小さいほど左）
const BUTTON_SORT: u32 = 0;
/// `TF_LANGBARITEMINFO::szDescription` の長さ（UTF-16。NUL を含む）
const DESCRIPTION_LEN: usize = 32;
/// ボタンの名前（言語バーのメニューで見える）
const BUTTON_DESCRIPTION: &str = "ikari 入力モード";
/// `winerror.h` の `CONNECT_E_*`（`ITfSource` の誤り。`windows` crate には入っていない）
const CONNECT_E_NOCONNECTION: HRESULT = HRESULT(0x8004_0200_u32 as i32);
const CONNECT_E_ADVISELIMIT: HRESULT = HRESULT(0x8004_0201_u32 as i32);
const CONNECT_E_CANNOTCONNECT: HRESULT = HRESULT(0x8004_0202_u32 as i32);

/// `ITfLangBarItemSink` を接続するときの cookie。接続は 1 つだけ受けるので、固定でよい
const ITEM_SINK_COOKIE: u32 = 1;

/// 言語バーのボタン。今のモードの文字と説明を持つ。
#[implement(ITfLangBarItemButton, ITfSource)]
struct ModeButton {
    mode: Cell<InputMode>,
    sink: RefCell<Option<ITfLangBarItemSink>>,
    compartments: ITfCompartmentMgr,
    client_id: u32,
    _dll: DllRef,
}

impl ModeButton {
    /// 文字・説明・アイコンが変わったことを、言語バーに知らせる
    fn notify(&self) {
        let sink = match self.sink.try_borrow() {
            Ok(sink) => sink.clone(),
            Err(_) => return,
        };
        if let Some(sink) = sink {
            // SAFETY: `sink` は TSF が渡したもので、生きている（`AdviseSink` で AddRef 済み）
            if let Err(e) = unsafe { sink.OnUpdate(TF_LBI_TEXT | TF_LBI_TOOLTIP | TF_LBI_ICON) } {
                log::error("lang bar OnUpdate failed", &e);
            }
        }
    }
}

impl ITfLangBarItem_Impl for ModeButton_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> windows_core::Result<()> {
        guard_result("panic in lang bar GetInfo", || {
            if info.is_null() {
                return Err(E_POINTER.into());
            }
            let mut description = [0u16; DESCRIPTION_LEN];
            // 最後の 1 つは NUL のために残す
            for (slot, unit) in description
                .iter_mut()
                .zip(BUTTON_DESCRIPTION.encode_utf16().take(DESCRIPTION_LEN - 1))
            {
                *slot = unit;
            }
            // SAFETY: `info` は呼び出し側が渡した書ける場所（null は上で除いた）
            unsafe {
                *info = TF_LANGBARITEMINFO {
                    clsidService: CLSID_TEXT_SERVICE,
                    guidItem: GUID_LBI_INPUTMODE,
                    dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                    ulSort: BUTTON_SORT,
                    szDescription: description,
                };
            }
            Ok(())
        })
    }

    fn GetStatus(&self) -> windows_core::Result<u32> {
        // 有効で、見えている
        guard_result("panic in lang bar GetStatus", || Ok(0))
    }

    fn Show(&self, _show: BOOL) -> windows_core::Result<()> {
        // 表示の出し入れは言語バー（OS）が決める。持つ状態はない
        guard_result("panic in lang bar Show", || Ok(()))
    }

    fn GetTooltipString(&self) -> windows_core::Result<BSTR> {
        guard_result("panic in lang bar GetTooltipString", || {
            Ok(BSTR::from(mode::tooltip(self.mode.get())))
        })
    }
}

impl ITfLangBarItemButton_Impl for ModeButton_Impl {
    fn OnClick(
        &self,
        _click: TfLBIClick,
        _point: &POINT,
        _area: *const RECT,
    ) -> windows_core::Result<()> {
        guard_result("panic in lang bar OnClick", || {
            // IME のオン・オフを反転するだけ。どのモードになるかは、compartment の通知から `shell-core` が決める
            let open = read_i32(&self.compartments, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)
                .map(|value| value != 0)
                .unwrap_or_else(|| mode::values_of(self.mode.get()).open);
            write_i32(
                &self.compartments,
                self.client_id,
                &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
                i32::from(!open),
            )
        })
    }

    fn InitMenu(&self, _menu: Ref<ITfMenu>) -> windows_core::Result<()> {
        // メニューの項目はまだない
        guard_result("panic in lang bar InitMenu", || Ok(()))
    }

    fn OnMenuSelect(&self, _id: u32) -> windows_core::Result<()> {
        guard_result("panic in lang bar OnMenuSelect", || Ok(()))
    }

    fn GetIcon(&self) -> windows_core::Result<HICON> {
        // アイコンの資源はまだない。文字（`GetText`）だけで出る。実機で、これで足りるかを確かめる
        guard_result("panic in lang bar GetIcon", || Ok(HICON::default()))
    }

    fn GetText(&self) -> windows_core::Result<BSTR> {
        guard_result("panic in lang bar GetText", || {
            Ok(BSTR::from(mode::label(self.mode.get())))
        })
    }
}

impl ITfSource_Impl for ModeButton_Impl {
    fn AdviseSink(&self, iid: *const GUID, sink: Ref<IUnknown>) -> windows_core::Result<u32> {
        guard_result("panic in lang bar AdviseSink", || {
            if iid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: null でない（上で確かめた）。呼び出し側が渡した GUID への参照
            if unsafe { *iid } != ITfLangBarItemSink::IID {
                return Err(CONNECT_E_CANNOTCONNECT.into());
            }
            let sink: ITfLangBarItemSink = sink.ok()?.cast()?;
            let mut slot = self
                .sink
                .try_borrow_mut()
                .map_err(|_| windows_core::Error::from(E_UNEXPECTED))?;
            if slot.is_some() {
                // 接続できるのは 1 つだけ
                return Err(CONNECT_E_ADVISELIMIT.into());
            }
            *slot = Some(sink);
            Ok(ITEM_SINK_COOKIE)
        })
    }

    fn UnadviseSink(&self, cookie: u32) -> windows_core::Result<()> {
        guard_result("panic in lang bar UnadviseSink", || {
            if cookie != ITEM_SINK_COOKIE {
                return Err(CONNECT_E_NOCONNECTION.into());
            }
            let mut slot = self
                .sink
                .try_borrow_mut()
                .map_err(|_| windows_core::Error::from(E_UNEXPECTED))?;
            if slot.take().is_none() {
                return Err(CONNECT_E_NOCONNECTION.into());
            }
            Ok(())
        })
    }
}

fn read_i32(compartments: &ITfCompartmentMgr, guid: &GUID) -> Option<i32> {
    // SAFETY: `guid` は呼び出しの間生きている
    let compartment = unsafe { compartments.GetCompartment(guid) }.ok()?;
    // SAFETY: COM の呼び出し。値がなければ失敗か空の VARIANT が返る（どちらも `None`）
    let variant = unsafe { compartment.GetValue() }.ok()?;
    i32::try_from(&variant).ok()
}

fn write_i32(
    compartments: &ITfCompartmentMgr,
    client_id: u32,
    guid: &GUID,
    value: i32,
) -> windows_core::Result<()> {
    // SAFETY: `guid` は呼び出しの間生きている
    let compartment = unsafe { compartments.GetCompartment(guid) }?;
    let variant = VARIANT::from(value);
    // SAFETY: `variant` は呼び出しの間生きている。`client_id` は `ActivateEx` で受けた番号
    unsafe { compartment.SetValue(client_id, &variant) }
}

/// 表示の道具一式（スレッドごとに 1 つ。`ActivateEx` で作り、`Deactivate` で外す）。
pub struct Indicator {
    compartments: ITfCompartmentMgr,
    client_id: u32,
    items: ITfLangBarItemMgr,
    item: ComObject<ModeButton>,
    /// compartment の通知の接続（`ITfSource` と cookie）
    advised: Vec<(ITfSource, u32)>,
}

/// 通知を受ける compartment（変更を読むのも書くのもこの 2 つ）
const WATCHED: [GUID; 2] = [
    GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
];

impl Indicator {
    /// 言語バーにボタンを足し、compartment の通知を登録する。`sink` は、通知を受けるテキストサービス。
    ///
    /// 一部だけ成功した場合は、足したものを外してから失敗を返す（残さない）
    pub fn install(
        thread_mgr: &ITfThreadMgr,
        client_id: u32,
        sink: &ITfCompartmentEventSink,
    ) -> windows_core::Result<Self> {
        let compartments: ITfCompartmentMgr = thread_mgr.cast()?;
        let items: ITfLangBarItemMgr = thread_mgr.cast()?;
        let item = ComObject::new(ModeButton {
            mode: Cell::new(InputMode::Japanese),
            sink: RefCell::new(None),
            compartments: compartments.clone(),
            client_id,
            _dll: DllRef::new(),
        });
        let item_iface: ITfLangBarItem = item.to_interface::<ITfLangBarItemButton>().cast()?;
        // SAFETY: `item_iface` は生きている。TSF が AddRef して持つ（`remove` で外す）
        unsafe { items.AddItem(&item_iface) }?;
        let mut indicator = Self {
            compartments,
            client_id,
            items,
            item,
            advised: Vec::new(),
        };
        if let Err(e) = indicator.advise(sink) {
            indicator.remove();
            return Err(e);
        }
        Ok(indicator)
    }

    fn advise(&mut self, sink: &ITfCompartmentEventSink) -> windows_core::Result<()> {
        for guid in &WATCHED {
            // SAFETY: `guid` は定数への参照
            let compartment = unsafe { self.compartments.GetCompartment(guid) }?;
            let source: ITfSource = compartment.cast()?;
            // SAFETY: `sink` は生きている。TSF が AddRef して持つ（`remove` で外す）
            let cookie = unsafe { source.AdviseSink(&ITfCompartmentEventSink::IID, sink) }?;
            self.advised.push((source, cookie));
        }
        Ok(())
    }

    /// 外す（通知の登録とボタン）。失敗は記録するだけ（終了の途中で呼ぶ）
    pub fn remove(&mut self) {
        for (source, cookie) in self.advised.drain(..) {
            // SAFETY: `AdviseSink` が返した番号
            if let Err(e) = unsafe { source.UnadviseSink(cookie) } {
                log::error("compartment UnadviseSink failed", &e);
            }
        }
        let Ok(item_iface) = self
            .item
            .to_interface::<ITfLangBarItemButton>()
            .cast::<ITfLangBarItem>()
        else {
            return;
        };
        // SAFETY: `AddItem` で足したもの
        if let Err(e) = unsafe { self.items.RemoveItem(&item_iface) } {
            log::error("lang bar RemoveItem failed", &e);
        }
    }

    /// モードを表示する（compartment を書き、ボタンの文字を替える）。`service` が自分の通知を読み捨てる旗を立てて呼ぶ。
    /// 書き込みの失敗は記録して続ける（片方が失敗しても、もう片方とボタンは更新する）
    pub fn show(&self, mode: InputMode) {
        let values = mode::values_of(mode);
        let writes = [
            (
                &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
                i32::from(values.open),
                "compartment SetValue failed (OPENCLOSE)",
            ),
            (
                &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
                // 変換モードの bit は 31 bit に収まる（`mode::conversion`）
                i32::try_from(values.conversion).unwrap_or_default(),
                "compartment SetValue failed (CONVERSION)",
            ),
        ];
        for (guid, value, message) in writes {
            if let Err(e) = write_i32(&self.compartments, self.client_id, guid, value) {
                log::error(message, &e);
            }
        }
        self.item.mode.set(mode);
        self.item.notify();
    }

    /// 診断用。compartment の種類（固定の文）と、今の値（読めなければ `None`）
    pub fn describe(&self, guid: &GUID) -> (&'static str, Option<i32>) {
        if *guid == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
            ("openclose", read_i32(&self.compartments, guid))
        } else if *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION {
            ("conversion", read_i32(&self.compartments, guid))
        } else {
            ("other", None)
        }
    }

    /// compartment の今の値から読んだモード。値がなければ `None`
    pub fn read(&self, guid: &GUID) -> Option<InputMode> {
        if *guid == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
            read_i32(&self.compartments, guid).map(|value| mode::from_open(value != 0))
        } else if *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION {
            read_i32(&self.compartments, guid)
                .map(|value| mode::from_conversion(u32::try_from(value).unwrap_or_default()))
        } else {
            None
        }
    }
}
