//! 設定画面が engine-host に話すパイプの契約（ADR 0040 §2.5、§5 の W4-s の契約。W4-s1）。**宣言だけ**。実装は W4-s2。
//!
//! - 相手: 設定画面は、TIP と同じ engine-host に話す（TIP の DLL はアプリごとにあり、画面が話す相手を 1 つに決められないため）。
//!   ユーザー辞書と学習の管理の要求（ADR 0005 §8.3）は、engine-host の中でエンジンに渡す。
//! - 名前: エンジンのパイプ（`ikari_pipe::pipe_name`）とは別のパイプにする。TIP の通り道に設定画面の要求を混ぜず、
//!   アクセスの制御（ユーザーの SID だけ）を別々に持てるようにするため。名前の組み立ては、エンジンのパイプと同じ規則
//!   （`<接頭辞><SID>-s<セッション番号>`。SID の検査も同じ）で、W4-s2 で `pipe` の関数に足す。
//! - 枠: `ikari_pipe::Frame`（`Request` と `Reply`。通し番号は同じ規則）。本体は `ikari.settings.v1.AppRequest` /
//!   `AppReply`（`proto/ikari/settings/v1/settings.proto`。macOS の `ikari_settings_app` と同じ）のバイト列。
//!   つまり macOS で shell-core が `ikari_settings_app` で答えているもの（既定値と範囲、検証、変更の組み立て）を、
//!   Windows では engine-host が同じ shell-core の関数で答える。画面が出す設定の一覧は
//!   `ikari_shell_core::settings::all_specs`（画面が直接リンクするか、一覧を要求で運ぶかは、画面の方式（S11）で決める）。
//! - TIP が設定の変更を知る方法（ファイルの変更を見るか、engine-host から知らせるか）は W4-s2 で決める。
//!
//! 判断の置き場所: Rust。

/// 設定画面のパイプの名前の先頭（`\\.\pipe\` から）。後ろに `<SID>-s<セッション番号>` を付ける（W4-s2）。
/// エンジンのパイプ（`\\.\pipe\ikari-ime-engine-v1-`）とは別。
pub const SETTINGS_PIPE_NAME_PREFIX: &str = r"\\.\pipe\ikari-ime-settings-v1-";

/// 枠の本体のプロトコル（proto のパッケージ名。`AppRequest` / `AppReply`）。版を上げるときは名前を変え、
/// 古い画面と engine-host の組み合わせは、接続の最初に断る（W4-s2）。
pub const SETTINGS_PIPE_PROTOCOL: &str = "ikari.settings.v1";

/// 設定画面のパイプで使う枠の種類。`Post`（返事のない通知）は、画面からは送らない（画面の要求はすべて返事を待つ）。
pub const SETTINGS_PIPE_FRAME_KINDS: [ikari_pipe::FrameKind; 2] =
    [ikari_pipe::FrameKind::Request, ikari_pipe::FrameKind::Reply];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_pipe_is_a_different_pipe_from_the_engine_pipe() {
        let engine = ikari_pipe::pipe_name("S-1-5-21-1-2-3-500", 1).unwrap();
        assert!(!engine.starts_with(SETTINGS_PIPE_NAME_PREFIX));
        assert!(SETTINGS_PIPE_NAME_PREFIX.starts_with(r"\\.\pipe\"));
        assert_eq!(SETTINGS_PIPE_PROTOCOL, "ikari.settings.v1");
    }
}
