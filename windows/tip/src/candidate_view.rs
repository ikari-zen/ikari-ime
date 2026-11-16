//! 候補窓の表示の状態（ADR 0062 §6.1、§7.4）。純粋（Mac でテストする）。判断の置き場所: Rust（ここ）。
//!
//! COM の層（`com::edit`）は、`TsfOp::ShowCandidates` / `MoveCandidates` / `HideCandidates` を受けたら、ここの
//! `show` / `update` / `hide` を呼ぶだけにする。ここが決めること:
//!
//! - **アンカーは `Hide` まで動かさない**（#86。macOS の `PanelUI.anchor` と同じ）。最初の `show` でだけ、
//!   アンカーと画面（作業領域、倍率）を決める。COM の層の「アンカーを取る」関数は、そのときだけ呼ぶ。
//! - アンカーが取れないときの順は `panel::resolve_anchor` と同じ（クライアント → 同じアプリの最後の正常値 →
//!   マウスのある画面）。アクセシビリティ（macOS の 2 番目）に当たるものは、Windows ではまだ空（ADR 0062 §6.1）。
//! - 送れたときだけ、`Shell::panel_shown` に渡す内容（上下と、新しく決めたアンカーの出どころ）を返す
//!   （ADR 0062 §5.4 の決定 3 の (c)）。
//! - 候補窓が落ちて戻ったとき（送れなかったあと）は、次の `update` で、固定したアンカーのまま `Show` を送り直す
//!   （新しい接続では、`Show` を送るまで窓の持ち主にならない。ADR 0062 §7.2）。

use std::time::Instant;

use ikari_shell_core::CandidatePanelModel;
use ikari_shell_core::panel::{
    AnchorSource, PanelSide, Point, Rect, Size, is_usable_anchor, resolve_anchor,
};

use crate::candidate_send::{
    CandidateConnector, CandidateLink, CandidatePlan, ScreenGeometry, plan_hide, plan_move,
    plan_show,
};

/// 画面 1 枚の、候補窓を置くのに要る値（物理の画面座標。左上が原点、y は下向き）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    /// 作業領域（タスクバーを除く）
    pub work_area: Rect,
    /// DPI / 96
    pub scale: f64,
}

/// OS の画面の情報（Windows は `com::screen`、テストは偽物）。
pub trait Desktop {
    /// `rect` を含む画面（なければいちばん近い画面）。取れなければ `None`。
    fn screen_of(&self, rect: &Rect) -> Option<Screen>;
    /// マウスのある画面。取れなければ `None`。
    fn cursor_screen(&self) -> Option<Screen>;
    /// 同じアプリ（プロセス）で最後に取れた正常なアンカー（ADR 0062 §6.1）。
    fn last_known(&self) -> Option<Rect>;
    /// クライアントから正常なアンカーが取れたときに覚える。
    fn remember(&self, anchor: Rect);
}

/// `Shell::panel_shown` に渡す内容（上下と、新しく決めたアンカーの出どころ）。
pub type PanelReport = (PanelSide, Option<AnchorSource>);

/// y の向きを反転する（y 下向き ↔ y 上向き。自分自身の逆）。`panel::layout_panel_y_down` と同じ写し方。
fn flip_y(rect: &Rect) -> Rect {
    Rect::new(
        rect.origin.x,
        -(rect.origin.y + rect.size.height),
        rect.size.width,
        rect.size.height,
    )
}

/// 最後の手段のアンカー（マウスのある画面の作業領域）。規則は `panel::resolve_anchor` のもの（y 上向き）を、
/// y を反転して使う（新しい規則を作らない）。
fn last_resort_anchor(work_area: &Rect) -> Rect {
    let up = resolve_anchor(None, None, None, &flip_y(work_area));
    flip_y(&up.rect)
}

/// アンカーと画面を決める。`client` はクライアントが返したアンカー（物理の画面座標）。
/// 画面がどれも取れなければ `None`（窓を出さない）。
pub fn resolve_geometry(client: Option<Rect>, desktop: &dyn Desktop) -> Option<ScreenGeometry> {
    // 使えるかの判定（有限、大きすぎない、原点が (0, 0) でない）は y の向きに依らない
    let (anchor, source) = match client.filter(is_usable_anchor) {
        Some(rect) => {
            desktop.remember(rect);
            (rect, AnchorSource::Client)
        }
        None => match desktop.last_known().filter(is_usable_anchor) {
            Some(rect) => (rect, AnchorSource::LastKnown),
            None => {
                let screen = desktop.cursor_screen()?;
                let anchor = last_resort_anchor(&screen.work_area);
                return Some(ScreenGeometry {
                    anchor,
                    work_area: screen.work_area,
                    scale: screen.scale,
                    source: Some(AnchorSource::MouseScreen),
                });
            }
        },
    };
    let screen = desktop
        .screen_of(&anchor)
        .or_else(|| desktop.cursor_screen())?;
    Some(ScreenGeometry {
        anchor,
        work_area: screen.work_area,
        scale: screen.scale,
        source: Some(source),
    })
}

