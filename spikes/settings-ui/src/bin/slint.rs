//! U-c: Rust の GUI crate（Slint。Fluent スタイル）で設定画面を出す。捨てるスパイク（#322）。
//! 設定の変更は標準出力に `CHANGE key<TAB>value`、計測は `RESULT ...`。

#[path = "../model.rs"]
mod model;

#[cfg(windows)]
#[path = "../args.rs"]
mod args;
#[cfg(windows)]
#[path = "../metrics.rs"]
mod metrics;

#[cfg(not(windows))]
fn main() {
    eprintln!("settings-slint は Windows でだけ動く（スパイク）");
    let _ = model::fixed_items();
}

#[cfg(windows)]
mod ui {
    // 画面の定義。文言は model.rs と同じ（固定）。型ごとの部品:
    // 真偽=Switch、整数と範囲=SpinBox、列挙=ComboBox、文字列=LineEdit、
    // キー割り当て=FocusScope で押したキーを受ける枠、Bundle ID の一覧=ListView + 追加欄
    slint::slint! {
        import { Switch, SpinBox, ComboBox, LineEdit, Button, ListView, ScrollView, Palette } from "std-widgets.slint";

        component Row inherits Rectangle {
            in property <string> label;
            in property <string> description;
            min-height: 52px;
            background: Palette.alternate-background;
            border-radius: 6px;
            HorizontalLayout {
                padding: 12px;
                spacing: 16px;
                VerticalLayout {
                    horizontal-stretch: 1;
                    alignment: center;
                    Text { text: root.label; font-weight: 500; color: Palette.foreground; }
                    Text { text: root.description; font-size: 12px; color: Palette.foreground.with-alpha(0.65); }
                }
                @children
            }
        }

        export component SettingsWindow inherits Window {
            title: "ikari-ime 設定（Slint）";
            preferred-width: 760px;
            preferred-height: 760px;
            background: Palette.background;

            in-out property <bool> wrap;
            in-out property <int> page-size: 3;
            in-out property <string> method: "romaji";
            in-out property <string> note;
            in-out property <string> chord: "none";
            in property <int> page-min: 1;
            in property <int> page-max: 9;
            in property <[string]> methods;
            in property <[string]> ids;
            in-out property <string> new-id;

            callback changed(string, string);
            callback add-id(string);
            callback remove-id(int);

            ScrollView {
                VerticalLayout {
                    padding: 20px;
                    spacing: 6px;
                    alignment: start;
                    Text { text: "ikari-ime 設定（スパイク: Slint）"; font-size: 20px; font-weight: 600; color: Palette.foreground; }

                    Row {
                        label: "候補をぐるっと回す";
                        description: "最後の候補の次に最初へ戻る";
                        Switch {
                            checked <=> root.wrap;
                            accessible-label: "候補をぐるっと回す";
                            toggled => { root.changed("paging.wrap", self.checked ? "true" : "false"); }
                        }
                    }
                    Row {
                        label: "1 ページの候補の数";
                        description: "1 から 9";
                        SpinBox {
                            minimum: root.page-min;
                            maximum: root.page-max;
                            value <=> root.page-size;
                            accessible-label: "1 ページの候補の数";
                            edited(v) => { root.changed("paging.size", v); }
                        }
                    }
                    Row {
                        label: "入力方式";
                        description: "ローマ字、またはかな";
                        ComboBox {
                            model: root.methods;
                            current-value <=> root.method;
                            accessible-label: "入力方式";
                            selected(v) => { root.changed("input.method", v); }
                        }
                    }
                    Row {
                        label: "メモ（日本語の入力欄の確認用）";
                        description: "ここで日本語の IME が使えるか確かめる";
                        LineEdit {
                            width: 220px;
                            text <=> root.note;
                            placeholder-text: "ここに日本語を入力";
                            accessible-label: "メモ";
                            edited(t) => { root.changed("spike.note", t); }
                        }
                    }
                    Row {
                        label: "キー割り当て: 変換";
                        description: "押したキーが入る。Backspace で none（外す）";
                        chord-box := Rectangle {
                            width: 220px;
                            height: 32px;
                            border-radius: 4px;
                            border-width: 1px;
                            border-color: chord-focus.has-focus ? Palette.accent-background : Palette.border;
                            accessible-role: text-input;
                            accessible-label: "キー割り当て: 変換";
                            accessible-value: root.chord;
                            Text { text: root.chord; color: Palette.foreground; x: 8px; vertical-alignment: center; }
                            chord-focus := FocusScope {
                                key-pressed(e) => {
                                    if (e.text == Key.Control || e.text == Key.Shift || e.text == Key.Alt || e.text == Key.Meta) {
                                        return accept;
                                    }
                                    if (e.text == Key.Tab || e.text == Key.Backtab) {
                                        return reject;
                                    }
                                    if (e.text == Key.Backspace && !e.modifiers.control && !e.modifiers.alt && !e.modifiers.shift) {
                                        root.chord = "none";
                                    } else {
                                        root.chord = (e.modifiers.control ? "ctrl+" : "")
                                            + (e.modifiers.alt ? "alt+" : "")
                                            + (e.modifiers.shift ? "shift+" : "")
                                            + (e.modifiers.meta ? "win+" : "")
                                            + e.text.to-lowercase();
                                    }
                                    root.changed("keymap.convert", root.chord);
                                    return accept;
                                }
                            }
                            TouchArea { clicked => { chord-focus.focus(); } }
                        }
                    }
                    Row {
                        label: "絵文字のショートコードを使わないアプリ";
                        description: "Bundle ID の一覧（Windows での識別子は未決）";
                        min-height: 190px;
                        VerticalLayout {
                            width: 260px;
                            spacing: 4px;
                            ListView {
                                height: 100px;
                                for id[i] in root.ids : Rectangle {
                                    height: 30px;
                                    HorizontalLayout {
                                        Text { text: id; vertical-alignment: center; horizontal-stretch: 1; color: Palette.foreground; }
                                        Button { text: "削除"; accessible-label: id + " を削除"; clicked => { root.remove-id(i); } }
                                    }
                                }
                            }
                            HorizontalLayout {
                                spacing: 6px;
                                LineEdit {
                                    text <=> root.new-id;
                                    placeholder-text: "追加する ID";
                                    accessible-label: "追加する ID";
                                }
                                Button {
                                    text: "追加";
                                    clicked => { root.add-id(root.new-id); root.new-id = ""; }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    use slint::{ComponentHandle, Model, SharedString, VecModel};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    let t_main = metrics::qpc();
    let args = args::Args::parse();
    let items = model::fixed_items();
    let find = |key: &str| items.iter().find(|i| i.key == key).expect("fixed item");

    let win = match ui::SettingsWindow::new() {
        Ok(w) => w,
        Err(e) => {
            println!("window build failed: {e}");
            return;
        }
    };
    println!(
        "RESULT B window_created={:.1}ms (main -> component built)",
        metrics::qpc_ms(t_main, metrics::qpc())
    );

    // 固定のデータから初期値を入れる
    let wrap = find("paging.wrap");
    win.set_wrap(wrap.initial == "true");
    let size = find("paging.size");
    if let model::Kind::Int { min, max } = size.kind {
        win.set_page_min(min as i32);
        win.set_page_max(max as i32);
    }
    win.set_page_size(size.initial.parse().unwrap_or(1));
    let method = find("input.method");
    if let model::Kind::Enum { choices } = &method.kind {
        let m: Vec<SharedString> = choices.iter().map(|c| (*c).into()).collect();
        win.set_methods(Rc::new(VecModel::from(m)).into());
    }
    win.set_method(method.initial.into());
    win.set_chord(find("keymap.convert").initial.into());
    let ids: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(
        find("emoji.shortcode_exclude")
            .initial
            .split(',')
            .filter(|s| !s.is_empty())
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ));
    win.set_ids(ids.clone().into());

    // 変更は標準出力へ
    win.on_changed(|key, value| model::report_change(&key, &value));
    let join = |ids: &VecModel<SharedString>| {
        ids.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    {
        let ids = ids.clone();
        win.on_add_id(move |id| {
            let id = id.trim();
            if !id.is_empty() {
                ids.push(id.into());
                model::report_change("emoji.shortcode_exclude", &join(&ids));
            }
        });
    }
    {
        let ids = ids.clone();
        win.on_remove_id(move |i| {
            if i >= 0 && (i as usize) < ids.row_count() {
                ids.remove(i as usize);
                model::report_change("emoji.shortcode_exclude", &join(&ids));
            }
        });
    }
    win.window().on_close_requested(|| {
        metrics::print_memory("B at-close");
        slint::CloseRequestResponse::HideWindow
    });

    // 最初の描画の直後に記録する（レンダラが通知に対応しないときは、イベントループの開始時で代用する）
    let reported = Rc::new(Cell::new(false));
    let settle = Duration::from_millis(args.settle_ms);
    let exit_after = args.exit_after_settle;
    let on_first_paint = {
        let reported = reported.clone();
        move |source: &str| {
            if reported.replace(true) {
                return;
            }
            metrics::report_first_paint(&format!("B({source})"), t_main);
            slint::Timer::single_shot(settle, move || {
                metrics::print_memory("B settled");
                if exit_after {
                    let _ = slint::quit_event_loop();
                }
            });
        }
    };
    let notifier_cb = on_first_paint.clone();
    let installed = win
        .window()
        .set_rendering_notifier(move |state, _| {
            if matches!(state, slint::RenderingState::AfterRendering) {
                notifier_cb("rendered");
            }
        })
        .is_ok();
    if !installed {
        println!("B: rendering notifier unavailable; using event loop start instead");
        slint::Timer::single_shot(Duration::from_millis(0), move || {
            on_first_paint("loop-start")
        });
    }

    if let Err(e) = win.run() {
        println!("run failed: {e}");
    }
}
