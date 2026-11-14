use ikari_shell_core::{
    CandidateItem, CandidateKind, CandidatePanelModel, ConnId, MarkedText, MenuModel, SettingsTab,
    TimerId, Utf16Len, Utf16Range,
};

use super::*;

fn panel() -> CandidatePanelModel {
    CandidatePanelModel {
        developer_note: String::new(),
        candidates: vec![CandidateItem {
            surface: "日本語".to_owned(),
            annotation: String::new(),
            kind: CandidateKind::Dictionary,
            label: String::new(),
        }],
        selected: 0,
        annotation: String::new(),
        reserved_rows: 1,
    }
}

fn range() -> Utf16Range {
    Utf16Range {
        location: Utf16Len(2),
        length: Utf16Len(3),
    }
}

/// `ShellAction` の全形を 1 つずつ（並びは `index` と同じ）
fn all_actions() -> Vec<ShellAction> {
    vec![
        ShellAction::PassThrough,
        ShellAction::Consume,
        ShellAction::SetMarkedText(MarkedText::plain("かな")),
        ShellAction::Commit {
            text: "確定".to_owned(),
        },
        ShellAction::ShowCandidates(panel()),
        ShellAction::MoveSelection(panel()),
        ShellAction::HideCandidates,
        ShellAction::DeleteText { range: range() },
        ShellAction::Restore {
            text: "元".to_owned(),
            replacing: range(),
        },
        ShellAction::ResolveStrayMarkedText,
        ShellAction::Prefetch {
            reading: "よみ".to_owned(),
        },
        ShellAction::ShowMenu(MenuModel::default()),
        ShellAction::ScheduleTimer {
            id: TimerId(1),
            after_ms: 50,
        },
        ShellAction::CancelTimer { id: TimerId(1) },
        ShellAction::BridgeReply {
            conn: ConnId(7),
            frame: vec![1, 2, 3],
        },
        ShellAction::OpenSettings {
            tab: Some(SettingsTab::Input),
        },
        ShellAction::SelectMode {
            mode_id: "ja".to_owned(),
        },
        ShellAction::BridgeClose { conn: ConnId(7) },
        ShellAction::ShowLearningQuarantineNotice,
    ]
}

/// 全形を網羅する `match`（`_` なし）。`ShellAction` に形が足されるとここでコンパイルが落ち、
/// `all_actions` に足すのを忘れない（`translate` 本体も同じ）
fn index(action: &ShellAction) -> usize {
    match action {
        ShellAction::PassThrough => 0,
        ShellAction::Consume => 1,
        ShellAction::SetMarkedText(_) => 2,
        ShellAction::Commit { .. } => 3,
        ShellAction::ShowCandidates(_) => 4,
        ShellAction::MoveSelection(_) => 5,
        ShellAction::HideCandidates => 6,
        ShellAction::DeleteText { .. } => 7,
        ShellAction::Restore { .. } => 8,
        ShellAction::ResolveStrayMarkedText => 9,
        ShellAction::Prefetch { .. } => 10,
        ShellAction::ShowMenu(_) => 11,
        ShellAction::ScheduleTimer { .. } => 12,
        ShellAction::CancelTimer { .. } => 13,
        ShellAction::BridgeReply { .. } => 14,
        ShellAction::OpenSettings { .. } => 15,
        ShellAction::SelectMode { .. } => 16,
        ShellAction::BridgeClose { .. } => 17,
        ShellAction::ShowLearningQuarantineNotice => 18,
    }
}

#[test]
fn every_action_form_is_listed_once() {
    let actions = all_actions();
    for (position, action) in actions.iter().enumerate() {
        assert_eq!(index(action), position);
    }
}

#[test]
fn every_action_translates() {
    // 空の列になってよいのは、OS 側で操作が要らない 3 つ（Consume、Prefetch、ShowMenu）だけ
    let silent = [1, 10, 11];
    for action in all_actions() {
        let ops = OpsTranslator::new().translate(&action);
        let position = index(&action);
        // 空のマーク文字列は合成がなければ空になるが、all_actions のものは空でない
        assert_eq!(ops.is_empty(), silent.contains(&position), "{action:?}");
    }
}

