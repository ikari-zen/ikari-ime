//! 入力モードのアイコン（言語バーのボタン `GUID_LBI_INPUTMODE` の `GetIcon`。#370）。
//!
//! タスクバーの入力インジケーターは、`GUID_LBI_INPUTMODE` のボタンの**アイコン**を入力モードとして出す。アイコンが
//! ないと、言語の略称（「日本」）に戻る（Microsoft の「IME の要件」の「IME must be compatible with the system tray」。
//! 既定は 16x16 で、DPI に合わせて拡大される）。DLL にはまだアイコンの資源がないので、モードの文字（`mode::label`。
//! 「あ」「A」）を実行時に描いて作る。
//!
//! 文字の色は、タスクバーの色（`SystemUsesLightTheme`）に合わせて白か黒にする。返したアイコンは、受け取った側が
//! `DestroyIcon` で捨てる（`ITfLangBarItemButton::GetIcon` の約束）ので、呼ばれるたびに新しく作る。

use std::ffi::c_void;

use windows::Win32::Foundation::RECT;
use windows::Win32::Foundation::{COLORREF, E_FAIL, ERROR_SUCCESS};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, GdiFlush,
    HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};
use windows_core::{BOOL, w};

/// `GetSystemMetrics` が 0 を返したとき（取れなかった）の大きさ。Microsoft の文書の既定
const DEFAULT_ICON_SIZE: i32 = 16;
/// 文字の高さ（アイコンの一辺に対する割合。百分率）。「あ」が欠けずに読める大きさ（実機で見て直す）
const GLYPH_HEIGHT_PERCENT: i32 = 90;
/// 文字の書体（Windows 10 以降に必ずある、UI 用の日本語の書体）
const FONT_FACE: windows_core::PCWSTR = w!("Yu Gothic UI");
/// 32 ビットの色（BGRA）
const BITS_PER_PIXEL: u16 = 32;
/// 単色のマスク（アルファつきのアイコンでは使われないが、`ICONINFO` には要る）
const MASK_BITS_PER_PIXEL: u32 = 1;
/// タスクバーの色の設定の場所
const PERSONALIZE_KEY: windows_core::PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const LIGHT_THEME_VALUE: windows_core::PCWSTR = w!("SystemUsesLightTheme");
/// 文字の色（明るいタスクバーでは黒、暗いタスクバーでは白）。RGB
const DARK_GLYPH: [u8; 3] = [0, 0, 0];
const LIGHT_GLYPH: [u8; 3] = [255, 255, 255];

/// `label` を描いたアイコンを作る。受け取った側が `DestroyIcon` で捨てる
pub fn mode_icon(label: &str) -> windows_core::Result<HICON> {
    // SAFETY: 引数のない問い合わせ
    let size = match unsafe { GetSystemMetrics(SM_CXSMICON) } {
        0 => DEFAULT_ICON_SIZE,
        size => size,
    };
    let glyph = if taskbar_is_light() {
        DARK_GLYPH
    } else {
        LIGHT_GLYPH
    };
    // SAFETY: 画面と互換のメモリの DC を作るだけ
    let dc = Dc(unsafe { CreateCompatibleDC(None) });
    if dc.0.is_invalid() {
        return Err(E_FAIL.into());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            // 負は上から下の並び
            biHeight: -size,
            biPlanes: 1,
            biBitCount: BITS_PER_PIXEL,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: `info` は呼び出しの間生きている。`bits` は書ける場所
    let color =
        Gdi(
            unsafe { CreateDIBSection(Some(dc.0), &info, DIB_RGB_COLORS, &mut bits, None, 0) }?
                .into(),
        );
    if bits.is_null() {
        return Err(E_FAIL.into());
    }
    // SAFETY: 単色のビットマップを作るだけ（中身は 0）
    let mask = Gdi(unsafe { CreateBitmap(size, size, 1, MASK_BITS_PER_PIXEL, None) }.into());
    // SAFETY: 書体を作るだけ。名前は定数
    let font: HFONT = unsafe {
        CreateFontW(
            -(size * GLYPH_HEIGHT_PERCENT / 100),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            FONT_FACE,
        )
    };
    let font = Gdi(font.into());

    draw(&dc, &color, &font, size, label);
    // SAFETY: GDI の描画を DIB に書き出させる（このあと `bits` を読む）
    let _ = unsafe { GdiFlush() };
    let pixels = (size * size) as usize;
    // SAFETY: `bits` は `size`x`size` の 32 ビットの DIB の先頭で、`color` が生きている間有効
    let buffer = unsafe { std::slice::from_raw_parts_mut(bits.cast::<[u8; 4]>(), pixels) };
    // GDI は透明度を書かない。白で描いた明るさを透明度にし、色はアルファを掛けた値にする（事前乗算）
    for pixel in buffer.iter_mut() {
        let coverage = pixel[0].max(pixel[1]).max(pixel[2]);
        let scale = |c: u8| ((u16::from(c) * u16::from(coverage)) / 255) as u8;
        *pixel = [scale(glyph[2]), scale(glyph[1]), scale(glyph[0]), coverage];
    }

    let icon = ICONINFO {
        fIcon: BOOL::from(true),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: HBITMAP(mask.0.0),
        hbmColor: HBITMAP(color.0.0),
    };
    // SAFETY: `icon` のビットマップは生きている。OS が複製するので、このあと捨ててよい
    unsafe { CreateIconIndirect(&icon) }
}

/// 黒地に白で `label` を中央に描く（明るさを透明度として使う）
fn draw(dc: &Dc, color: &Gdi, font: &Gdi, size: i32, label: &str) {
    // SAFETY: `dc` に、生きているビットマップと書体を選ぶ。戻すのは `dc` を捨てる前
    let previous_bitmap = unsafe { SelectObject(dc.0, color.0) };
    let previous_font = unsafe { SelectObject(dc.0, font.0) };
    let mut text: Vec<u16> = label.encode_utf16().collect();
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: size,
        bottom: size,
    };
    // SAFETY: `dc` は生きている。`text` と `rect` は呼び出しの間生きている
    unsafe {
        SetBkMode(dc.0, TRANSPARENT);
        SetTextColor(dc.0, COLORREF(0x00FF_FFFF));
        DrawTextW(
            dc.0,
            &mut text,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc.0, previous_font);
        SelectObject(dc.0, previous_bitmap);
    }
}

/// タスクバーが明るい色か（読めなければ暗い色。Windows 11 の既定）
fn taskbar_is_light() -> bool {
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    // SAFETY: `value` と `len` は書ける場所で、`len` は `value` の大きさ
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE_KEY,
            LIGHT_THEME_VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut len),
        )
    };
    status == ERROR_SUCCESS && value != 0
}

/// 捨てる GDI のオブジェクト
struct Gdi(HGDIOBJ);

impl Drop for Gdi {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: このオブジェクトが作ったもので、DC から外してある
            let _ = unsafe { DeleteObject(self.0) };
        }
    }
}

/// 捨てるメモリの DC
struct Dc(HDC);

impl Drop for Dc {
    fn drop(&mut self) {
        // SAFETY: `CreateCompatibleDC` で作ったもの
        let _ = unsafe { DeleteDC(self.0) };
    }
}
