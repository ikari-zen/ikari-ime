//! 候補窓の命令の状態（ADR 0062 §7.2、§7.3。#405）。純粋（Mac でテストする）。
//!
//! 受けた命令（接続ごとのバイト列）から、画面に渡す `RenderState` を作る。規則:
//! - `seq` は接続ごとに数える。受けた最大以下のものは捨てる（古い・重なった命令）
//! - 窓の持ち主は、最後に `Show` を送った接続。`Update` と `Hide` は、持ち主の接続から、今の世代のものだけを受ける
//!   （`Hide{0}` は世代を問わないが、持ち主の接続からのときだけ効く）
//! - 持ち主の接続が切れたら隠す
//! - 知らない欄と知らない命令は黙って無視し、窓の状態を変えない（版の違い。ADR 0063）
//! - 壊れた命令（proto として読めない）を送った接続は、その後の命令をすべて捨てる（切ったのと同じに扱う。
//!   持ち主なら隠す）。片道なので返事はしない
//! - 受けたものは `limits` で描ける形にしてから使う（ADR 0068 §5）
//! - `side` が `Above` のときは、候補を上下逆に並べる（1 位をアンカーに近い側に。ADR 0014 §2.2）
//!
//! 状態が変わったときだけ `RenderState` を返す（`revision` は 1 から増える）。ログに出すのは固定の語と番号だけ（`Note`）。

use std::collections::HashMap;

use prost::Message;

use crate::command::{
    ANY_GENERATION, CANDIDATE_SCHEMA_VERSION, CandidateCommand, Hide, RenderState, Show, Update,
    candidate_command,
};
use crate::limits::{self, Rejection};
use ikari_proto::shell::v1::PanelSide;

/// 接続の番号（受け手が付ける。0 は使わない）。
pub type ConnId = u64;

/// 起きたことの記録（固定の語と番号だけ。候補の文字は入れない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    /// 送り手の版が自分と違う（動き続ける）
    SchemaMismatch { conn: ConnId, theirs: u32 },
    /// 古い・重なった `seq`
    StaleSeq { conn: ConnId, seq: u64 },
    /// 持ち主でない接続の `Update` / `Hide`
    NotOwner { conn: ConnId },
    /// 今の世代でない `Update` / `Hide`
    StaleGeneration { conn: ConnId },
    /// 上限の外で捨てた
    Rejected { conn: ConnId, reason: Rejection },
    /// proto として読めない。この接続のこれからの命令を捨てる
    Malformed { conn: ConnId },
    /// 知らない命令（空の oneof）
    UnknownOp { conn: ConnId },
}

#[derive(Debug, Default)]
struct Conn {
    last_seq: u64,
    /// 壊れた命令を送った（これからの命令を捨てる）
    poisoned: bool,
}

/// 見えている窓の中身。
#[derive(Debug, Clone)]
struct Shown {
    show: Show,
}

/// 命令の状態。
#[derive(Debug, Default)]
pub struct CandidateState {
    conns: HashMap<ConnId, Conn>,
    /// 窓の持ち主の接続と、その世代
    owner: Option<(ConnId, u64)>,
    shown: Option<Shown>,
    revision: u64,
}

/// 1 つの出来事の結果。
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// 描く状態が変わったとき
    pub render: Option<RenderState>,
    pub note: Option<Note>,
}

impl Outcome {
    fn note(note: Note) -> Self {
        Self {
            render: None,
            note: Some(note),
        }
    }
}

