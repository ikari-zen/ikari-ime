//! エンジンの側の、檻役への口（macOS。[`CageClient`] の libxpc の実装。ADR 0077 §3.2）。
//!
//! - 殻が仲立ちした匿名の口（`xpc_endpoint`）を持つ。つなぐたびに、相手の署名要件を付ける（合わない相手の返事は
//!   libxpc が落とす。最初の伝言は fd を付けない `hello`）。
//! - 子 1 つにつき接続 1 本（檻役は接続ごとに子を 1 つまで持ち、接続が無効になったら止めて回収する）。`launch` の
//!   前に、その接続でも `hello` を送り、返事を確かめる（[`check_hello`]）。確かめるまで fd を送らない。

use std::ffi::{CString, c_void};
use std::io;
use std::os::fd::AsFd;
use std::sync::mpsc;
use std::time::Duration;

use super::xpc::{self, Connection, Kind, Obj};
use super::{
    CAGE_PROTOCOL_VERSION, CageChild, CageClient, CageLaunch, CageReply, HELLO_REPLY_DEADLINE,
    HelloAnswer, LAUNCH_REPLY_DEADLINE, PeerCheck, STOP_REPLY_DEADLINE, check_hello, wire,
};

/// 檻役への匿名の口（エンジンの側）
#[derive(Debug)]
pub struct XpcCageClient {
    endpoint: Obj,
    requirement: CString,
}

impl XpcCageClient {
    /// 殻から届いた匿名の口を持つ。`requirement` は、相手（檻役）の署名要件
    ///
    /// # Safety
    /// `endpoint` は、呼び出しの間生きている libxpc の値（`xpc_endpoint_t`）であること。こちらは retain を足して
    /// 持つので、呼び出し側の参照はそのまま呼び出し側が返す
    pub unsafe fn from_endpoint(
        endpoint: *mut c_void,
        requirement: &str,
    ) -> io::Result<XpcCageClient> {
        let endpoint = Obj::retain(endpoint)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "null xpc endpoint"))?;
        XpcCageClient::from_obj(endpoint, requirement)
    }

    pub(super) fn from_obj(endpoint: Obj, requirement: &str) -> io::Result<XpcCageClient> {
        if endpoint.kind() != Kind::Endpoint {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not an xpc endpoint",
            ));
        }
        let requirement = CString::new(requirement)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in peer requirement"))?;
        Ok(XpcCageClient {
            endpoint,
            requirement,
        })
    }

    /// 新しい接続を開く（相手の署名要件つき）
    fn open(&self) -> io::Result<Connection> {
        let conn = Connection::to_endpoint(&self.endpoint)?;
        // 返事の誤りは、それぞれの返事のハンドラで受ける。接続のハンドラは何もしない
        conn.on_event(|_| {});
        let required = conn.require_peer(&self.requirement);
        conn.resume();
        required?;
        Ok(conn)
    }

    fn hello_on(conn: &Connection) -> io::Result<HelloAnswer> {
        let message = Obj::dictionary()?;
        message.set_str(wire::KEY_OP, wire::OP_HELLO);
        message.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
        let reply = request(conn, &message, HELLO_REPLY_DEADLINE)?;
        Ok(HelloAnswer {
            // 返事が届いた = libxpc が相手の署名要件を確かめた
            peer: PeerCheck::Verified,
            reply: reply_word(&reply),
            version: reply.get_u64(wire::KEY_VERSION),
        })
    }
}

impl CageClient for XpcCageClient {
    fn hello(&self) -> io::Result<HelloAnswer> {
        let conn = self.open()?;
        XpcCageClient::hello_on(&conn)
    }

    fn launch(&self, launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
        let conn = self.open()?;
        let answer = XpcCageClient::hello_on(&conn)?;
        check_hello(answer).map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;
        let message = Obj::dictionary()?;
        message.set_str(wire::KEY_OP, wire::OP_LAUNCH);
        message.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
        message.set_str(wire::KEY_GPU, launch.gpu.word());
        message.set_fd(wire::KEY_STDIN, launch.stdin.as_fd());
        message.set_fd(wire::KEY_STDOUT, launch.stdout.as_fd());
        message.set_fd(wire::KEY_MODEL, launch.model.as_fd());
        // 伝言が fd の写しを持つ。子の側の端は、ここで閉じる（閉じないと、子が終わっても読み取りが終わらない）
        drop(launch);
        let reply = request(&conn, &message, LAUNCH_REPLY_DEADLINE)?;
        drop(message);
        match (reply_word(&reply), reply.get_u64(wire::KEY_PID)) {
            (Some(CageReply::Ok), Some(pid)) => {
                let pid = u32::try_from(pid).map_err(|_| io::Error::other("cage pid range"))?;
                Ok(Box::new(XpcCageChild {
                    conn: Some(conn),
                    pid,
                }))
            }
            (word, _) => Err(io::Error::other(format!(
                "cage launch: {}",
                word.map_or("bad-reply", CageReply::word)
            ))),
        }
    }
}

/// 檻役が起動した子の持ち手。捨てると接続を取り消す（檻役が子を止めて回収する）
#[derive(Debug)]
struct XpcCageChild {
    conn: Option<Connection>,
    pid: u32,
}

impl CageChild for XpcCageChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn stop(&mut self) -> io::Result<()> {
        // 返事のあとも、来なくても、接続は捨てる（子は 1 接続に 1 つ）
        let Some(conn) = self.conn.take() else {
            return Ok(());
        };
        let message = Obj::dictionary()?;
        message.set_str(wire::KEY_OP, wire::OP_STOP);
        message.set_u64(wire::KEY_VERSION, u64::from(CAGE_PROTOCOL_VERSION));
        let reply = request(&conn, &message, STOP_REPLY_DEADLINE)?;
        match reply_word(&reply) {
            Some(CageReply::Ok) => Ok(()),
            word => Err(io::Error::other(format!(
                "cage stop: {}",
                word.map_or("bad-reply", CageReply::word)
            ))),
        }
    }
}

fn reply_word(reply: &Obj) -> Option<CageReply> {
    reply
        .get_str(wire::KEY_REPLY)
        .as_deref()
        .and_then(CageReply::from_word)
}

/// 送って、返事の辞書を `deadline` まで待つ。相手の確認で落とされた・接続が切れた・締め切りは誤り
pub(super) fn request(conn: &Connection, message: &Obj, deadline: Duration) -> io::Result<Obj> {
    let (tx, rx) = mpsc::sync_channel::<io::Result<Obj>>(1);
    conn.send_with_reply(message, move |reply| {
        let result = match xpc::kind_of(reply) {
            Kind::Dictionary => Obj::retain(reply).ok_or_else(|| io::Error::other("xpc reply")),
            Kind::PeerRequirement => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cage peer requirement",
            )),
            Kind::ConnectionInvalid | Kind::ConnectionInterrupted => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "cage connection",
            )),
            _ => Err(io::Error::other("cage reply")),
        };
        let _ = tx.try_send(result);
    });
    match rx.recv_timeout(deadline) {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "cage reply")),
    }
}
