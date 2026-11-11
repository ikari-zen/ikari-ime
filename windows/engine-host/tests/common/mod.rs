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

/// 辞書を置いたデータのディレクトリで、LM なしのエンジンを開く。
pub fn open_engine() -> (Engine, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let dict = UserDir::new(data.path()).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
    }
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