/// 候補窓の表示の状態。TIP のインスタンス（アプリのスレッド）ごとに 1 つ持つ。
pub struct CandidateView<C: CandidateConnector> {
    link: CandidateLink<C>,
    /// `Hide` まで動かさないアンカーと画面
    anchored: Option<ScreenGeometry>,
    /// 今の接続で `Show` を送れていて、まだ `Hide` していない
    visible: bool,
}

impl<C: CandidateConnector> CandidateView<C> {
    pub fn new(connector: C) -> Self {
        Self {
            link: CandidateLink::new(connector),
            anchored: None,
            visible: false,
        }
    }

    /// 最後に送った命令の `seq`（診断ログ用。`dbgcap` と候補窓のログを突き合わせる）。
    pub fn last_seq(&self) -> Option<u64> {
        self.link.last_seq()
    }

    /// 固定したアンカーと画面（診断ログ用）。
    pub fn geometry(&self) -> Option<&ScreenGeometry> {
        self.anchored.as_ref()
    }

    /// 計画を送り、送れたら `report` を返す。送れなければ窓は見えていない扱いにする。
    fn send(&mut self, plan: &CandidatePlan, now: Instant) -> Option<PanelReport> {
        match self.link.send(plan, now) {
            Ok(()) => {
                self.visible = plan.command.is_some();
                plan.panel_shown
            }
            Err(_) => {
                self.visible = false;
                None
            }
        }
    }

    /// `ShowCandidates`。`client` はクライアントのアンカーを取る関数（アンカーを固定していないときだけ呼ぶ）。
    pub fn show(
        &mut self,
        model: &CandidatePanelModel,
        client: impl FnOnce() -> Option<Rect>,
        desktop: &dyn Desktop,
        now: Instant,
    ) -> Option<PanelReport> {
        let (geometry, fresh) = match self.anchored {
            Some(geometry) => (geometry, false),
            None => {
                let geometry = resolve_geometry(client(), desktop)?;
                self.anchored = Some(geometry);
                (geometry, true)
            }
        };
        let mut plan = plan_show(model, &geometry);
        if !fresh {
            // アンカーは取り直していない（macOS の `PanelUI.showCandidates` と同じ）
            plan.panel_shown = plan.panel_shown.map(|(side, _)| (side, None));
        }
        self.send(&plan, now)
    }

    /// `MoveCandidates`。窓を出していなければ（`Show` の前）何もしない。前の送りが失敗していたら、固定した
    /// アンカーのまま `Show` を送り直す（つなぎ直しは送り口の間隔に従う）。
    pub fn update(&mut self, model: &CandidatePanelModel, now: Instant) -> Option<PanelReport> {
        let geometry = self.anchored?;
        let plan = if self.visible {
            plan_move(model, &geometry)
        } else {
            let mut plan = plan_show(model, &geometry);
            plan.panel_shown = plan.panel_shown.map(|(side, _)| (side, None));
            plan
        };
        self.send(&plan, now)
    }

    /// `HideCandidates`、文書のフォーカスが外れたとき。アンカーを捨てる。つながっているときだけ `Hide` を送る
    /// （隠すためだけに、つなぎに行かない。切れていれば、候補窓は持ち主の接続が切れたときに自分で隠す）。
    pub fn hide(&mut self, now: Instant) {
        self.anchored = None;
        self.visible = false;
        if self.link.is_connected() {
            let _ = self.link.send(&plan_hide(), now);
        }
    }

    /// TIP が外れるとき（`Deactivate`）。隠してから接続を閉じる。
    pub fn close(&mut self, now: Instant) {
        self.hide(now);
        self.link.close();
    }
}

