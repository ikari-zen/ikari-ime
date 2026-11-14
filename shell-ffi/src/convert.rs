//! `shell-core` の型と `ikari.shell.v1`（proto）の写し（ADR 0041 §4.4、§7.3 M1-b）。
//!
//! 欄の意味は `shell-core` の `types.rs` を正とする。写しは両方向とも持つ:
//! - FFI が使う向き: イベント（proto → 型）、動作（型 → proto）、ホストの返事（proto → 型）、設定（両方）
//! - 逆の向き: テストと、`shell-core` を直接使わない呼び手が往復を確かめるため
//!
//! proto → 型 は、型で表せない値（oneof が空、列挙が範囲外・UNSPECIFIED、数が `u8` を超える）を
//! `ConvertError` にする（黙って既定値に寄せない）。

use std::collections::BTreeMap;

use ikari_proto::shell::v1 as pb;
use ikari_shell_core::panel::{AnchorSource, PanelSide, Rect};
use ikari_shell_core::{
    CandidateItem, CandidateKind, CandidatePanelModel, ConnId, InputMethodKind, KeyInput, KeyKind,
    KeymapPreset, MarkedText, MenuCommand, MenuItem, MenuItemKind, MenuModel, MenuRadioGroup,
    Modifiers, SelectedText, SettingsChanges, SettingsEntry, SettingsIssue, SettingsIssueKind,
    SettingsSnapshot, SettingsTab, SettingsValidation, ShellAction, ShellEvent, SurroundingText,
    TimerId, Utf16Len, Utf16Range,
};

/// proto のバイト列を型にできなかった理由（ユーザーの文字列は含めない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// protobuf として読めない
    Decode,
    /// 必須の欄（oneof、入れ子のメッセージ）がない。欄の名前
    Missing(&'static str),
    /// 列挙か数が型の範囲の外。欄の名前
    OutOfRange(&'static str),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode => f.write_str("not a valid protobuf message"),
            Self::Missing(field) => write!(f, "missing field: {field}"),
            Self::OutOfRange(field) => write!(f, "value out of range: {field}"),
        }
    }
}

impl std::error::Error for ConvertError {}

type Result<T> = std::result::Result<T, ConvertError>;

// ---------------------------------------------------------------- バイト列

/// `ShellEvent` のバイト列を読む（`ikari_shell_dispatch` の入力）。
pub fn decode_event(bytes: &[u8]) -> Result<ShellEvent> {
    let message =
        <pb::ShellEvent as prost::Message>::decode(bytes).map_err(|_| ConvertError::Decode)?;
    event_from_pb(message)
}

/// `ShellEvent` をバイト列にする（テストと、Rust から FFI を呼ぶ側）。
pub fn encode_event(event: &ShellEvent) -> Vec<u8> {
    prost::Message::encode_to_vec(&event_to_pb(event))
}

/// 動作の列を `ShellActions` のバイト列にする（`ikari_shell_dispatch` の出力）。
pub fn encode_actions(actions: &[ShellAction]) -> Vec<u8> {
    let message = pb::ShellActions {
        actions: actions.iter().map(action_to_pb).collect(),
    };
    prost::Message::encode_to_vec(&message)
}

/// `ShellActions` のバイト列を読む（テストと、Rust から FFI を呼ぶ側）。
pub fn decode_actions(bytes: &[u8]) -> Result<Vec<ShellAction>> {
    let message =
        <pb::ShellActions as prost::Message>::decode(bytes).map_err(|_| ConvertError::Decode)?;
    message.actions.into_iter().map(action_from_pb).collect()
}

/// ホストの `surrounding` の返事を読む。
pub fn decode_surrounding(bytes: &[u8]) -> Result<SurroundingText> {
    let message =
        <pb::SurroundingText as prost::Message>::decode(bytes).map_err(|_| ConvertError::Decode)?;
    Ok(SurroundingText {
        left: message.left,
        right: message.right,
        document_length: Utf16Len(message.document_length),
        caret: message.caret.map(range_from_pb),
    })
}

/// ホストの `selection` の返事を読む。
pub fn decode_selection(bytes: &[u8]) -> Result<SelectedText> {
    let message =
        <pb::SelectedText as prost::Message>::decode(bytes).map_err(|_| ConvertError::Decode)?;
    Ok(SelectedText {
        range: range_from_pb(
            message
                .range
                .ok_or(ConvertError::Missing("SelectedText.range"))?,
        ),
        text: message.text,
    })
}

