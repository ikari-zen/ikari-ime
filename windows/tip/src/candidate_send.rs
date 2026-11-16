//! 候補窓（`ikari-candidate.exe`）への片道の送り口（ADR 0062 §3、§5.4、§7.4）。純粋（Mac でテストする）。
//!
//! `TsfOp::ShowCandidates` / `MoveCandidates` / `HideCandidates` は変えない。COM の層がアンカー（物理の画面座標）、
//! 作業領域、倍率を取り、`plan_*` に渡す。`plan_*` は純粋な関数で、「送る命令」と「`Shell::panel_shown` に渡す内容」を返す。
//! COM の層は、`CandidateLink::send` で送り、**送れたときだけ** `panel_shown` を呼ぶ（送れなければ上下は `Below` のまま。
//! 見えない窓に合わせて ↑/↓ を入れ替えない。ADR 0062 §5.4 の決定 3 の (c)）。判断の置き場所: Rust（ここ）。
//!
//! - 上下（`side`）は TIP が決める。行の高さ（`ikari_candidate::metrics`。行数と行の種類だけで決まる）× 倍率で
//!   `panel::layout_panel_y_down` を呼ぶ。予約の行（`reserved_rows`）の高さで決め、候補が増えても上下が入れ替わらない。
//! - `seq`（接続ごとに 1 から）と世代（`Show` ごとに 1 から。`Update` と `Hide` は今の世代）は送り口が付ける。
//!   `plan_*` の命令の `seq` と世代は 0 のまま。
//! - 接続したら、最初に `Hello` を送る。
//! - 送れなかったら（書きの締め切り `WRITE_DEADLINE` を超えた、切れた）接続を捨てる。再送しない。次に送るときに、
//!   エンジンと同じ間隔（`transport::BACKOFF_*`）でつなぎ直す。つなぐ試行は 1 回の送りにつき多くて 1 回
//!   （キーの処理を、接続の試行 1 回と書きの締め切りより長く止めない。決定 3 の (b)）。
//! - 相手の確認（S4。ADR 0068 §5）は、接続の手段（`CandidateConnector`。Windows は `com::connector`）が、送る前に行う。

use std::time::Instant;

use ikari_candidate::command::{
    ANY_GENERATION, CANDIDATE_SCHEMA_VERSION, CandidateCommand, Hello, Hide, Show, Update,
    candidate_command,
};
use ikari_candidate::metrics::{PanelRows, panel_height_dip};
use ikari_shell_core::CandidatePanelModel;
use ikari_shell_core::panel::{AnchorSource, PanelSide, Rect, Size, layout_panel_y_down};
use ikari_shell_core::panel_pb::{panel_to_pb, rect_to_pb, side_to_pb};
use prost::Message;

use crate::transport::{BACKOFF_INITIAL, BACKOFF_MAX, Backoff};

/// COM の層が取った、画面の上の位置の材料（すべて物理の画面座標。左上が原点、y は下向き）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenGeometry {
    /// 合成の先頭の 1 文字の矩形（`Hide` まで動かさない。ADR 0062 §6.1）
    pub anchor: Rect,
    /// アンカーの画面の作業領域（タスクバーを除く）
    pub work_area: Rect,
    /// DPI / 96
    pub scale: f64,
    /// アンカーがどこから取れたか（診断ログ）。`panel_shown` に渡す
    pub source: Option<AnchorSource>,
}

/// `plan_*` の結果。
#[derive(Debug, Clone, PartialEq)]
pub struct CandidatePlan {
    /// 送る命令（送らないなら `None`）。`seq` と世代は送り口（`CandidateSender`）が付ける
    pub command: Option<CandidateCommand>,
    /// 送れたときに `Shell::panel_shown(side, source)` に渡す内容（呼ばないなら `None`）
    pub panel_shown: Option<(PanelSide, Option<AnchorSource>)>,
}

impl CandidatePlan {
    /// 何もしない計画。
    const NOTHING: Self = Self {
        command: None,
        panel_shown: None,
    };
}