/// 物理の画面座標の 2 点（左上と右下）から矩形を作る（COM の層が `RECT` を写すときに使う）。
pub fn rect_from_corners(top_left: Point, bottom_right: Point) -> Rect {
    Rect {
        origin: top_left,
        size: Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate_send::{CandidatePost, SendError};
    use ikari_candidate::command::{CandidateCommand, candidate_command};
    use ikari_shell_core::CandidateItem;
    use prost::Message;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    const WORK: Rect = Rect::new(0.0, 0.0, 1920.0, 1040.0);
    const SECOND_WORK: Rect = Rect::new(1920.0, 0.0, 2560.0, 1400.0);

    fn model(n: usize) -> CandidatePanelModel {
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
            reserved_rows: 0,
            developer_note: String::new(),
        }
    }

    /// 画面 2 枚（左は倍率 1、右は倍率 1.5）。マウスは左。
    #[derive(Default)]
    struct FakeDesktop {
        last: Cell<Option<Rect>>,
        no_screens: bool,
    }

    impl Desktop for FakeDesktop {
        fn screen_of(&self, rect: &Rect) -> Option<Screen> {
            if self.no_screens {
                return None;
            }
            Some(if rect.min_x() >= SECOND_WORK.min_x() {
                Screen {
                    work_area: SECOND_WORK,
                    scale: 1.5,
                }
            } else {
                Screen {
                    work_area: WORK,
                    scale: 1.0,
                }
            })
        }

        fn cursor_screen(&self) -> Option<Screen> {
            (!self.no_screens).then_some(Screen {
                work_area: WORK,
                scale: 1.0,
            })
        }

        fn last_known(&self) -> Option<Rect> {
            self.last.get()
        }

        fn remember(&self, anchor: Rect) {
            self.last.set(Some(anchor));
        }
    }

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

    impl FakeConnector {
        /// 今の接続で送った命令の種類（`Hello` を除く）
        fn kinds(&self) -> Vec<&'static str> {
            let current = self.current.borrow();
            let Some(rec) = current.as_ref() else {
                return Vec::new();
            };
            rec.sent
                .borrow()
                .iter()
                .filter_map(|c| match c.op.as_ref()? {
                    candidate_command::Op::Hello(_) => None,
                    candidate_command::Op::Show(_) => Some("show"),
                    candidate_command::Op::Update(_) => Some("update"),
                    candidate_command::Op::Hide(_) => Some("hide"),
                })
                .collect()
        }

        fn shown_anchor(&self) -> Option<Rect> {
            let current = self.current.borrow();
            current
                .as_ref()?
                .sent
                .borrow()
                .iter()
                .rev()
                .find_map(|c| match c.op.as_ref()? {
                    candidate_command::Op::Show(show) => {
                        let a = show.anchor.as_ref()?;
                        Some(Rect::new(a.x, a.y, a.width, a.height))
                    }
                    _ => None,
                })
        }
    }

    const CLIENT: Rect = Rect::new(100.0, 200.0, 1.0, 20.0);

    #[test]
    fn the_anchor_is_resolved_once_and_kept_until_hide() {
        let connector = FakeConnector::default();
        let desktop = FakeDesktop::default();
        let mut view = CandidateView::new(&connector);
        let now = Instant::now();
        let calls = Cell::new(0);
        let probe = || {
            calls.set(calls.get() + 1);
            Some(CLIENT)
        };
        assert_eq!(
            view.show(&model(3), probe, &desktop, now),
            Some((PanelSide::Below, Some(AnchorSource::Client)))
        );
        // 2 回目の Show は、アンカーを取り直さない（出どころも返さない）
        let moved = Rect::new(500.0, 600.0, 1.0, 20.0);
        assert_eq!(
            view.show(&model(3), || Some(moved), &desktop, now),
            Some((PanelSide::Below, None))
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(connector.shown_anchor(), Some(CLIENT));
        assert_eq!(view.update(&model(3), now), Some((PanelSide::Below, None)));
        view.hide(now);
        assert_eq!(connector.kinds(), vec!["show", "show", "update", "hide"]);
        // Hide のあとは取り直す
        view.show(&model(3), || Some(moved), &desktop, now);
        assert_eq!(connector.shown_anchor(), Some(moved));
        assert_eq!(desktop.last_known(), Some(moved));
    }

    #[test]
    fn falls_back_to_the_last_known_anchor_then_to_the_mouse_screen() {
        let desktop = FakeDesktop::default();
        // 何も取れない: マウスのある画面の、横の中央、上から 3/4（`resolve_anchor` の規則を y 下向きにしたもの）
        let geometry = resolve_geometry(None, &desktop).unwrap();
        assert_eq!(geometry.source, Some(AnchorSource::MouseScreen));
        assert_eq!(geometry.anchor, Rect::new(960.0, 780.0, 0.0, 0.0));
        assert_eq!(geometry.work_area, WORK);

        // クライアントが (0, 0) を返した（取れなかった）: 最後の正常値
        desktop.remember(CLIENT);
        let geometry = resolve_geometry(Some(Rect::new(0.0, 0.0, 0.0, 17.0)), &desktop).unwrap();
        assert_eq!(geometry.source, Some(AnchorSource::LastKnown));
        assert_eq!(geometry.anchor, CLIENT);
        // 使えない値は覚えない
        assert_eq!(desktop.last_known(), Some(CLIENT));
    }

    #[test]
    fn the_screen_is_the_one_holding_the_anchor() {
        let desktop = FakeDesktop::default();
        let anchor = Rect::new(2000.0, 300.0, 1.0, 30.0);
        let geometry = resolve_geometry(Some(anchor), &desktop).unwrap();
        assert_eq!(geometry.work_area, SECOND_WORK);
        assert_eq!(geometry.scale, 1.5);
    }

    #[test]
    fn no_screen_shows_nothing_and_does_not_connect() {
        let connector = FakeConnector::default();
        let desktop = FakeDesktop {
            no_screens: true,
            ..Default::default()
        };
        let mut view = CandidateView::new(&connector);
        let now = Instant::now();
        assert_eq!(view.show(&model(1), || None, &desktop, now), None);
        assert_eq!(view.update(&model(1), now), None);
        assert_eq!(connector.opened.get(), 0);
    }

    #[test]
    fn a_refused_window_reports_nothing_so_the_side_stays_below() {
        let connector = FakeConnector::default();
        connector.refuse.set(true);
        let desktop = FakeDesktop::default();
        let mut view = CandidateView::new(&connector);
        let now = Instant::now();
        assert_eq!(view.show(&model(3), || Some(CLIENT), &desktop, now), None);
        assert_eq!(view.update(&model(3), now), None);
    }

    #[test]
    fn update_before_show_does_nothing() {
        let connector = FakeConnector::default();
        let mut view = CandidateView::new(&connector);
        assert_eq!(view.update(&model(1), Instant::now()), None);
        assert_eq!(connector.opened.get(), 0);
    }

    #[test]
    fn hide_does_not_connect_just_to_hide() {
        let connector = FakeConnector::default();
        let mut view = CandidateView::new(&connector);
        view.hide(Instant::now());
        view.close(Instant::now());
        assert_eq!(connector.opened.get(), 0);
    }

    #[test]
    fn after_the_window_comes_back_the_next_update_shows_again_with_the_same_anchor() {
        let connector = FakeConnector::default();
        let desktop = FakeDesktop::default();
        let mut view = CandidateView::new(&connector);
        let t0 = Instant::now();
        view.show(&model(3), || Some(CLIENT), &desktop, t0);
        // 窓が落ちた: 送れない
        connector.current.borrow().as_ref().unwrap().fail.set(true);
        assert_eq!(view.update(&model(3), t0), None);
        // 間隔のあと、新しい接続では Show から（アンカーは固定のまま、出どころは返さない）
        let later = t0 + crate::transport::BACKOFF_INITIAL + Duration::from_millis(1);
        assert_eq!(
            view.update(&model(3), later),
            Some((PanelSide::Below, None))
        );
        assert_eq!(connector.opened.get(), 2);
        assert_eq!(connector.kinds(), vec!["show"]);
        assert_eq!(connector.shown_anchor(), Some(CLIENT));
        assert!(view.last_seq().is_some());
    }

    #[test]
    fn close_hides_and_drops_the_connection() {
        let connector = FakeConnector::default();
        let desktop = FakeDesktop::default();
        let mut view = CandidateView::new(&connector);
        let now = Instant::now();
        view.show(&model(1), || Some(CLIENT), &desktop, now);
        view.close(now);
        assert_eq!(connector.kinds(), vec!["show", "hide"]);
        assert!(view.geometry().is_none());
        assert_eq!(view.last_seq(), None);
    }

    #[test]
    fn corners_make_a_rect() {
        let rect = rect_from_corners(Point { x: 10.0, y: 20.0 }, Point { x: 12.0, y: 50.0 });
        assert_eq!(rect, Rect::new(10.0, 20.0, 2.0, 30.0));
    }
}
