//! 結合テストの共通部品（`serve.rs`: メモリ上の流れ、`pipe_convert.rs`: 名前付きパイプ）。

#![allow(dead_code)]

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use ikari_core::user::UserDir;
use ikari_engine::{Engine, EngineConfig};
use ikari_pipe::Client;
use ikari_proto::SCHEMA_VERSION;
use ikari_proto::v1 as pb;
use ikari_proto::v1::{Envelope, envelope};
use prost::Message;

/// 辞書の「この」の候補（`testdata/sudachi-mini/lex.csv`）
pub const READING: &str = "この";
pub const CANDIDATES: [&str; 3] = ["児野", "古野", "好乃"];
pub const SESSION: u64 = 1;

pub fn long() -> Duration {
    ikari_testtime::ms(10_000)
}

/// fixture の辞書をビルドした `dict/`。テスト全体で 1 回だけ作る。
pub fn fixture_dict_dir() -> &'static Path {
    static DICT: OnceLock<tempfile::TempDir> = OnceLock::new();
    DICT.get_or_init(|| {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/sudachi-mini");
        let out = tempfile::tempdir().unwrap();
        ikari_dict_builder::build(&ikari_dict_builder::BuildConfig {
            lex: vec![src.join("lex.csv")],
            matrix: src.join("matrix.def"),
            out_dir: out.path().to_owned(),
            unk_pos: "名詞,普通名詞,一般,*,*,*".to_owned(),
            unk_cost: 10_000,
            label: "engine-host-test".to_owned(),
        })
        .expect("build the fixture dictionary");
        out
    })
    .path()
}

/// fixture の辞書を `dict/` に置いたデータのディレクトリ。
pub fn data_dir_with_dict() -> tempfile::TempDir {
    let data = tempfile::tempdir().unwrap();
    let dict = UserDir::new(data.path()).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
    }
    data
}

/// 辞書を置いたデータのディレクトリで、LM なしのエンジンを開く。
pub fn open_engine() -> (Engine, tempfile::TempDir) {
    let data = data_dir_with_dict();
    let engine = Engine::open(&EngineConfig {
        data_dir: data.path().to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    })
    .expect("open the engine");
    (engine, data)
}

pub fn envelope_bytes(request_id: u64, body: envelope::Body) -> Vec<u8> {
    Envelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        body: Some(body),
    }
    .encode_to_vec()
}

pub fn call(client: &Client, request_id: u64, body: envelope::Body) -> envelope::Body {
    let reply = client
        .request(&envelope_bytes(request_id, body), long())
        .expect("reply");
    let reply = Envelope::decode(reply.as_slice()).expect("reply is an Envelope");
    assert_eq!(reply.request_id, request_id, "request_id is echoed");
    reply.body.expect("reply body")
}

pub fn begin_session(client: &Client, session: u64) {
    let reply = call(
        client,
        1,
        envelope::Body::BeginSession(pb::BeginSession {
            session_id: session,
            client_bundle_id: "engine-host-test".into(),
            secure: false,
        }),
    );
    assert!(matches!(reply, envelope::Body::Ack(_)), "{reply:?}");
}

pub fn convert(client: &Client, session: u64) -> pb::ConvertReply {
    let reply = call(
        client,
        2,
        envelope::Body::Convert(pb::Convert {
            session_id: session,
            reading: READING.into(),
            context: None,
            context_rev: 0,
            page_size: 0,
        }),
    );
    match reply {
        envelope::Body::ConvertReply(r) => r,
        other => panic!("unexpected: {other:?}"),
    }
}

pub fn end_session(client: &Client, session: u64) {
    let reply = call(
        client,
        3,
        envelope::Body::EndSession(pb::EndSession {
            session_id: session,
        }),
    );
    assert!(matches!(reply, envelope::Body::Ack(_)), "{reply:?}");
}

/// 変換の 1 ページ目を引く（返事の本体をそのまま返す。別の接続からなら `UNKNOWN_CONVERSION`）
pub fn page(client: &Client, conversion_id: u64) -> envelope::Body {
    call(
        client,
        4,
        envelope::Body::Page(pb::PageRequest {
            conversion_id,
            index: 0,
        }),
    )
}

pub fn error_code(body: &envelope::Body) -> pb::ErrorCode {
    match body {
        envelope::Body::Error(e) => pb::ErrorCode::try_from(e.code).expect("known error code"),
        other => panic!("expected an error, got {other:?}"),
    }
}

/// 2 本の接続が同じ `session_id` を使っても混ざらないこと（ADR 0064 D1・D2、§4.4 の 2）。
/// `serve.rs`（メモリ上の流れ）と `pipe_convert.rs`（名前付きパイプ）が同じ中身を流す
pub fn assert_connections_with_the_same_session_id_stay_apart(a: &Client, b: &Client) {
    begin_session(a, SESSION);
    begin_session(b, SESSION);
    // A の変換は、B からは引けない（変換の表は作った接続からだけ受ける）
    let ra = convert(a, SESSION);
    assert_eq!(
        error_code(&page(b, ra.conversion_id)),
        pb::ErrorCode::UnknownConversion
    );
    assert!(matches!(
        page(a, ra.conversion_id),
        envelope::Body::PageReply(_)
    ));
    // A が終わっても、B の同じ番号のセッションは動き続ける
    end_session(a, SESSION);
    assert!(!convert(b, SESSION).candidates.is_empty());
    let reply = call(
        a,
        5,
        envelope::Body::Convert(pb::Convert {
            session_id: SESSION,
            reading: READING.into(),
            context: None,
            context_rev: 0,
            page_size: 0,
        }),
    );
    assert_eq!(error_code(&reply), pb::ErrorCode::UnknownSession);
}
