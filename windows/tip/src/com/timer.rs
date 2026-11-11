//! `TsfOp::ScheduleTimer` / `CancelTimer` の実行（スレッドのタイマー。`SetTimer` に窓を渡さない）。
//!
//! タイマーは UI スレッドのメッセージの繰り返しで `WM_TIMER` として届き、`on_timer` が呼ばれる。
//! 表はスレッドごと（TSF のテキストサービスはスレッドごとに作られ、ほかのスレッドから呼ばれない）。
//! どのタイマーも 1 回だけ（届いたら止める）。同じ `TimerId` を予約し直したら、前のものを止める。
//!
//! 表は、届け先のテキストサービスを強い参照で持つ（届くまで寿命を延ばす）。`Deactivate` で `cancel_all` する。

use std::cell::RefCell;
use std::collections::HashMap;

use ikari_shell_core::TimerId;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows_core::ComObject;

use super::guard::guard;
use super::log;
use super::service::TextService;

/// `SetTimer` の「新しい番号を OS に選ばせる」
const NEW_TIMER: usize = 0;

struct Entry {
    service: ComObject<TextService>,
    id: TimerId,
}

thread_local! {
    /// OS のタイマーの番号 → 届け先
    static TIMERS: RefCell<HashMap<usize, Entry>> = RefCell::new(HashMap::new());
}

fn same(a: &ComObject<TextService>, b: &ComObject<TextService>) -> bool {
    std::ptr::eq(a.get(), b.get())
}

fn kill(os_id: usize) {
    // SAFETY: 窓のないスレッドのタイマー（`SetTimer(None, …)` が返した番号）を止める。消えた番号でも害はない
    let _ = unsafe { KillTimer(None, os_id) };
}

/// 外した届け先（錠の外で落とすため、呼ぶ側に返す）
fn take_matching(pred: impl Fn(&Entry) -> bool) -> Vec<(usize, Entry)> {
    TIMERS.with(|timers| {
        let Ok(mut timers) = timers.try_borrow_mut() else {
            return Vec::new();
        };
        let ids: Vec<usize> = timers
            .iter()
            .filter(|(_, entry)| pred(entry))
            .map(|(os_id, _)| *os_id)
            .collect();
        ids.into_iter()
            .filter_map(|os_id| timers.remove(&os_id).map(|entry| (os_id, entry)))
            .collect()
    })
}

pub fn schedule(service: &ComObject<TextService>, id: TimerId, after_ms: u32) {
    cancel(service, id);
    // SAFETY: 窓を渡さないスレッドのタイマー。`on_timer` は `extern "system"` で、DLL が読まれている間は有効
    // （届け先を表が持つので、`DllCanUnloadNow` は S_FALSE を返し続ける）
    let os_id = unsafe { SetTimer(None, NEW_TIMER, after_ms, Some(on_timer)) };
    if os_id == 0 {
        log::event("SetTimer failed");
        return;
    }
    TIMERS.with(|timers| {
        if let Ok(mut timers) = timers.try_borrow_mut() {
            timers.insert(
                os_id,
                Entry {
                    service: service.clone(),
                    id,
                },
            );
        } else {
            kill(os_id);
        }
    });
}

pub fn cancel(service: &ComObject<TextService>, id: TimerId) {
    for (os_id, entry) in take_matching(|entry| entry.id == id && same(&entry.service, service)) {
        kill(os_id);
        drop(entry);
    }
}

pub fn cancel_all(service: &ComObject<TextService>) {
    for (os_id, entry) in take_matching(|entry| same(&entry.service, service)) {
        kill(os_id);
        drop(entry);
    }
}

/// # Safety
/// OS が `WM_TIMER` の配送で呼ぶ。引数は読むだけ。
unsafe extern "system" fn on_timer(_hwnd: HWND, _message: u32, os_id: usize, _time: u32) {
    guard("panic in on_timer", (), || {
        kill(os_id);
        let entry = TIMERS.with(|timers| timers.try_borrow_mut().ok()?.remove(&os_id));
        if let Some(entry) = entry {
            TextService::timer_fired(&entry.service, entry.id);
        }
    });
}
