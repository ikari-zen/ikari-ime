//! 開発用（実機の確認の道具。製品に入らない）: 動いている `ikari-candidate.exe` に、固定の候補の `Show` を送り、
//! 決めた秒数のあとに `Hide` を送る。TIP なしで、窓の出方（位置、上下、前面、最前面）を確かめる
//! （`docs/windows-real-device-check.md` の候補窓の項目）。
//!
//! 使い方（Windows。候補窓と同じユーザーとセッションで流す）:
//!   candidate-send.exe <x> <y> [above|below] [scale] [seconds] [long|invisible|invisible:<n>|invisible:note]
//! `long` は、上限（行の数、1 行の長さ。`limits`）を超える候補を送る（切り詰めた見た目を見る）。
//! `invisible` は、見えない文字と表示の向きを変える文字を混ぜた候補、注釈、モデルの注釈を送る（`limits` が除くことを見る。
//! #450 FP-5）。1 画面（5 行）に収まらないので、`invisible:<n>`（n は 1 始まりの行番号）はその行だけを 1 行目の候補として、
//! `invisible:note` は注釈とモデルの注釈だけを 1 候補に付けて送る。送った内容（コードポイントを `U+XXXX` で書いたもの）を標準出力に出す。
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

    /// 注釈を見るときの候補の表記（見えない文字を含まない）
    pub const NOTE_SURFACE: &str = "漢字";
    pub const MODE_PREFIX: &str = "invisible";
    pub const NOTE_SELECTOR: &str = "note";

    /// 送る内容: (名前、送る文字列) の並びと、注釈を付けるか
    #[derive(Debug, PartialEq, Eq)]
    pub struct Payload {
        pub candidates: Vec<(&'static str, &'static str)>,
        /// 先頭の候補とモデルに `ANNOTATION` を付ける
        pub annotated: bool,
    }

    /// `invisible`（全部）、`invisible:<n>`（n 行目だけ）、`invisible:note`（注釈だけ）を読む。
    /// `invisible` で始まらない指定は `None`（別のモード）。
    pub fn parse(mode: &str) -> Option<Result<Payload, String>> {
        let rest = mode.strip_prefix(MODE_PREFIX)?;
        if rest.is_empty() {
            return Some(Ok(Payload {
                candidates: ROWS.iter().map(|(n, s, _)| (*n, *s)).collect(),
                annotated: true,
            }));
        }
        let selector = rest.strip_prefix(':')?;
        if selector == NOTE_SELECTOR {
            return Some(Ok(Payload {
                candidates: vec![(NOTE_SELECTOR, NOTE_SURFACE)],
                annotated: true,
            }));
        }
        let row = selector
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| ROWS.get(i));
        Some(match row {
            Some((name, sent, _)) => Ok(Payload {
                candidates: vec![(*name, *sent)],
                annotated: false,
            }),
            None => Err(format!(
                "bad row: {selector} (1..={} or {NOTE_SELECTOR})",
                ROWS.len()
            )),
        })
    }

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
        if let Some(payload) = mode.and_then(invisible::parse) {
            let payload = payload?;
            model.candidates = payload
                .candidates
                .iter()
                .enumerate()
                .map(|(i, (name, sent))| {
                    println!("sent {name}: {}", invisible::escape(sent));
                    CandidateItem {
                        surface: (*sent).to_owned(),
                        annotation: if payload.annotated && i == 0 {
                            invisible::ANNOTATION.to_owned()
                        } else {
                            String::new()
                        },
                        ..Default::default()
                    }
                })
                .collect();
            if payload.annotated {
                println!(
                    "sent annotation: {}",
                    invisible::escape(invisible::ANNOTATION)
                );
                model.annotation = invisible::ANNOTATION.to_owned();
            }
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
    use super::invisible::{ANNOTATION, ANNOTATION_EXPECTED, NOTE_SURFACE, ROWS, escape, parse};
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

    #[test]
    fn all_sends_every_row_with_annotation() {
        let p = parse("invisible").unwrap().unwrap();
        assert_eq!(p.candidates.len(), ROWS.len());
        assert!(p.annotated);
    }

    #[test]
    fn each_row_number_sends_exactly_that_row() {
        for (i, (name, sent, expected)) in ROWS.iter().enumerate() {
            let p = parse(&format!("invisible:{}", i + 1)).unwrap().unwrap();
            assert_eq!(p.candidates, vec![(*name, *sent)]);
            assert!(!p.annotated);
            assert_eq!(sanitize_text(p.candidates[0].1), *expected, "{name}");
        }
    }

    #[test]
    fn out_of_range_row_is_an_error() {
        for bad in [
            "invisible:0",
            "invisible:8",
            "invisible:-1",
            "invisible:x",
            "invisible:",
        ] {
            assert!(parse(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn note_sends_one_annotated_candidate() {
        let p = parse("invisible:note").unwrap().unwrap();
        assert_eq!(p.candidates, vec![("note", NOTE_SURFACE)]);
        assert!(p.annotated);
        assert_eq!(sanitize_text(ANNOTATION), ANNOTATION_EXPECTED);
    }

    #[test]
    fn other_modes_are_not_invisible() {
        assert!(parse("long").is_none());
        assert!(parse("invisiblex").is_none());
    }
}
