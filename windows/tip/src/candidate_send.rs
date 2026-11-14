//! 候補窓（`ikari-candidate.exe`）への片道の送り口（ADR 0062 §3、§7.4）。**宣言とスタブ**（中身は L1 が実装する）。
//!
//! `TsfOp::ShowCandidates` / `MoveCandidates` / `HideCandidates` は変えない。COM の層がアンカー（物理の画面座標）、
//! 作業領域、倍率を取り、`plan_*` に渡す。`plan_*` は純粋な関数で、「送る命令」と「`Shell::panel_shown` に渡す内容」を返す
//! （Mac でテストする）。COM の層は、送ることと `panel_shown` を呼ぶことだけをする。判断の置き場所: Rust（ここ）。

use ikari_candidate::command::CandidateCommand;
use ikari_shell_core::CandidatePanelModel;
use ikari_shell_core::panel::{AnchorSource, PanelSide, Rect};

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
    /// 送る命令（送らないなら `None`）。`seq` は送り口（`CandidateSender`）が付ける
    pub command: Option<CandidateCommand>,
    /// `Shell::panel_shown(side, source)` に渡す内容（呼ばないなら `None`）
    pub panel_shown: Option<(PanelSide, Option<AnchorSource>)>,
}

impl CandidatePlan {
    /// 何もしない計画（スタブの戻り値）。
    const NOTHING: Self = Self {
        command: None,
        panel_shown: None,
    };
}

/// `ShowCandidates` の計画。スタブ（何も送らず、何も呼ばない。L1 が実装する）。
pub fn plan_show(_model: &CandidatePanelModel, _geometry: &ScreenGeometry) -> CandidatePlan {
    CandidatePlan::NOTHING
}

/// `MoveCandidates` の計画（行数が変わって `side` が変わる場合に備えて計算し直す）。スタブ。
pub fn plan_move(_model: &CandidatePanelModel, _geometry: &ScreenGeometry) -> CandidatePlan {
    CandidatePlan::NOTHING
}

/// `HideCandidates`、文書のフォーカスが外れたとき、`Deactivate` の計画。スタブ。
pub fn plan_hide() -> CandidatePlan {
    CandidatePlan::NOTHING
}

/// 送れなかった理由（入力の内容は含めない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// 候補窓につながっていない（窓を出さずに入力を続ける。つなぎ直しは L1）
    NotConnected,
    /// まだ実装されていない（スタブ）
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

/// 命令に `seq`（接続ごとに 1 から増える）を付けて符号化し、送り先に渡す。スタブ（L1 が実装する）。
pub struct CandidateSender<P: CandidatePost> {
    _post: P,
}

impl<P: CandidatePost> CandidateSender<P> {
    pub fn new(post: P) -> Self {
        Self { _post: post }
    }

    /// 1 つ送る。スタブ（`NotImplemented`）。
    pub fn send(&mut self, _command: CandidateCommand) -> Result<(), SendError> {
        Err(SendError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder;
    impl CandidatePost for Recorder {
        fn post(&self, _bytes: &[u8]) -> Result<(), SendError> {
            Ok(())
        }
    }

    #[test]
    fn stubs_send_nothing_and_change_no_behavior() {
        let geometry = ScreenGeometry {
            anchor: Rect::new(10.0, 10.0, 0.0, 18.0),
            work_area: Rect::new(0.0, 0.0, 1920.0, 1040.0),
            scale: 1.0,
            source: Some(AnchorSource::Client),
        };
        let model = CandidatePanelModel {
            candidates: Vec::new(),
            selected: 0,
            annotation: String::new(),
            reserved_rows: 0,
            developer_note: String::new(),
        };
        assert_eq!(plan_show(&model, &geometry).command, None);
        assert_eq!(plan_move(&model, &geometry).panel_shown, None);
        assert_eq!(plan_hide().command, None);
        let mut sender = CandidateSender::new(Recorder);
        assert_eq!(
            sender.send(CandidateCommand::default()),
            Err(SendError::NotImplemented)
        );
    }
}