impl CandidateState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 接続が来た。
    pub fn connected(&mut self, conn: ConnId) {
        self.conns.insert(conn, Conn::default());
    }

    /// 接続が切れた。持ち主なら隠す。
    pub fn disconnected(&mut self, conn: ConnId) -> Outcome {
        self.conns.remove(&conn);
        self.drop_owner(conn)
    }

    /// 1 つの枠の本体（`CandidateCommand`）を受けた。
    pub fn received(&mut self, conn: ConnId, bytes: &[u8]) -> Outcome {
        let state = self.conns.entry(conn).or_default();
        if state.poisoned {
            return Outcome::default();
        }
        let Ok(command) = CandidateCommand::decode(bytes) else {
            state.poisoned = true;
            let mut out = self.drop_owner(conn);
            out.note = Some(Note::Malformed { conn });
            return out;
        };
        if command.seq <= state.last_seq {
            return Outcome::note(Note::StaleSeq {
                conn,
                seq: command.seq,
            });
        }
        state.last_seq = command.seq;
        match command.op {
            Some(candidate_command::Op::Hello(hello)) => {
                if hello.schema_version != CANDIDATE_SCHEMA_VERSION {
                    Outcome::note(Note::SchemaMismatch {
                        conn,
                        theirs: hello.schema_version,
                    })
                } else {
                    Outcome::default()
                }
            }
            Some(candidate_command::Op::Show(show)) => self.show(conn, show),
            Some(candidate_command::Op::Update(update)) => self.update(conn, update),
            Some(candidate_command::Op::Hide(hide)) => self.hide(conn, hide),
            None => Outcome::note(Note::UnknownOp { conn }),
        }
    }

    fn show(&mut self, conn: ConnId, show: Show) -> Outcome {
        let show = match limits::sanitize_show(show) {
            Ok(show) => show,
            Err(reason) => return Outcome::note(Note::Rejected { conn, reason }),
        };
        self.owner = Some((conn, show.generation));
        self.shown = Some(Shown { show });
        self.render()
    }

    /// `conn` が今の世代の持ち主か。違えば理由。
    fn check_owner(&self, conn: ConnId, generation: u64, any: bool) -> Result<(), Note> {
        match self.owner {
            Some((owner, _)) if owner != conn => Err(Note::NotOwner { conn }),
            None => Err(Note::NotOwner { conn }),
            Some((_, current)) if !any && current != generation => {
                Err(Note::StaleGeneration { conn })
            }
            Some(_) => Ok(()),
        }
    }

    fn update(&mut self, conn: ConnId, update: Update) -> Outcome {
        if let Err(note) = self.check_owner(conn, update.generation, false) {
            return Outcome::note(note);
        }
        let Some(shown) = self.shown.as_mut() else {
            // 持ち主が隠したあとの Update（同じ世代でも出し直さない。出すのは Show だけ）
            return Outcome::note(Note::StaleGeneration { conn });
        };
        let model = match limits::sanitize_model(update.model.unwrap_or_default()) {
            Ok(model) => model,
            Err(reason) => return Outcome::note(Note::Rejected { conn, reason }),
        };
        shown.show.model = Some(model);
        shown.show.side = update.side;
        self.render()
    }

    fn hide(&mut self, conn: ConnId, hide: Hide) -> Outcome {
        if let Err(note) =
            self.check_owner(conn, hide.generation, hide.generation == ANY_GENERATION)
        {
            return Outcome::note(note);
        }
        if self.shown.take().is_none() {
            return Outcome::default();
        }
        self.render()
    }

    fn drop_owner(&mut self, conn: ConnId) -> Outcome {
        if self.owner.is_some_and(|(owner, _)| owner == conn) {
            self.owner = None;
            if self.shown.take().is_some() {
                return self.render();
            }
        }
        Outcome::default()
    }

    fn render(&mut self) -> Outcome {
        self.revision += 1;
        Outcome {
            render: Some(self.render_state()),
            note: None,
        }
    }

    /// 今の描く状態。
    pub fn render_state(&self) -> RenderState {
        let Some(Shown { show }) = &self.shown else {
            return RenderState {
                visible: false,
                revision: self.revision,
                ..Default::default()
            };
        };
        let side = normalized_side(show.side);
        let rows = show.model.as_ref().map_or(0, |m| m.candidates.len());
        RenderState {
            visible: true,
            revision: self.revision,
            generation: show.generation,
            model: show.model.clone(),
            side: side.into(),
            anchor: show.anchor,
            work_area: show.work_area,
            scale: show.scale,
            display_order: display_order(rows, side),
        }
    }
}

/// 知らない値・指定なしは下（`Below`）として扱う（上下を入れ替えない側）。
fn normalized_side(side: i32) -> PanelSide {
    match PanelSide::try_from(side) {
        Ok(PanelSide::Above) => PanelSide::Above,
        _ => PanelSide::Below,
    }
}