/// `SettingsChanges` のバイト列を読む（`ikari_settings_rewrite` の入力）。
pub fn decode_settings_changes(bytes: &[u8]) -> Result<SettingsChanges> {
    let message =
        <pb::SettingsChanges as prost::Message>::decode(bytes).map_err(|_| ConvertError::Decode)?;
    Ok(SettingsChanges {
        set: message
            .set
            .into_iter()
            .map(|entry| SettingsEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect(),
        reset: message.reset,
    })
}

/// `SettingsValidation` をバイト列にする（`ikari_settings_validate` の出力）。
pub fn encode_settings_validation(validation: &SettingsValidation) -> Vec<u8> {
    let message = pb::SettingsValidation {
        snapshot: Some(snapshot_to_pb(&validation.snapshot)),
        issues: validation.issues.iter().map(issue_to_pb).collect(),
    };
    prost::Message::encode_to_vec(&message)
}

/// `rewrite` の結果を `SettingsRewrite` のバイト列にする（`ikari_settings_rewrite` の出力）。
pub fn encode_settings_rewrite(result: std::result::Result<&str, &[SettingsIssue]>) -> Vec<u8> {
    let result = match result {
        Ok(text) => pb::settings_rewrite::Result::Text(text.to_owned()),
        Err(issues) => pb::settings_rewrite::Result::IssuesAdded(pb::SettingsIssues {
            issues: issues.iter().map(issue_to_pb).collect(),
        }),
    };
    prost::Message::encode_to_vec(&pb::SettingsRewrite {
        result: Some(result),
    })
}

// ---------------------------------------------------------------- 共通

fn range_from_pb(range: pb::Utf16Range) -> Utf16Range {
    Utf16Range {
        location: Utf16Len(range.location),
        length: Utf16Len(range.length),
    }
}

fn range_to_pb(range: &Utf16Range) -> pb::Utf16Range {
    pb::Utf16Range {
        location: range.location.get(),
        length: range.length.get(),
    }
}

pub(crate) fn modifiers_from_pb(modifiers: Option<pb::Modifiers>) -> Modifiers {
    modifiers.map_or(Modifiers::NONE, |m| Modifiers {
        shift: m.shift,
        control: m.control,
        option: m.option,
        command: m.command,
    })
}

pub(crate) fn modifiers_to_pb(modifiers: &Modifiers) -> Option<pb::Modifiers> {
    (!modifiers.is_empty()).then_some(pb::Modifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        option: modifiers.option,
        command: modifiers.command,
    })
}

fn enum_value<T: TryFrom<i32>>(value: i32, field: &'static str) -> Result<T> {
    T::try_from(value).map_err(|_| ConvertError::OutOfRange(field))
}

fn small(value: u32, field: &'static str) -> Result<u8> {
    u8::try_from(value).map_err(|_| ConvertError::OutOfRange(field))
}

// ---------------------------------------------------------------- キー

pub(crate) fn key_from_pb(key: pb::KeyInput) -> Result<KeyInput> {
    let kind = match enum_value::<pb::KeyKind>(key.kind, "KeyInput.kind")? {
        pb::KeyKind::Unspecified => return Err(ConvertError::OutOfRange("KeyInput.kind")),
        pb::KeyKind::Character => KeyKind::Character { text: key.text },
        pb::KeyKind::Jis => KeyKind::Jis {
            code: small(key.jis, "KeyInput.jis")?,
        },
        pb::KeyKind::Space => KeyKind::Space,
        pb::KeyKind::Enter => KeyKind::Enter,
        pb::KeyKind::Escape => KeyKind::Escape,
        pb::KeyKind::Backspace => KeyKind::Backspace,
        pb::KeyKind::Left => KeyKind::Left,
        pb::KeyKind::Right => KeyKind::Right,
        pb::KeyKind::Up => KeyKind::Up,
        pb::KeyKind::Down => KeyKind::Down,
        pb::KeyKind::Function => KeyKind::Function {
            number: small(key.function, "KeyInput.function")?,
        },
        pb::KeyKind::Other => KeyKind::Other,
        pb::KeyKind::Eisu => KeyKind::Eisu,
        pb::KeyKind::Kana => KeyKind::Kana,
    };
    // layout_text は JIS のキーにだけ意味がある（ほかでは捨てる）
    let layout_text = matches!(kind, KeyKind::Jis { .. })
        .then_some(key.layout_text)
        .flatten();
    Ok(KeyInput {
        key: kind,
        modifiers: modifiers_from_pb(key.modifiers),
        is_repeat: key.is_repeat,
        secure: key.secure,
        layout_text,
    })
}

fn key_to_pb(key: &KeyInput) -> pb::KeyInput {
    let mut message = pb::KeyInput {
        modifiers: modifiers_to_pb(&key.modifiers),
        is_repeat: key.is_repeat,
        secure: key.secure,
        layout_text: key.layout_text.clone(),
        ..Default::default()
    };
    let kind = match &key.key {
        KeyKind::Character { text } => {
            message.text = text.clone();
            pb::KeyKind::Character
        }
        KeyKind::Jis { code } => {
            message.jis = u32::from(*code);
            pb::KeyKind::Jis
        }
        KeyKind::Space => pb::KeyKind::Space,
        KeyKind::Enter => pb::KeyKind::Enter,
        KeyKind::Escape => pb::KeyKind::Escape,
        KeyKind::Backspace => pb::KeyKind::Backspace,
        KeyKind::Left => pb::KeyKind::Left,
        KeyKind::Right => pb::KeyKind::Right,
        KeyKind::Up => pb::KeyKind::Up,
        KeyKind::Down => pb::KeyKind::Down,
        KeyKind::Function { number } => {
            message.function = u32::from(*number);
            pb::KeyKind::Function
        }
        KeyKind::Other => pb::KeyKind::Other,
        KeyKind::Eisu => pb::KeyKind::Eisu,
        KeyKind::Kana => pb::KeyKind::Kana,
    };
    message.kind = kind.into();
    message
}

// ---------------------------------------------------------------- 表示

fn marked_from_pb(marked: pb::MarkedText) -> MarkedText {
    MarkedText {
        text: marked.text,
        highlighted_length: Utf16Len(marked.highlighted_length),
        segments: marked.segments.into_iter().map(Utf16Len).collect(),
        focused: marked.focused,
        replacement_range: marked.replacement_range.map(range_from_pb),
        underlined: marked.underlined,
        delete_target: marked.delete_target,
    }
}

fn marked_to_pb(marked: &MarkedText) -> pb::MarkedText {
    pb::MarkedText {
        text: marked.text.clone(),
        highlighted_length: marked.highlighted_length.get(),
        segments: marked.segments.iter().map(|len| len.get()).collect(),
        focused: marked.focused,
        replacement_range: marked.replacement_range.as_ref().map(range_to_pb),
        underlined: marked.underlined,
        delete_target: marked.delete_target,
    }
}

