//! 候補ウィンドウの配置の口（ヘッダの `ikari_panel_*`。L-E で追加）。`shell-core` の `panel` の純関数を呼ぶだけ。
//! 状態を持たないので、どのスレッドからでも呼べる。入出力は protobuf（`PanelAnchorRequest` など）。

use std::panic::{AssertUnwindSafe, catch_unwind};

use ikari_proto::shell::v1 as pb;
use ikari_shell_core::panel::{Size, layout_panel, resolve_anchor};
use prost::Message;

use crate::convert::{rect_from_pb, rect_to_pb, side_to_pb, source_to_pb};
use crate::shell::{
    IKARI_SHELL_ERR_DECODE, IKARI_SHELL_ERR_NULL, IKARI_SHELL_ERR_PANIC, IKARI_SHELL_OK,
};
use crate::{IkariShellBuf, IkariStr, bytes};

fn run(
    request: IkariStr,
    out: *mut IkariShellBuf,
    body: impl FnOnce(&[u8]) -> Result<Vec<u8>, i32>,
) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    // SAFETY: 借用の文字列（ヘッダの約束）
    let raw = unsafe { bytes(request.ptr, request.len) };
    let (status, buf) = match catch_unwind(AssertUnwindSafe(|| body(raw))) {
        Ok(Ok(encoded)) => (IKARI_SHELL_OK, IkariShellBuf::from_vec(encoded)),
        Ok(Err(status)) => (status, IkariShellBuf::EMPTY),
        Err(_) => (IKARI_SHELL_ERR_PANIC, IkariShellBuf::EMPTY),
    };
    // SAFETY: 上で NULL を除いた。書き込める（ヘッダの約束）
    unsafe { out.write(buf) };
    status
}

/// # Safety
/// `request` は借用の文字列（読める `len` バイト、または NULL）。`out` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_panel_resolve_anchor(
    request: IkariStr,
    out: *mut IkariShellBuf,
) -> i32 {
    run(request, out, |raw| {
        let request = pb::PanelAnchorRequest::decode(raw).map_err(|_| IKARI_SHELL_ERR_DECODE)?;
        let mouse = rect_from_pb(request.mouse_screen_visible_frame).unwrap_or_default();
        let anchor = resolve_anchor(
            rect_from_pb(request.client).as_ref(),
            rect_from_pb(request.accessibility).as_ref(),
            rect_from_pb(request.last_known).as_ref(),
            &mouse,
        );
        Ok(pb::PanelAnchor {
            rect: Some(rect_to_pb(&anchor.rect)),
            source: source_to_pb(Some(anchor.source)).into(),
        }
        .encode_to_vec())
    })
}

/// # Safety
/// `request` は借用の文字列（読める `len` バイト、または NULL）。`out` は NULL か書き込めること。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_panel_layout(request: IkariStr, out: *mut IkariShellBuf) -> i32 {
    run(request, out, |raw| {
        let request = pb::PanelLayoutRequest::decode(raw).map_err(|_| IKARI_SHELL_ERR_DECODE)?;
        let anchor = rect_from_pb(request.anchor).unwrap_or_default();
        let visible = rect_from_pb(request.visible_frame).unwrap_or_default();
        let placement = layout_panel(
            &anchor,
            Size {
                width: request.width,
                height: request.height,
            },
            request.reserved_height,
            &visible,
        );
        Ok(pb::PanelPlacement {
            frame: Some(rect_to_pb(&placement.frame)),
            side: side_to_pb(placement.side).into(),
        }
        .encode_to_vec())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(
        f: unsafe extern "C" fn(IkariStr, *mut IkariShellBuf) -> i32,
        request: &[u8],
    ) -> (i32, Vec<u8>) {
        let mut out = IkariShellBuf::EMPTY;
        let status = unsafe { f(IkariStr::of_bytes(request), &mut out) };
        (status, unsafe { out.into_vec() })
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Option<pb::Rect> {
        Some(pb::Rect {
            x,
            y,
            width,
            height,
        })
    }

    #[test]
    fn resolve_anchor_skips_unusable_rects() {
        let request = pb::PanelAnchorRequest {
            client: rect(0.0, 0.0, 0.0, 17.0), // 「取れなかった」の値
            accessibility: rect(100.0, 200.0, 1.0, 1.0),
            last_known: None,
            mouse_screen_visible_frame: rect(0.0, 0.0, 1000.0, 800.0),
        };
        let (status, out) = call(ikari_panel_resolve_anchor, &request.encode_to_vec());
        assert_eq!(status, IKARI_SHELL_OK);
        let anchor = pb::PanelAnchor::decode(out.as_slice()).unwrap();
        assert_eq!(anchor.source, pb::AnchorSource::Accessibility as i32);
        assert_eq!(anchor.rect, rect(100.0, 200.0, 1.0, 1.0));
    }

    #[test]
    fn layout_goes_above_when_below_does_not_fit() {
        let request = pb::PanelLayoutRequest {
            anchor: rect(100.0, 50.0, 0.0, 20.0),
            width: 200.0,
            height: 100.0,
            reserved_height: Some(200.0),
            visible_frame: rect(0.0, 0.0, 1000.0, 800.0),
        };
        let (status, out) = call(ikari_panel_layout, &request.encode_to_vec());
        assert_eq!(status, IKARI_SHELL_OK);
        let placement = pb::PanelPlacement::decode(out.as_slice()).unwrap();
        assert_eq!(placement.side, pb::PanelSide::Above as i32);
        assert_eq!(placement.frame, rect(100.0, 70.0, 200.0, 100.0));
    }

    #[test]
    fn garbage_is_a_decode_error_with_empty_output() {
        let (status, out) = call(ikari_panel_layout, &[0xff, 0xff]);
        assert_eq!(status, IKARI_SHELL_ERR_DECODE);
        assert!(out.is_empty());
    }
}