#[test]
fn one_to_one_forms() {
    let mut t = OpsTranslator::new();
    let cases = [
        (ShellAction::PassThrough, TsfOp::PassThrough),
        (
            ShellAction::ShowCandidates(panel()),
            TsfOp::ShowCandidates(panel()),
        ),
        (
            ShellAction::MoveSelection(panel()),
            TsfOp::MoveCandidates(panel()),
        ),
        (ShellAction::HideCandidates, TsfOp::HideCandidates),
        (
            ShellAction::DeleteText { range: range() },
            TsfOp::DeleteText { range: range() },
        ),
        (
            ShellAction::Restore {
                text: "元".to_owned(),
                replacing: range(),
            },
            TsfOp::RestoreText {
                text: "元".to_owned(),
                replacing: range(),
            },
        ),
        (
            ShellAction::ScheduleTimer {
                id: TimerId(4),
                after_ms: 30,
            },
            TsfOp::ScheduleTimer {
                id: TimerId(4),
                after_ms: 30,
            },
        ),
        (
            ShellAction::CancelTimer { id: TimerId(4) },
            TsfOp::CancelTimer { id: TimerId(4) },
        ),
        (
            ShellAction::OpenSettings { tab: None },
            TsfOp::OpenSettings { tab: None },
        ),
        (
            ShellAction::SelectMode {
                mode_id: "en".to_owned(),
            },
            TsfOp::SelectMode {
                mode_id: "en".to_owned(),
            },
        ),
        (
            ShellAction::BridgeReply {
                conn: ConnId(1),
                frame: vec![9],
            },
            TsfOp::BridgeReply {
                conn: ConnId(1),
                frame: vec![9],
            },
        ),
        (
            ShellAction::BridgeClose { conn: ConnId(1) },
            TsfOp::BridgeClose { conn: ConnId(1) },
        ),
        (
            ShellAction::ShowLearningQuarantineNotice,
            TsfOp::ShowLearningQuarantineNotice,
        ),
    ];
    for (action, expected) in cases {
        assert_eq!(t.translate(&action), vec![expected], "{action:?}");
    }
}

#[test]
fn composition_start_update_commit() {
    let mut t = OpsTranslator::new();
    let first = ShellAction::SetMarkedText(MarkedText::plain("か"));
    let second = ShellAction::SetMarkedText(MarkedText::plain("かな"));
    // 最初だけ開始が付く
    assert_eq!(
        t.translate(&first),
        vec![
            TsfOp::StartComposition,
            TsfOp::UpdateComposition(MarkedText::plain("か"))
        ]
    );
    assert!(t.is_composing());
    assert_eq!(
        t.translate(&second),
        vec![TsfOp::UpdateComposition(MarkedText::plain("かな"))]
    );
    let commit = ShellAction::Commit {
        text: "仮名".to_owned(),
    };
    assert_eq!(
        t.translate(&commit),
        vec![TsfOp::CommitComposition {
            text: "仮名".to_owned()
        }]
    );
    assert!(!t.is_composing());
    // 確定のあとは、また開始から
    assert_eq!(t.translate(&first)[0], TsfOp::StartComposition);
}

#[test]
fn empty_marked_text_ends_composition_once() {
    let mut t = OpsTranslator::new();
    let clear = ShellAction::SetMarkedText(MarkedText::plain(""));
    assert!(t.translate(&clear).is_empty(), "合成がなければ何もしない");
    t.translate(&ShellAction::SetMarkedText(MarkedText::plain("か")));
    assert_eq!(t.translate(&clear), vec![TsfOp::EndComposition]);
    assert!(!t.is_composing());
    assert!(t.translate(&clear).is_empty());
}

#[test]
fn stray_and_reset_forget_composition() {
    let mut t = OpsTranslator::new();
    t.translate(&ShellAction::SetMarkedText(MarkedText::plain("か")));
    assert_eq!(
        t.translate(&ShellAction::ResolveStrayMarkedText),
        vec![TsfOp::ResolveStrayComposition]
    );
    assert!(!t.is_composing());
    t.translate(&ShellAction::SetMarkedText(MarkedText::plain("か")));
    t.reset();
    assert!(!t.is_composing());
}

#[test]
fn translate_all_keeps_order_and_key_consumed() {
    let mut t = OpsTranslator::new();
    let ops = t.translate_all(&[
        ShellAction::Consume,
        ShellAction::SetMarkedText(MarkedText::plain("か")),
        ShellAction::ShowCandidates(panel()),
    ]);
    assert_eq!(
        ops,
        vec![
            TsfOp::StartComposition,
            TsfOp::UpdateComposition(MarkedText::plain("か")),
            TsfOp::ShowCandidates(panel()),
        ]
    );
    assert!(key_consumed(&ops));
    assert!(key_consumed(&[]));
    let passed = t.translate_all(&[ShellAction::HideCandidates, ShellAction::PassThrough]);
    assert!(!key_consumed(&passed));
}