fn candidate_kind_from_pb(kind: i32) -> Result<CandidateKind> {
    Ok(
        match enum_value::<pb::CandidateKind>(kind, "CandidateItem.kind")? {
            pb::CandidateKind::Unspecified => CandidateKind::Unspecified,
            pb::CandidateKind::Dictionary => CandidateKind::Dictionary,
            pb::CandidateKind::Hiragana => CandidateKind::Hiragana,
            pb::CandidateKind::Katakana => CandidateKind::Katakana,
            pb::CandidateKind::Rewriter => CandidateKind::Rewriter,
            pb::CandidateKind::Literal => CandidateKind::Literal,
        },
    )
}

fn candidate_kind_to_pb(kind: CandidateKind) -> pb::CandidateKind {
    match kind {
        CandidateKind::Unspecified => pb::CandidateKind::Unspecified,
        CandidateKind::Dictionary => pb::CandidateKind::Dictionary,
        CandidateKind::Hiragana => pb::CandidateKind::Hiragana,
        CandidateKind::Katakana => pb::CandidateKind::Katakana,
        CandidateKind::Rewriter => pb::CandidateKind::Rewriter,
        CandidateKind::Literal => pb::CandidateKind::Literal,
    }
}

fn panel_from_pb(panel: pb::CandidatePanelModel) -> Result<CandidatePanelModel> {
    let candidates = panel
        .candidates
        .into_iter()
        .map(|item| {
            Ok(CandidateItem {
                surface: item.surface,
                annotation: item.annotation,
                kind: candidate_kind_from_pb(item.kind)?,
                label: item.label,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CandidatePanelModel {
        developer_note: panel.developer_note,
        candidates,
        selected: panel.selected,
        annotation: panel.annotation,
        reserved_rows: panel.reserved_rows,
    })
}

fn panel_to_pb(panel: &CandidatePanelModel) -> pb::CandidatePanelModel {
    pb::CandidatePanelModel {
        developer_note: panel.developer_note.clone(),
        candidates: panel
            .candidates
            .iter()
            .map(|item| pb::CandidateItem {
                surface: item.surface.clone(),
                annotation: item.annotation.clone(),
                kind: candidate_kind_to_pb(item.kind).into(),
                label: item.label.clone(),
            })
            .collect(),
        selected: panel.selected,
        annotation: panel.annotation.clone(),
        reserved_rows: panel.reserved_rows,
    }
}

// ---------------------------------------------------------------- メニュー

pub(crate) fn preset_from_pb(value: i32, field: &'static str) -> Result<KeymapPreset> {
    match enum_value::<pb::KeymapPreset>(value, field)? {
        pb::KeymapPreset::Unspecified => Err(ConvertError::OutOfRange(field)),
        pb::KeymapPreset::Macos => Ok(KeymapPreset::Macos),
        pb::KeymapPreset::MsIme => Ok(KeymapPreset::MsIme),
        pb::KeymapPreset::Atok => Ok(KeymapPreset::Atok),
    }
}

pub(crate) fn preset_to_pb(preset: KeymapPreset) -> pb::KeymapPreset {
    match preset {
        KeymapPreset::Macos => pb::KeymapPreset::Macos,
        KeymapPreset::MsIme => pb::KeymapPreset::MsIme,
        KeymapPreset::Atok => pb::KeymapPreset::Atok,
    }
}

pub(crate) fn method_from_pb(value: i32, field: &'static str) -> Result<InputMethodKind> {
    match enum_value::<pb::InputMethodKind>(value, field)? {
        pb::InputMethodKind::Unspecified => Err(ConvertError::OutOfRange(field)),
        pb::InputMethodKind::Romaji => Ok(InputMethodKind::Romaji),
        pb::InputMethodKind::Kana => Ok(InputMethodKind::Kana),
    }
}

pub(crate) fn method_to_pb(method: InputMethodKind) -> pb::InputMethodKind {
    match method {
        InputMethodKind::Romaji => pb::InputMethodKind::Romaji,
        InputMethodKind::Kana => pb::InputMethodKind::Kana,
    }
}

/// UNSPECIFIED は「前回のタブ」（`None`）。
fn tab_from_pb(value: i32) -> Result<Option<SettingsTab>> {
    Ok(
        match enum_value::<pb::SettingsTab>(value, "OpenSettings.tab")? {
            pb::SettingsTab::Unspecified => None,
            pb::SettingsTab::Input => Some(SettingsTab::Input),
            pb::SettingsTab::Conversion => Some(SettingsTab::Conversion),
            pb::SettingsTab::UserDictionary => Some(SettingsTab::UserDictionary),
            pb::SettingsTab::Learning => Some(SettingsTab::Learning),
            pb::SettingsTab::About => Some(SettingsTab::About),
        },
    )
}

fn tab_to_pb(tab: Option<SettingsTab>) -> pb::OpenSettings {
    let tab = match tab {
        None => pb::SettingsTab::Unspecified,
        Some(SettingsTab::Input) => pb::SettingsTab::Input,
        Some(SettingsTab::Conversion) => pb::SettingsTab::Conversion,
        Some(SettingsTab::UserDictionary) => pb::SettingsTab::UserDictionary,
        Some(SettingsTab::Learning) => pb::SettingsTab::Learning,
        Some(SettingsTab::About) => pb::SettingsTab::About,
    };
    pb::OpenSettings { tab: tab.into() }
}

fn menu_command_from_pb(command: pb::MenuCommand) -> Result<MenuCommand> {
    use pb::menu_command::Command;
    Ok(
        match command
            .command
            .ok_or(ConvertError::Missing("MenuCommand.command"))?
        {
            Command::SelectKeymapPreset(preset) => MenuCommand::SelectKeymapPreset {
                preset: preset_from_pb(preset, "MenuCommand.select_keymap_preset")?,
            },
            Command::SelectInputMethod(method) => MenuCommand::SelectInputMethod {
                method: method_from_pb(method, "MenuCommand.select_input_method")?,
            },
            Command::SetLm(enabled) => MenuCommand::SetLm { enabled },
            Command::SetProfileDomain(domain) => MenuCommand::SetProfileDomain {
                id: domain.id,
                included: domain.included,
            },
            Command::OpenSettings(open) => MenuCommand::OpenSettings {
                tab: tab_from_pb(open.tab)?,
            },
        },
    )
}

fn menu_command_to_pb(command: &MenuCommand) -> pb::MenuCommand {
    use pb::menu_command::Command;
    let command = match command {
        MenuCommand::SelectKeymapPreset { preset } => {
            Command::SelectKeymapPreset(preset_to_pb(*preset).into())
        }
        MenuCommand::SelectInputMethod { method } => {
            Command::SelectInputMethod(method_to_pb(*method).into())
        }
        MenuCommand::SetLm { enabled } => Command::SetLm(*enabled),
        MenuCommand::SetProfileDomain { id, included } => {
            Command::SetProfileDomain(pb::SetProfileDomain {
                id: id.clone(),
                included: *included,
            })
        }
        MenuCommand::OpenSettings { tab } => Command::OpenSettings(tab_to_pb(*tab)),
    };
    pb::MenuCommand {
        command: Some(command),
    }
}

fn menu_item_from_pb(item: pb::MenuItem) -> Result<MenuItem> {
    let kind = match enum_value::<pb::MenuItemKind>(item.kind, "MenuItem.kind")? {
        pb::MenuItemKind::Unspecified => return Err(ConvertError::OutOfRange("MenuItem.kind")),
        pb::MenuItemKind::Separator => MenuItemKind::Separator,
        pb::MenuItemKind::Action => MenuItemKind::Action,
        pb::MenuItemKind::Toggle => MenuItemKind::Toggle,
        pb::MenuItemKind::Radio => MenuItemKind::Radio {
            group: match enum_value::<pb::MenuRadioGroup>(item.group, "MenuItem.group")? {
                pb::MenuRadioGroup::Unspecified => {
                    return Err(ConvertError::OutOfRange("MenuItem.group"));
                }
                pb::MenuRadioGroup::KeymapPreset => MenuRadioGroup::KeymapPreset,
                pb::MenuRadioGroup::InputMethod => MenuRadioGroup::InputMethod,
            },
        },
    };
    Ok(MenuItem {
        kind,
        title: item.title,
        command: item.command.map(menu_command_from_pb).transpose()?,
        checked: item.checked,
        enabled: item.enabled,
    })
}

fn menu_item_to_pb(item: &MenuItem) -> pb::MenuItem {
    let (kind, group) = match item.kind {
        MenuItemKind::Separator => (pb::MenuItemKind::Separator, pb::MenuRadioGroup::Unspecified),
        MenuItemKind::Action => (pb::MenuItemKind::Action, pb::MenuRadioGroup::Unspecified),
        MenuItemKind::Toggle => (pb::MenuItemKind::Toggle, pb::MenuRadioGroup::Unspecified),
        MenuItemKind::Radio { group } => (
            pb::MenuItemKind::Radio,
            match group {
                MenuRadioGroup::KeymapPreset => pb::MenuRadioGroup::KeymapPreset,
                MenuRadioGroup::InputMethod => pb::MenuRadioGroup::InputMethod,
            },
        ),
    };
    pb::MenuItem {
        kind: kind.into(),
        group: group.into(),
        title: item.title.clone(),
        command: item.command.as_ref().map(menu_command_to_pb),
        checked: item.checked,
        enabled: item.enabled,
    }
}

// ---------------------------------------------------------------- イベントと動作

fn event_from_pb(event: pb::ShellEvent) -> Result<ShellEvent> {
    use pb::shell_event::Event;
    Ok(
        match event
            .event
            .ok_or(ConvertError::Missing("ShellEvent.event"))?
        {
            Event::Key(key) => ShellEvent::Key(key_from_pb(key)?),
            Event::Activate(activate) => ShellEvent::Activate {
                bundle_id: activate.bundle_id,
                secure: activate.secure,
            },
            Event::Deactivate(_) => ShellEvent::Deactivate,
            Event::MenuCommand(command) => ShellEvent::MenuCommand {
                command: menu_command_from_pb(command)?,
            },
            Event::SettingsFileChanged(_) => ShellEvent::SettingsFileChanged,
            Event::Timer(timer) => ShellEvent::Timer {
                id: TimerId(timer.id),
            },
            Event::BridgeFrame(frame) => ShellEvent::BridgeFrame {
                conn: ConnId(frame.conn),
                frame: frame.frame,
            },
            Event::InputModeChanged(changed) => ShellEvent::InputModeChanged {
                mode_id: changed.mode_id,
                secure: changed.secure,
            },
            Event::CommitComposition(_) => ShellEvent::CommitComposition,
            Event::PanelShown(shown) => ShellEvent::PanelShown {
                side: side_from_pb(shown.side)?,
                source: source_from_pb(shown.source)?,
            },
            Event::PanelHidden(_) => ShellEvent::PanelHidden,
            Event::BridgeClosed(closed) => ShellEvent::BridgeClosed {
                conn: ConnId(closed.conn),
            },
        },
    )
}

fn event_to_pb(event: &ShellEvent) -> pb::ShellEvent {
    use pb::shell_event::Event;
    let event = match event {
        ShellEvent::Key(key) => Event::Key(key_to_pb(key)),
        ShellEvent::Activate { bundle_id, secure } => Event::Activate(pb::Activate {
            bundle_id: bundle_id.clone(),
            secure: *secure,
        }),
        ShellEvent::Deactivate => Event::Deactivate(pb::Empty {}),
        ShellEvent::MenuCommand { command } => Event::MenuCommand(menu_command_to_pb(command)),
        ShellEvent::SettingsFileChanged => Event::SettingsFileChanged(pb::Empty {}),
        ShellEvent::Timer { id } => Event::Timer(pb::TimerFired { id: id.0 }),
        ShellEvent::BridgeFrame { conn, frame } => Event::BridgeFrame(pb::BridgeFrame {
            conn: conn.0,
            frame: frame.clone(),
        }),
        ShellEvent::InputModeChanged { mode_id, secure } => {
            Event::InputModeChanged(pb::InputModeChanged {
                mode_id: mode_id.clone(),
                secure: *secure,
            })
        }
        ShellEvent::CommitComposition => Event::CommitComposition(pb::Empty {}),
        ShellEvent::PanelShown { side, source } => Event::PanelShown(pb::PanelShown {
            side: side_to_pb(*side).into(),
            source: source_to_pb(*source).into(),
        }),
        ShellEvent::PanelHidden => Event::PanelHidden(pb::Empty {}),
        ShellEvent::BridgeClosed { conn } => Event::BridgeClosed(pb::BridgeClosed { conn: conn.0 }),
    };
    pb::ShellEvent { event: Some(event) }
}

fn action_to_pb(action: &ShellAction) -> pb::ShellAction {
    use pb::shell_action::Action;
    let action = match action {
        ShellAction::PassThrough => Action::PassThrough(pb::Empty {}),
        ShellAction::Consume => Action::Consume(pb::Empty {}),
        ShellAction::SetMarkedText(marked) => Action::SetMarkedText(marked_to_pb(marked)),
        ShellAction::Commit { text } => Action::Commit(pb::Commit { text: text.clone() }),
        ShellAction::ShowCandidates(panel) => Action::ShowCandidates(panel_to_pb(panel)),
        ShellAction::MoveSelection(panel) => Action::MoveSelection(panel_to_pb(panel)),
        ShellAction::HideCandidates => Action::HideCandidates(pb::Empty {}),
        ShellAction::Restore { text, replacing } => Action::Restore(pb::Restore {
            text: text.clone(),
            replacing: Some(range_to_pb(replacing)),
        }),
        ShellAction::DeleteText { range } => Action::DeleteText(pb::DeleteText {
            range: Some(range_to_pb(range)),
        }),
        ShellAction::ResolveStrayMarkedText => Action::ResolveStrayMarkedText(pb::Empty {}),
        ShellAction::Prefetch { reading } => Action::Prefetch(pb::Prefetch {
            reading: reading.clone(),
        }),
        ShellAction::ShowMenu(menu) => Action::ShowMenu(pb::MenuModel {
            items: menu.items.iter().map(menu_item_to_pb).collect(),
        }),
        ShellAction::ScheduleTimer { id, after_ms } => Action::ScheduleTimer(pb::ScheduleTimer {
            id: id.0,
            after_ms: *after_ms,
        }),
        ShellAction::CancelTimer { id } => Action::CancelTimer(pb::CancelTimer { id: id.0 }),
        ShellAction::BridgeReply { conn, frame } => Action::BridgeReply(pb::BridgeReply {
            conn: conn.0,
            frame: frame.clone(),
        }),
        ShellAction::OpenSettings { tab } => Action::OpenSettings(tab_to_pb(*tab)),
        ShellAction::SelectMode { mode_id } => Action::SelectMode(pb::SelectMode {
            mode_id: mode_id.clone(),
        }),
        ShellAction::BridgeClose { conn } => Action::BridgeClose(pb::BridgeClose { conn: conn.0 }),
        ShellAction::ShowLearningQuarantineNotice => {
            Action::ShowLearningQuarantineNotice(pb::Empty {})
        }
    };
    pb::ShellAction {
        action: Some(action),
    }
}

fn action_from_pb(action: pb::ShellAction) -> Result<ShellAction> {
    use pb::shell_action::Action;
    Ok(
        match action
            .action
            .ok_or(ConvertError::Missing("ShellAction.action"))?
        {
            Action::PassThrough(_) => ShellAction::PassThrough,
            Action::Consume(_) => ShellAction::Consume,
            Action::SetMarkedText(marked) => ShellAction::SetMarkedText(marked_from_pb(marked)),
            Action::Commit(commit) => ShellAction::Commit { text: commit.text },
            Action::ShowCandidates(panel) => ShellAction::ShowCandidates(panel_from_pb(panel)?),
            Action::MoveSelection(panel) => ShellAction::MoveSelection(panel_from_pb(panel)?),
            Action::HideCandidates(_) => ShellAction::HideCandidates,
            Action::Restore(restore) => ShellAction::Restore {
                text: restore.text,
                replacing: range_from_pb(
                    restore
                        .replacing
                        .ok_or(ConvertError::Missing("Restore.replacing"))?,
                ),
            },
            Action::DeleteText(delete) => ShellAction::DeleteText {
                range: range_from_pb(
                    delete
                        .range
                        .ok_or(ConvertError::Missing("DeleteText.range"))?,
                ),
            },
            Action::ResolveStrayMarkedText(_) => ShellAction::ResolveStrayMarkedText,
            Action::Prefetch(prefetch) => ShellAction::Prefetch {
                reading: prefetch.reading,
            },
            Action::ShowMenu(menu) => ShellAction::ShowMenu(MenuModel {
                items: menu
                    .items
                    .into_iter()
                    .map(menu_item_from_pb)
                    .collect::<Result<Vec<_>>>()?,
            }),
            Action::ScheduleTimer(timer) => ShellAction::ScheduleTimer {
                id: TimerId(timer.id),
                after_ms: timer.after_ms,
            },
            Action::CancelTimer(timer) => ShellAction::CancelTimer {
                id: TimerId(timer.id),
            },
            Action::BridgeReply(reply) => ShellAction::BridgeReply {
                conn: ConnId(reply.conn),
                frame: reply.frame,
            },
            Action::OpenSettings(open) => ShellAction::OpenSettings {
                tab: tab_from_pb(open.tab)?,
            },
            Action::SelectMode(select) => ShellAction::SelectMode {
                mode_id: select.mode_id,
            },
            Action::BridgeClose(close) => ShellAction::BridgeClose {
                conn: ConnId(close.conn),
            },
            Action::ShowLearningQuarantineNotice(_) => ShellAction::ShowLearningQuarantineNotice,
        },
    )
}

// ---------------------------------------------------------------- 候補ウィンドウ（L-E）

pub fn side_from_pb(value: i32) -> Result<PanelSide> {
    match enum_value::<pb::PanelSide>(value, "PanelSide")? {
        pb::PanelSide::Unspecified => Err(ConvertError::OutOfRange("PanelSide")),
        pb::PanelSide::Below => Ok(PanelSide::Below),
        pb::PanelSide::Above => Ok(PanelSide::Above),
    }
}

pub fn side_to_pb(side: PanelSide) -> pb::PanelSide {
    match side {
        PanelSide::Below => pb::PanelSide::Below,
        PanelSide::Above => pb::PanelSide::Above,
    }
}

/// UNSPECIFIED は「今回は新しく取っていない」（`None`）
pub fn source_from_pb(value: i32) -> Result<Option<AnchorSource>> {
    Ok(
        match enum_value::<pb::AnchorSource>(value, "AnchorSource")? {
            pb::AnchorSource::Unspecified => None,
            pb::AnchorSource::Client => Some(AnchorSource::Client),
            pb::AnchorSource::Accessibility => Some(AnchorSource::Accessibility),
            pb::AnchorSource::LastKnown => Some(AnchorSource::LastKnown),
            pb::AnchorSource::MouseScreen => Some(AnchorSource::MouseScreen),
        },
    )
}

pub fn source_to_pb(source: Option<AnchorSource>) -> pb::AnchorSource {
    match source {
        None => pb::AnchorSource::Unspecified,
        Some(AnchorSource::Client) => pb::AnchorSource::Client,
        Some(AnchorSource::Accessibility) => pb::AnchorSource::Accessibility,
        Some(AnchorSource::LastKnown) => pb::AnchorSource::LastKnown,
        Some(AnchorSource::MouseScreen) => pb::AnchorSource::MouseScreen,
    }
}

pub fn rect_from_pb(rect: Option<pb::Rect>) -> Option<Rect> {
    rect.map(|r| Rect::new(r.x, r.y, r.width, r.height))
}

pub fn rect_to_pb(rect: &Rect) -> pb::Rect {
    pb::Rect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

// ---------------------------------------------------------------- 設定

fn issue_to_pb(issue: &SettingsIssue) -> pb::SettingsIssue {
    let kind = match issue.kind {
        SettingsIssueKind::Malformed => pb::SettingsIssueKind::Malformed,
        SettingsIssueKind::UnknownKey => pb::SettingsIssueKind::UnknownKey,
        SettingsIssueKind::InvalidValue => pb::SettingsIssueKind::InvalidValue,
    };
    pb::SettingsIssue {
        line: issue.line,
        key: issue.key.clone(),
        kind: kind.into(),
    }
}

fn snapshot_to_pb(snapshot: &SettingsSnapshot) -> pb::SettingsSnapshot {
    pb::SettingsSnapshot {
        developer_lm_status: snapshot.developer_lm_status,
        input_method: method_to_pb(snapshot.input_method).into(),
        keymap_preset: preset_to_pb(snapshot.keymap_preset).into(),
        keymap_overrides: snapshot
            .keymap_overrides
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        paging_wrap: snapshot.paging_wrap,
        page_size: snapshot.page_size,
        engine_lm: snapshot.engine_lm,
        engine_idle_unload_secs: snapshot.engine_idle_unload_secs,
        engine_lm_deadline_ms: snapshot.engine_lm_deadline_ms,
        engine_convert_wait_ms: snapshot.engine_convert_wait_ms,
        emoji_shortcode: snapshot.emoji_shortcode,
        emoji_shortcode_exclude: snapshot.emoji_shortcode_exclude.clone(),
        emoji_small_letters: snapshot.emoji_small_letters,
        postal_japanese: snapshot.postal_japanese,
        postal_english: snapshot.postal_english,
        postal_te_prefix: snapshot.postal_te_prefix,
        unicode_notation: snapshot.unicode_notation,
        date_weekday_dates: snapshot.date_weekday_dates,
        candidate_hide_when_confident: snapshot.candidate_hide_when_confident,
        clause_min_chars: snapshot.clause_min_chars,
    }
}

/// `SettingsSnapshot` の proto を型にする（テストと、設定を読む側）。
pub fn snapshot_from_pb(snapshot: pb::SettingsSnapshot) -> Result<SettingsSnapshot> {
    Ok(SettingsSnapshot {
        developer_lm_status: snapshot.developer_lm_status,
        input_method: method_from_pb(snapshot.input_method, "SettingsSnapshot.input_method")?,
        keymap_preset: preset_from_pb(snapshot.keymap_preset, "SettingsSnapshot.keymap_preset")?,
        keymap_overrides: snapshot
            .keymap_overrides
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        paging_wrap: snapshot.paging_wrap,
        page_size: snapshot.page_size,
        engine_lm: snapshot.engine_lm,
        engine_idle_unload_secs: snapshot.engine_idle_unload_secs,
        engine_lm_deadline_ms: snapshot.engine_lm_deadline_ms,
        engine_convert_wait_ms: snapshot.engine_convert_wait_ms,
        emoji_shortcode: snapshot.emoji_shortcode,
        emoji_shortcode_exclude: snapshot.emoji_shortcode_exclude,
        emoji_small_letters: snapshot.emoji_small_letters,
        postal_japanese: snapshot.postal_japanese,
        postal_english: snapshot.postal_english,
        postal_te_prefix: snapshot.postal_te_prefix,
        unicode_notation: snapshot.unicode_notation,
        date_weekday_dates: snapshot.date_weekday_dates,
        candidate_hide_when_confident: snapshot.candidate_hide_when_confident,
        clause_min_chars: snapshot.clause_min_chars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_event() -> Vec<ShellEvent> {
        let keys = [
            KeyKind::Character { text: "a".into() },
            KeyKind::Jis { code: 47 },
            KeyKind::Space,
            KeyKind::Enter,
            KeyKind::Escape,
            KeyKind::Backspace,
            KeyKind::Left,
            KeyKind::Right,
            KeyKind::Up,
            KeyKind::Down,
            KeyKind::Function { number: 20 },
            KeyKind::Other,
        ];
        let mut events: Vec<ShellEvent> = keys
            .into_iter()
            .map(|key| {
                ShellEvent::Key(KeyInput {
                    key,
                    modifiers: Modifiers {
                        shift: true,
                        command: true,
                        ..Modifiers::NONE
                    },
                    is_repeat: true,
                    secure: false,
                    layout_text: None,
                })
            })
            .collect();
        events.extend([
            ShellEvent::Key(KeyInput {
                key: KeyKind::Space,
                modifiers: Modifiers::NONE,
                is_repeat: false,
                secure: true,
                layout_text: None,
            }),
            ShellEvent::Key(KeyInput {
                layout_text: Some("a".into()),
                ..KeyInput::new(KeyKind::Jis { code: 22 })
            }),
            ShellEvent::Key(KeyInput::new(KeyKind::Eisu)),
            ShellEvent::Key(KeyInput::new(KeyKind::Kana)),
            ShellEvent::Activate {
                bundle_id: "com.apple.TextEdit".into(),
                secure: true,
            },
            ShellEvent::InputModeChanged {
                mode_id: "io.github.ikari-zen.inputmethod.ikari.English".into(),
                secure: false,
            },
            ShellEvent::CommitComposition,
            ShellEvent::PanelShown {
                side: PanelSide::Above,
                source: Some(AnchorSource::Accessibility),
            },
            ShellEvent::PanelShown {
                side: PanelSide::Below,
                source: None,
            },
            ShellEvent::PanelHidden,
            ShellEvent::BridgeClosed { conn: ConnId(3) },
            ShellEvent::Deactivate,
            ShellEvent::SettingsFileChanged,
            ShellEvent::Timer { id: TimerId(7) },
            ShellEvent::BridgeFrame {
                conn: ConnId(u64::MAX),
                frame: vec![0, 1, 255],
            },
        ]);
        for command in every_menu_command() {
            events.push(ShellEvent::MenuCommand { command });
        }
        events
    }

    fn every_menu_command() -> Vec<MenuCommand> {
        vec![
            MenuCommand::SelectKeymapPreset {
                preset: KeymapPreset::MsIme,
            },
            MenuCommand::SelectInputMethod {
                method: InputMethodKind::Kana,
            },
            MenuCommand::SetLm { enabled: true },
            MenuCommand::SetProfileDomain {
                id: "it".into(),
                included: true,
            },
            MenuCommand::OpenSettings { tab: None },
            MenuCommand::OpenSettings {
                tab: Some(SettingsTab::UserDictionary),
            },
        ]
    }

    fn every_action() -> Vec<ShellAction> {
        let panel = CandidatePanelModel {
            developer_note: String::new(),
            candidates: vec![
                CandidateItem {
                    surface: "品詞".into(),
                    annotation: "ひんし".into(),
                    kind: CandidateKind::Dictionary,
                    label: String::new(),
                },
                CandidateItem {
                    surface: "👀".into(),
                    annotation: String::new(),
                    kind: CandidateKind::Rewriter,
                    label: ":eyes:".into(),
                },
                CandidateItem {
                    surface: "x".into(),
                    annotation: String::new(),
                    kind: CandidateKind::Unspecified,
                    label: String::new(),
                },
            ],
            selected: 1,
            annotation: "注".into(),
            reserved_rows: 9,
        };
        vec![
            ShellAction::PassThrough,
            ShellAction::Consume,
            ShellAction::SetMarkedText(MarkedText::plain("ひんし")),
            ShellAction::SetMarkedText(MarkedText {
                text: "品詞を".into(),
                highlighted_length: Utf16Len(0),
                segments: vec![Utf16Len(2), Utf16Len(1)],
                focused: Some(0),
                replacement_range: Some(Utf16Range {
                    location: Utf16Len(3),
                    length: Utf16Len(2),
                }),
                underlined: false,
                delete_target: true,
            }),
            ShellAction::Commit {
                text: "品詞".into(),
            },
            ShellAction::ShowCandidates(panel.clone()),
            ShellAction::MoveSelection(panel),
            ShellAction::HideCandidates,
            ShellAction::Restore {
                text: "ひんし".into(),
                replacing: Utf16Range {
                    location: Utf16Len(0),
                    length: Utf16Len(2),
                },
            },
            ShellAction::DeleteText {
                range: Utf16Range {
                    location: Utf16Len(5),
                    length: Utf16Len(3),
                },
            },
            ShellAction::ResolveStrayMarkedText,
            ShellAction::Prefetch {
                reading: "ひん".into(),
            },
            ShellAction::ShowMenu(MenuModel {
                items: vec![
                    MenuItem {
                        kind: MenuItemKind::Radio {
                            group: MenuRadioGroup::KeymapPreset,
                        },
                        title: "macOS".into(),
                        command: Some(MenuCommand::SelectKeymapPreset {
                            preset: KeymapPreset::Macos,
                        }),
                        checked: true,
                        enabled: true,
                    },
                    MenuItem {
                        kind: MenuItemKind::Separator,
                        title: String::new(),
                        command: None,
                        checked: false,
                        enabled: true,
                    },
                    MenuItem {
                        kind: MenuItemKind::Toggle,
                        title: "LM".into(),
                        command: Some(MenuCommand::SetLm { enabled: false }),
                        checked: false,
                        enabled: false,
                    },
                    MenuItem {
                        kind: MenuItemKind::Action,
                        title: "設定".into(),
                        command: Some(MenuCommand::OpenSettings { tab: None }),
                        checked: false,
                        enabled: true,
                    },
                ],
            }),
            ShellAction::ScheduleTimer {
                id: TimerId(3),
                after_ms: 250,
            },
            ShellAction::CancelTimer { id: TimerId(3) },
            ShellAction::BridgeReply {
                conn: ConnId(9),
                frame: vec![1, 2, 3],
            },
            ShellAction::OpenSettings {
                tab: Some(SettingsTab::About),
            },
            ShellAction::SelectMode {
                mode_id: "io.github.ikari-zen.inputmethod.ikari.Japanese".into(),
            },
            ShellAction::BridgeClose { conn: ConnId(9) },
            ShellAction::ShowLearningQuarantineNotice,
        ]
    }

    #[test]
    fn every_event_round_trips() {
        for event in every_event() {
            let bytes = encode_event(&event);
            assert_eq!(decode_event(&bytes).unwrap(), event, "{event:?}");
        }
    }

    #[test]
    fn every_action_round_trips() {
        let actions = every_action();
        let bytes = encode_actions(&actions);
        assert_eq!(decode_actions(&bytes).unwrap(), actions);
        assert_eq!(decode_actions(&encode_actions(&[])).unwrap(), Vec::new());
    }

    #[test]
    fn values_the_types_cannot_hold_are_rejected() {
        let encode = |event: pb::ShellEvent| prost::Message::encode_to_vec(&event);
        assert_eq!(
            decode_event(&encode(pb::ShellEvent { event: None })),
            Err(ConvertError::Missing("ShellEvent.event"))
        );
        let key = |kind: pb::KeyKind, jis: u32, function: u32| {
            encode(pb::ShellEvent {
                event: Some(pb::shell_event::Event::Key(pb::KeyInput {
                    kind: kind.into(),
                    jis,
                    function,
                    ..Default::default()
                })),
            })
        };
        assert_eq!(
            decode_event(&key(pb::KeyKind::Unspecified, 0, 0)),
            Err(ConvertError::OutOfRange("KeyInput.kind"))
        );
        assert_eq!(
            decode_event(&key(pb::KeyKind::Jis, 256, 0)),
            Err(ConvertError::OutOfRange("KeyInput.jis"))
        );
        assert_eq!(
            decode_event(&key(pb::KeyKind::Function, 0, 1000)),
            Err(ConvertError::OutOfRange("KeyInput.function"))
        );
        let unknown_kind = encode(pb::ShellEvent {
            event: Some(pb::shell_event::Event::Key(pb::KeyInput {
                kind: 999,
                ..Default::default()
            })),
        });
        assert_eq!(
            decode_event(&unknown_kind),
            Err(ConvertError::OutOfRange("KeyInput.kind"))
        );
        let preset = encode(pb::ShellEvent {
            event: Some(pb::shell_event::Event::MenuCommand(pb::MenuCommand {
                command: Some(pb::menu_command::Command::SelectKeymapPreset(0)),
            })),
        });
        assert_eq!(
            decode_event(&preset),
            Err(ConvertError::OutOfRange("MenuCommand.select_keymap_preset"))
        );
        assert_eq!(decode_event(&[0xff, 0xff, 0xff]), Err(ConvertError::Decode));
        assert_eq!(
            decode_selection(&[]),
            Err(ConvertError::Missing("SelectedText.range"))
        );
    }

    #[test]
    fn settings_snapshot_round_trips() {
        let validation = ikari_shell_core::settings::validate("keymap.preset\tatok\n").unwrap();
        let bytes = encode_settings_validation(&validation);
        let back = <pb::SettingsValidation as prost::Message>::decode(bytes.as_slice()).unwrap();
        assert_eq!(
            snapshot_from_pb(back.snapshot.unwrap()).unwrap(),
            validation.snapshot
        );
        assert_eq!(validation.snapshot.keymap_preset, KeymapPreset::Atok);
    }
}