/// 倍率が使える値か（有限で正）。使えなければ窓を出さない（候補窓の側でも捨てる）。
fn scale_ok(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

/// モデルと位置から上下を決める。予約の行の高さ（実際の行数より多ければ）で決める。
fn decide_side(model: &CandidatePanelModel, geometry: &ScreenGeometry) -> PanelSide {
    let rows = PanelRows::of(model);
    let height = panel_height_dip(rows) * geometry.scale;
    let reserved = panel_height_dip(rows.with_candidate_rows(model.reserved_rows)) * geometry.scale;
    let size = Size { width: 0.0, height };
    layout_panel_y_down(&geometry.anchor, size, Some(reserved), &geometry.work_area).side
}

fn command(op: candidate_command::Op) -> Option<CandidateCommand> {
    Some(CandidateCommand {
        seq: 0,
        op: Some(op),
    })
}

/// `ShowCandidates` の計画。
pub fn plan_show(model: &CandidatePanelModel, geometry: &ScreenGeometry) -> CandidatePlan {
    if !scale_ok(geometry.scale) {
        return CandidatePlan::NOTHING;
    }
    let side = decide_side(model, geometry);
    CandidatePlan {
        command: command(candidate_command::Op::Show(Show {
            generation: 0,
            model: Some(panel_to_pb(model)),
            anchor: Some(rect_to_pb(&geometry.anchor)),
            work_area: Some(rect_to_pb(&geometry.work_area)),
            side: side_to_pb(side).into(),
            scale: geometry.scale,
        })),
        panel_shown: Some((side, geometry.source)),
    }
}

/// `MoveCandidates` の計画（行数が変わって `side` が変わる場合に備えて計算し直す）。`geometry` は `Show` のときのもの
/// （アンカーは `Hide` まで動かさない）。アンカーは取り直していないので、`panel_shown` の `source` は `None`。
pub fn plan_move(model: &CandidatePanelModel, geometry: &ScreenGeometry) -> CandidatePlan {
    if !scale_ok(geometry.scale) {
        return CandidatePlan::NOTHING;
    }
    let side = decide_side(model, geometry);
    CandidatePlan {
        command: command(candidate_command::Op::Update(Update {
            generation: 0,
            model: Some(panel_to_pb(model)),
            side: side_to_pb(side).into(),
        })),
        panel_shown: Some((side, None)),
    }
}

/// `HideCandidates`、文書のフォーカスが外れたとき、`Deactivate` の計画。世代は送り口が今のものを付ける
/// （この接続でまだ `Show` を送っていなければ「世代を問わず隠す」）。
pub fn plan_hide() -> CandidatePlan {
    CandidatePlan {
        command: command(candidate_command::Op::Hide(Hide {
            generation: ANY_GENERATION,
        })),
        panel_shown: None,
    }
}

/// 送れなかった理由（入力の内容は含めない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// 候補窓につながっていない・切れた（窓を出さずに入力を続ける）
    NotConnected,
    /// まだ実装されていない（今は使わない。契約の互換のために残す）
    NotImplemented,
}

/// 片道の送り先。本番は `pipe::Client`（`post`。返事を待たない）、テストは記録するだけの偽物。
pub trait CandidatePost {
    fn post(&self, bytes: &[u8]) -> Result<(), SendError>;
}

impl CandidatePost for ikari_pipe::Client {
    fn post(&self, bytes: &[u8]) -> Result<(), SendError> {
        ikari_pipe::Client::post(self, bytes).map_err(|_| SendError::NotConnected)
    }
}

/// 命令に `seq`（接続ごとに 1 から増える）と世代を付けて符号化し、送り先に渡す。接続 1 本ぶん。
pub struct CandidateSender<P: CandidatePost> {
    post: P,
    /// 送り手（このアプリ）のプロセスの番号。`Hello` に入れる（ログ用）
    pid: u32,
    hello_sent: bool,
    last_seq: u64,
    /// この接続で最後に送った `Show` の世代（0 はまだ送っていない）
    generation: u64,
}