/// 上から描く順の、候補の添字。`Above` なら逆順（1 位が下＝アンカーに近い側）。
pub fn display_order(rows: usize, side: PanelSide) -> Vec<u32> {
    let rows = u32::try_from(rows).unwrap_or(u32::MAX);
    match side {
        PanelSide::Above => (0..rows).rev().collect(),
        _ => (0..rows).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Hello;
    use ikari_proto::shell::v1::{CandidateItem, CandidatePanelModel, Rect};

    const A: ConnId = 1;
    const B: ConnId = 2;

    fn model(n: usize, selected: u32) -> CandidatePanelModel {
        CandidatePanelModel {
            candidates: (0..n)
                .map(|i| CandidateItem {
                    surface: format!("候補{i}"),
                    ..Default::default()
                })
                .collect(),
            selected,
            ..Default::default()
        }
    }

    fn show(generation: u64, side: PanelSide) -> candidate_command::Op {
        candidate_command::Op::Show(Show {
            generation,
            model: Some(model(3, 0)),
            anchor: Some(Rect {
                x: 100.0,
                y: 100.0,
                width: 1.0,
                height: 20.0,
            }),
            work_area: Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1040.0,
            }),
            side: side.into(),
            scale: 1.0,
        })
    }

    fn update(generation: u64, selected: u32) -> candidate_command::Op {
        candidate_command::Op::Update(Update {
            generation,
            model: Some(model(3, selected)),
            side: PanelSide::Below.into(),
        })
    }

    fn hide(generation: u64) -> candidate_command::Op {
        candidate_command::Op::Hide(Hide { generation })
    }

    fn bytes(seq: u64, op: candidate_command::Op) -> Vec<u8> {
        CandidateCommand { seq, op: Some(op) }.encode_to_vec()
    }

    fn setup() -> CandidateState {
        let mut s = CandidateState::new();
        s.connected(A);
        s.connected(B);
        s
    }

    fn visible(out: &Outcome) -> Option<bool> {
        out.render.as_ref().map(|r| r.visible)
    }

    #[test]
    fn show_update_hide_from_the_owner() {
        let mut s = setup();
        let out = s.received(A, &bytes(1, show(1, PanelSide::Below)));
        let r = out.render.unwrap();
        assert!(r.visible);
        assert_eq!(r.revision, 1);
        assert_eq!(r.display_order, vec![0, 1, 2]);
        let r = s.received(A, &bytes(2, update(1, 2))).render.unwrap();
        assert_eq!(r.model.unwrap().selected, 2);
        assert_eq!(r.revision, 2);
        assert_eq!(visible(&s.received(A, &bytes(3, hide(1)))), Some(false));
    }

    #[test]
    fn above_reverses_the_order() {
        let mut s = setup();
        let r = s
            .received(A, &bytes(1, show(1, PanelSide::Above)))
            .render
            .unwrap();
        assert_eq!(r.display_order, vec![2, 1, 0]);
        assert_eq!(r.side, PanelSide::Above as i32);
    }

    #[test]
    fn stale_seq_is_dropped_per_connection() {
        let mut s = setup();
        s.received(A, &bytes(5, show(1, PanelSide::Below)));
        let out = s.received(A, &bytes(5, update(1, 1)));
        assert_eq!(out.note, Some(Note::StaleSeq { conn: A, seq: 5 }));
        assert!(out.render.is_none());
        // 別の接続の seq は別に数える
        assert!(
            s.received(B, &bytes(1, show(1, PanelSide::Below)))
                .render
                .is_some()
        );
    }

    #[test]
    fn late_hide_from_another_app_does_not_hide_the_new_owner() {
        // #405: A から B へフォーカスが移ったあと、A の遅れた Hide{0} と Hide{1} が届く
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        s.received(B, &bytes(1, show(1, PanelSide::Below)));
        for (seq, generation) in [(2, ANY_GENERATION), (3, 1)] {
            let out = s.received(A, &bytes(seq, hide(generation)));
            assert_eq!(out.note, Some(Note::NotOwner { conn: A }));
            assert!(out.render.is_none());
        }
        assert!(s.render_state().visible);
        // A の Update も効かない
        assert_eq!(
            s.received(A, &bytes(4, update(1, 1))).note,
            Some(Note::NotOwner { conn: A })
        );
    }

    #[test]
    fn stale_generation_is_dropped_but_any_generation_hides() {
        let mut s = setup();
        s.received(A, &bytes(1, show(2, PanelSide::Below)));
        assert_eq!(
            s.received(A, &bytes(2, hide(1))).note,
            Some(Note::StaleGeneration { conn: A })
        );
        assert_eq!(
            s.received(A, &bytes(3, update(1, 1))).note,
            Some(Note::StaleGeneration { conn: A })
        );
        assert_eq!(
            visible(&s.received(A, &bytes(4, hide(ANY_GENERATION)))),
            Some(false)
        );
    }

    #[test]
    fn update_after_hide_does_not_show_again() {
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        s.received(A, &bytes(2, hide(1)));
        let out = s.received(A, &bytes(3, update(1, 1)));
        assert!(out.render.is_none());
        assert!(!s.render_state().visible);
    }

    #[test]
    fn owner_disconnect_hides_and_others_do_not() {
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        assert!(s.disconnected(B).render.is_none());
        assert_eq!(visible(&s.disconnected(A)), Some(false));
        // 持ち主がいなくなったあとの命令は効かない
        assert_eq!(
            s.received(B, &bytes(1, hide(ANY_GENERATION))).note,
            Some(Note::NotOwner { conn: B })
        );
    }

    #[test]
    fn malformed_bytes_poison_the_connection_and_hide_its_window() {
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        let out = s.received(A, &[0xFF, 0xFF, 0xFF]);
        assert_eq!(out.note, Some(Note::Malformed { conn: A }));
        assert_eq!(visible(&out), Some(false));
        // そのあとの正しい命令も捨てる
        let out = s.received(A, &bytes(2, show(2, PanelSide::Below)));
        assert_eq!(out, Outcome::default());
        assert!(!s.render_state().visible);
    }

    #[test]
    fn unknown_op_and_schema_mismatch_change_nothing() {
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        let empty = CandidateCommand { seq: 2, op: None }.encode_to_vec();
        let out = s.received(A, &empty);
        assert_eq!(out.note, Some(Note::UnknownOp { conn: A }));
        assert!(out.render.is_none());
        let hello = candidate_command::Op::Hello(Hello {
            schema_version: CANDIDATE_SCHEMA_VERSION + 1,
            pid: 42,
        });
        let out = s.received(A, &bytes(3, hello));
        assert_eq!(
            out.note,
            Some(Note::SchemaMismatch {
                conn: A,
                theirs: CANDIDATE_SCHEMA_VERSION + 1
            })
        );
        assert!(s.render_state().visible);
    }

    #[test]
    fn rejected_show_keeps_the_previous_window() {
        let mut s = setup();
        s.received(A, &bytes(1, show(1, PanelSide::Below)));
        let bad = candidate_command::Op::Show(Show {
            generation: 2,
            model: Some(model(3, 7)),
            ..match show(2, PanelSide::Below) {
                candidate_command::Op::Show(s) => s,
                _ => unreachable!(),
            }
        });
        let out = s.received(B, &bytes(1, bad));
        assert_eq!(
            out.note,
            Some(Note::Rejected {
                conn: B,
                reason: Rejection::SelectionOutOfRange
            })
        );
        // 持ち主は A のまま
        assert!(s.received(A, &bytes(2, update(1, 1))).render.is_some());
    }

    #[test]
    fn unknown_side_is_treated_as_below() {
        let mut s = setup();
        let mut op = show(1, PanelSide::Below);
        if let candidate_command::Op::Show(show) = &mut op {
            show.side = 99;
        }
        let r = s.received(A, &bytes(1, op)).render.unwrap();
        assert_eq!(r.side, PanelSide::Below as i32);
        assert_eq!(r.display_order, vec![0, 1, 2]);
    }
}
