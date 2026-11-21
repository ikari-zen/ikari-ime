//! 開発用（実機の確認の道具。製品に入らない）: 動いている `ikari-candidate.exe` に、固定の候補の `Show` を送り、
//! 決めた秒数のあとに `Hide` を送る。TIP なしで、窓の出方（位置、上下、前面、最前面）を確かめる
//! （`docs/windows-real-device-check.md` の候補窓の項目）。
//!
//! 使い方（Windows。候補窓と同じユーザーとセッションで流す）:
//!   candidate-send.exe <x> <y> [above|below] [scale] [seconds] [long|invisible]
//! `long` は、上限（行の数、1 行の長さ。`limits`）を超える候補を送る（切り詰めた見た目を見る）。
//! `invisible` は、見えない文字と表示の向きを変える文字を混ぜた候補、注釈、モデルの注釈を送る（`limits` が除くことを見る。
//! #450 FP-5）。送った内容（コードポイントを `U+XXXX` で書いたもの）を標準出力に出す。
//! 候補窓は別のものが動いているとパイプ名が衝突する（名前は SID とセッションで決まり、変えられない）ので、
//! engine-host を止めて、手で起動した窓を相手にする（項目 59、61）。
//! x と y はアンカー（物理のピクセル、左上が原点）。作業領域は主画面の 1920x1040 とみなす（確認用の固定の値）。

/// `invisible` のモードで送る文字列（`limits::sanitize_text` が除く文字を混ぜる）。純粋なので Mac でテストが回る。
#[cfg_attr(not(windows), allow(dead_code))]
mod invisible {
    /// 行ごとの (見出し、除くはずの文字を混ぜた文字列、除かれたあとの見込み)。
    /// 見出しと前後は、除かれたあとも見える文字にする（窓の上で何が起きたか読める）
    pub const ROWS: &[(&str, &str, &str)] = &[
        ("zero-width", "ab\u{200B}cd\u{2060}ef\u{FEFF}gh", "abcdefgh"),
        // 除かれなければ、RLO のあとが逆順に見える（abc の次に fed）
        ("bidi-override", "abc\u{202E}def\u{202C}ghi", "abcdefghi"),
        ("bidi-isolate", "ab\u{2066}cd\u{2069}ef", "abcdef"),
        ("line-separator", "ab\u{2028}cd\u{2029}ef", "abcdef"),
        // 改行・タブ・NUL・C1 などの制御文字
        ("control", "ab\ncd\te\u{0}f\u{85}g", "abcdefg"),
        // 「A」「B」のタグ文字（除かれなければ、環境によって豆腐や空白で見える）
        ("tag", "ab\u{E0041}\u{E0042}cd", "abcd"),
        // ZWJ は残す（絵文字の綴りに要る）: 家族の絵文字が 1 つに見える
        (
            "zwj-kept",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        ),
    ];
    /// 注釈とモデルの注釈に混ぜる文字列（先頭の候補にだけ付ける）
    pub const ANNOTATION: &str = "note\u{200B}\u{202E}-x";
    #[cfg(test)]
    pub const ANNOTATION_EXPECTED: &str = "note-x";

    /// コードポイントを `U+XXXX` で書き出す（ASCII の文字はそのまま）。標準出力に何を送ったかを残す
    pub fn escape(text: &str) -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_graphic() {
                    c.to_string()
                } else {
                    format!("<U+{:04X}>", c as u32)
                }
            })
            .collect()
    }
}

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

    use crate::invisible;

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
    /// `long` のときの行の数と 1 行の文字の数（上限より多い）
    const LONG_ROWS: usize = 40;
    const LONG_CHARS: usize = 1000;

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
        let mode = args.get(5).map(String::as_str);
        let mut model = CandidatePanelModel {
            selected: 0,
            reserved_rows: 9,
            ..Default::default()
        };
        if mode == Some("invisible") {
            model.candidates = invisible::ROWS
                .iter()
                .enumerate()
                .map(|(i, (name, sent, _))| {
                    println!("sent {name}: {}", invisible::escape(sent));
                    CandidateItem {
                        surface: (*sent).to_owned(),
                        annotation: if i == 0 {
                            invisible::ANNOTATION.to_owned()
                        } else {
                            String::new()
                        },
                        ..Default::default()
                    }
                })
                .collect();
            model.annotation = invisible::ANNOTATION.to_owned();
        } else {
            let surfaces: Vec<String> = if mode == Some("long") {
                (0..LONG_ROWS)
                    .map(|i| format!("{i}{}", "長".repeat(LONG_CHARS)))
                    .collect()
            } else {
                SAMPLE.iter().map(|s| (*s).to_owned()).collect()
            };
            model.candidates = surfaces
                .into_iter()
                .map(|surface| CandidateItem {
                    surface,
                    ..Default::default()
                })
                .collect();
        }
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

#[cfg(test)]
mod tests {
    use super::invisible::{ANNOTATION, ANNOTATION_EXPECTED, ROWS, escape};
    use ikari_candidate::limits::sanitize_text;

    #[test]
    fn every_row_is_sanitized_to_its_expectation() {
        for (name, sent, expected) in ROWS {
            assert_eq!(sanitize_text(sent), *expected, "{name}");
        }
        assert_eq!(sanitize_text(ANNOTATION), ANNOTATION_EXPECTED);
    }

    #[test]
    fn every_row_changes_except_the_kept_zwj() {
        for (name, sent, expected) in ROWS {
            assert_eq!(sent == expected, *name == "zwj-kept", "{name}");
        }
    }

    #[test]
    fn escape_shows_invisible_characters() {
        assert_eq!(escape("a\u{200B}b\u{E0041}"), "a<U+200B>b<U+E0041>");
    }
}