impl<P: CandidatePost> CandidateSender<P> {
    pub fn new(post: P) -> Self {
        Self::with_pid(post, std::process::id())
    }

    pub fn with_pid(post: P, pid: u32) -> Self {
        Self {
            post,
            pid,
            hello_sent: false,
            last_seq: 0,
            generation: 0,
        }
    }

    fn post_one(&mut self, mut command: CandidateCommand) -> Result<(), SendError> {
        self.last_seq += 1;
        command.seq = self.last_seq;
        self.post.post(&command.encode_to_vec())
    }

    /// 1 つ送る（まだなら先に `Hello`）。`Show` は新しい世代、`Update` と `Hide` は今の世代にする。
    pub fn send(&mut self, mut command: CandidateCommand) -> Result<(), SendError> {
        if !self.hello_sent {
            self.post_one(CandidateCommand {
                seq: 0,
                op: Some(candidate_command::Op::Hello(Hello {
                    schema_version: CANDIDATE_SCHEMA_VERSION,
                    pid: self.pid,
                })),
            })?;
            self.hello_sent = true;
        }
        match command.op.as_mut() {
            Some(candidate_command::Op::Show(show)) => {
                self.generation += 1;
                show.generation = self.generation;
            }
            Some(candidate_command::Op::Update(update)) => update.generation = self.generation,
            Some(candidate_command::Op::Hide(hide)) => hide.generation = self.generation,
            Some(candidate_command::Op::Hello(_)) | None => {}
        }
        self.post_one(command)
    }
}

/// 候補窓への接続を開く手段。相手の確認（S4）は、ここで送る前に行い、違えば `None`。
pub trait CandidateConnector {
    type Post: CandidatePost;

    fn open(&self) -> Option<Self::Post>;
}

/// 候補窓への接続（つなぎ直しを含む）。TIP のインスタンス（アプリのスレッド）ごとに 1 つ持つ。
pub struct CandidateLink<C: CandidateConnector> {
    connector: C,
    sender: Option<CandidateSender<C::Post>>,
    backoff: Backoff,
}

