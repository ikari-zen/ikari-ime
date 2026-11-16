//! 開発用（実機の確認の道具。製品に入らない）: 動いている `ikari-candidate.exe` に、固定の候補の `Show` を送り、
//! 決めた秒数のあとに `Hide` を送る。TIP なしで、窓の出方（位置、上下、前面、最前面）を確かめる
//! （`docs/windows-real-device-check.md` の候補窓の項目）。
//!
//! 使い方（Windows。候補窓と同じユーザーとセッションで流す）:
//!   candidate-send.exe <x> <y> [above|below] [scale] [seconds]
//! x と y はアンカー（物理のピクセル、左上が原点）。作業領域は主画面の 1920x1040 とみなす（確認用の固定の値）。

#[cfg(windows)]
fn main() {
    if let Err(message) = windows_main::run() {
        eprintln!("candidate-send: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("candidate-send: Windows only");
    std::process::exit(1);
}

#[cfg(windows)]
mod windows_main {
    use std::time::Duration;

    use ikari_candidate::command::{
        CANDIDATE_SCHEMA_VERSION, CandidateCommand, Hello, Hide, Show, candidate_command,
    };
    use ikari_pipe::os::windows::{connect, current_session_id, current_user_sid};
    use ikari_proto::shell::v1::{CandidateItem, CandidatePanelModel, PanelSide, Rect};
    use prost::Message;

    const SAMPLE: &[&str] = &["漢字", "感じ", "幹事", "監事", "かんじ"];
    const WORK_AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0,
    };
    const ANCHOR_HEIGHT: f64 = 20.0;
    const DEFAULT_SECONDS: u64 = 5;

    pub fn run() -> Result<(), String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let num = |i: usize, default: f64| -> Result<f64, String> {
            args.get(i).map_or(Ok(default), |s| {
                s.parse().map_err(|_| format!("bad number: {s}"))
            })
        };
        let x = num(0, 200.0)?;
        let y = num(1, 200.0)?;
        let side = match args.get(2).map(String::as_str) {
            Some("above") => PanelSide::Above,
            _ => PanelSide::Below,
        };
        let scale = num(3, 1.0)?;
        let seconds = args.get(4).map_or(Ok(DEFAULT_SECONDS), |s| {
            s.parse().map_err(|_| format!("bad seconds: {s}"))
        })?;

        let sid = current_user_sid().map_err(|e| e.to_string())?;
        let session = current_session_id().map_err(|e| e.to_string())?;
        let name = ikari_pipe::candidate_pipe_name(&sid, session).map_err(|e| format!("{e:?}"))?;
        let client = connect(&name).map_err(|e| format!("connect: {e}"))?;
        let post = |seq: u64, op: candidate_command::Op| -> Result<(), String> {
            let bytes = CandidateCommand { seq, op: Some(op) }.encode_to_vec();
            client.post(&bytes).map_err(|e| format!("post: {e:?}"))
        };
        post(
            1,
            candidate_command::Op::Hello(Hello {
                schema_version: CANDIDATE_SCHEMA_VERSION,
                pid: std::process::id(),
            }),
        )?;
        let model = CandidatePanelModel {
            candidates: SAMPLE
                .iter()
                .map(|s| CandidateItem {
                    surface: (*s).to_owned(),
                    ..Default::default()
                })
                .collect(),
            selected: 0,
            reserved_rows: 9,
            ..Default::default()
        };
        post(
            2,
            candidate_command::Op::Show(Show {
                generation: 1,
                model: Some(model),
                anchor: Some(Rect {
                    x,
                    y,
                    width: 1.0,
                    height: ANCHOR_HEIGHT,
                }),
                work_area: Some(WORK_AREA),
                side: side.into(),
                scale,
            }),
        )?;
        println!("shown at ({x},{y}) side={side:?} scale={scale}; hiding in {seconds}s");
        std::thread::sleep(Duration::from_secs(seconds));
        post(3, candidate_command::Op::Hide(Hide { generation: 1 }))?;
        println!("hidden");
        Ok(())
    }
}