impl<C: CandidateConnector> CandidateLink<C> {
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            sender: None,
            backoff: Backoff::new(BACKOFF_INITIAL, BACKOFF_MAX),
        }
    }

    /// つながっているか（診断ログ用）。
    pub fn is_connected(&self) -> bool {
        self.sender.is_some()
    }

    /// 計画の命令を送る。命令がなければ何もしない（`Ok`）。送れなかったら接続を捨てる。
    /// 戻りが `Ok` のときだけ、COM の層は `plan.panel_shown` を `Shell::panel_shown` に渡す。
    pub fn send(&mut self, plan: &CandidatePlan, now: Instant) -> Result<(), SendError> {
        let Some(command) = plan.command.clone() else {
            return Ok(());
        };
        if self.sender.is_none() {
            if !self.backoff.ready(now) {
                return Err(SendError::NotConnected);
            }
            match self.connector.open() {
                Some(post) => {
                    self.backoff.succeeded();
                    self.sender = Some(CandidateSender::new(post));
                }
                None => {
                    self.backoff.failed(now);
                    return Err(SendError::NotConnected);
                }
            }
        }
        let Some(sender) = self.sender.as_mut() else {
            return Err(SendError::NotConnected);
        };
        let result = sender.send(command);
        if result.is_err() {
            // 再送しない。次に送るときにつなぎ直す（切れた相手が戻るまでの試行は間隔を置く）
            self.sender = None;
            self.backoff.failed(now);
        }
        result
    }

    /// 接続を捨てる（TIP が外れるとき）。候補窓は、持ち主の接続が切れたら窓を隠す。
    pub fn close(&mut self) {
        self.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikari_shell_core::CandidateItem;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    const WORK: Rect = Rect::new(0.0, 0.0, 1920.0, 1040.0);

    fn model(n: usize, reserved_rows: u32) -> CandidatePanelModel {
        CandidatePanelModel {
            candidates: (0..n)
                .map(|i| CandidateItem {
                    surface: format!("候補{i}"),
                    annotation: String::new(),
                    kind: Default::default(),
                    label: String::new(),
                })
                .collect(),
            selected: 0,
            annotation: String::new(),
            reserved_rows,
            developer_note: String::new(),
        }
    }

    fn geometry(anchor_y: f64, scale: f64) -> ScreenGeometry {
        ScreenGeometry {
            anchor: Rect::new(100.0, anchor_y, 1.0, 20.0),
            work_area: WORK,
            scale,
            source: Some(AnchorSource::Client),
        }
    }

    fn op(plan: &CandidatePlan) -> &candidate_command::Op {
        plan.command.as_ref().unwrap().op.as_ref().unwrap()
    }

    #[test]
    fn show_below_when_there_is_room() {
        let plan = plan_show(&model(3, 9), &geometry(100.0, 1.0));
        assert_eq!(
            plan.panel_shown,
            Some((PanelSide::Below, Some(AnchorSource::Client)))
        );
        let candidate_command::Op::Show(show) = op(&plan) else {
            panic!("not a show");
        };
        assert_eq!(show.side, side_to_pb(PanelSide::Below) as i32);
        assert_eq!(show.scale, 1.0);
        assert_eq!(show.model.as_ref().unwrap().candidates.len(), 3);
    }

    #[test]
    fn side_is_decided_with_the_reserved_rows_and_the_scale() {
        // 9 行の予約（26 × 9 = 234 DIP）。下の余白は 1040 - 820 = 220 物理ピクセル
        let anchor_y = 800.0;
        // 3 行なら入るが、予約の 9 行は入らないので上
        assert_eq!(
            plan_show(&model(3, 9), &geometry(anchor_y, 1.0))
                .panel_shown
                .unwrap()
                .0,
            PanelSide::Above
        );
        // 予約がなければ（3 行 = 78）下に入る
        assert_eq!(
            plan_show(&model(3, 0), &geometry(anchor_y, 1.0))
                .panel_shown
                .unwrap()
                .0,
            PanelSide::Below
        );
        // 倍率 3 では 3 行（234 物理ピクセル）も入らない
        assert_eq!(
            plan_show(&model(3, 0), &geometry(anchor_y, 3.0))
                .panel_shown
                .unwrap()
                .0,
            PanelSide::Above
        );
    }

    #[test]
    fn move_recomputes_the_side_without_a_new_source() {
        let plan = plan_move(&model(3, 9), &geometry(800.0, 1.0));
        assert_eq!(plan.panel_shown, Some((PanelSide::Above, None)));
        assert!(matches!(op(&plan), candidate_command::Op::Update(_)));
    }

    #[test]
    fn bad_scale_sends_nothing() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                plan_show(&model(1, 0), &geometry(100.0, scale)),
                CandidatePlan::NOTHING
            );
            assert_eq!(
                plan_move(&model(1, 0), &geometry(100.0, scale)),
                CandidatePlan::NOTHING
            );
        }
    }

    #[test]
    fn hide_does_not_call_panel_shown() {
        let plan = plan_hide();
        assert_eq!(plan.panel_shown, None);
        assert!(matches!(op(&plan), candidate_command::Op::Hide(_)));
    }

    /// 送ったものを記録する。`fail` が立っていれば失敗する。
    #[derive(Clone, Default)]
    struct Recorder {
        sent: Rc<RefCell<Vec<CandidateCommand>>>,
        fail: Rc<Cell<bool>>,
    }

    impl CandidatePost for Recorder {
        fn post(&self, bytes: &[u8]) -> Result<(), SendError> {
            if self.fail.get() {
                return Err(SendError::NotConnected);
            }
            self.sent
                .borrow_mut()
                .push(CandidateCommand::decode(bytes).unwrap());
            Ok(())
        }
    }

    #[test]
    fn sender_sends_hello_first_and_stamps_seq_and_generation() {
        let rec = Recorder::default();
        let mut sender = CandidateSender::with_pid(rec.clone(), 42);
        let g = geometry(100.0, 1.0);
        for plan in [
            plan_hide(),
            plan_show(&model(2, 0), &g),
            plan_move(&model(2, 0), &g),
            plan_hide(),
            plan_show(&model(2, 0), &g),
        ] {
            sender.send(plan.command.unwrap()).unwrap();
        }
        let sent = rec.sent.borrow();
        let seqs: Vec<u64> = sent.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
        let stamps: Vec<(&str, u64)> = sent
            .iter()
            .map(|c| match c.op.as_ref().unwrap() {
                candidate_command::Op::Hello(h) => {
                    assert_eq!(h.pid, 42);
                    assert_eq!(h.schema_version, CANDIDATE_SCHEMA_VERSION);
                    ("hello", 0)
                }
                candidate_command::Op::Show(s) => ("show", s.generation),
                candidate_command::Op::Update(u) => ("update", u.generation),
                candidate_command::Op::Hide(h) => ("hide", h.generation),
            })
            .collect();
        assert_eq!(
            stamps,
            vec![
                ("hello", 0),
                // まだ Show を送っていない接続の Hide は、世代を問わない
                ("hide", ANY_GENERATION),
                ("show", 1),
                ("update", 1),
                ("hide", 1),
                ("show", 2),
            ]
        );
    }

    /// 開くたびに新しい記録を作る接続の手段。`refuse` が立っていれば開けない（相手の確認で断った、つながらない）。
    #[derive(Default)]
    struct FakeConnector {
        opened: Cell<u32>,
        refuse: Cell<bool>,
        current: RefCell<Option<Recorder>>,
    }

    impl CandidateConnector for &FakeConnector {
        type Post = Recorder;

        fn open(&self) -> Option<Recorder> {
            self.opened.set(self.opened.get() + 1);
            if self.refuse.get() {
                return None;
            }
            let rec = Recorder::default();
            *self.current.borrow_mut() = Some(rec.clone());
            Some(rec)
        }
    }

    #[test]
    fn link_connects_lazily_and_reconnects_after_a_failure_with_backoff() {
        let connector = FakeConnector::default();
        let mut link = CandidateLink::new(&connector);
        let t0 = Instant::now();
        let show = plan_show(&model(1, 0), &geometry(100.0, 1.0));
        // 命令がない計画は、つながない
        link.send(&CandidatePlan::NOTHING, t0).unwrap();
        assert_eq!(connector.opened.get(), 0);

        link.send(&show, t0).unwrap();
        assert_eq!(connector.opened.get(), 1);
        assert!(link.is_connected());

        // 書きが失敗した: 接続を捨てる。すぐには試さない
        connector.current.borrow().as_ref().unwrap().fail.set(true);
        assert_eq!(link.send(&show, t0), Err(SendError::NotConnected));
        assert!(!link.is_connected());
        assert_eq!(link.send(&show, t0), Err(SendError::NotConnected));
        assert_eq!(connector.opened.get(), 1);

        // 間隔のあとにつなぎ直し、新しい接続では Hello から始める
        let later = t0 + BACKOFF_INITIAL + Duration::from_millis(1);
        link.send(&show, later).unwrap();
        assert_eq!(connector.opened.get(), 2);
        let sent = connector
            .current
            .borrow()
            .as_ref()
            .unwrap()
            .sent
            .borrow()
            .clone();
        assert!(matches!(sent[0].op, Some(candidate_command::Op::Hello(_))));
        assert_eq!(sent[0].seq, 1);
    }

    #[test]
    fn refused_peer_sends_nothing_and_waits_before_trying_again() {
        let connector = FakeConnector::default();
        connector.refuse.set(true);
        let mut link = CandidateLink::new(&connector);
        let t0 = Instant::now();
        let show = plan_show(&model(1, 0), &geometry(100.0, 1.0));
        assert_eq!(link.send(&show, t0), Err(SendError::NotConnected));
        // 1 回の送りにつき、つなぐ試行は多くて 1 回。間隔の中では試さない
        assert_eq!(link.send(&show, t0), Err(SendError::NotConnected));
        assert_eq!(connector.opened.get(), 1);
        assert!(connector.current.borrow().is_none());
    }
}
