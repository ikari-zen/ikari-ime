//! macOS の LM の檻の段 2（ADR 0077 形 A）: エンジンと檻役（`ikari-lm-cage`。`IkariLMCage.xpc`）の取り決め。
//!
//! **契約（第 4 弾の契約コミット）**。エンジンの側（`os` の起動の口）と檻役の側（新しい bin）の両方がこのモジュールを
//! 使う。ここにあるのは、型、固定の語、仮置きの値、テストの骨組みだけ。libxpc の呼び出し（`block2`）と、
//! 起動・止める・回収の実装はレーン D-3-mac が足す（同じモジュールの下。ADR 0077 §3.4 の「エンジンの側の libxpc の
//! 呼び出しも同じモジュールに置く」）。
//!
//! 決まっていること（ADR 0077 §3.1、§3.2、§11 の条件）:
//! - 檻役は判断を持たない。起動するのは自分の束の中の `ikari-lm-host` の絶対パスだけで、引数は [`CageGpu`] から
//!   檻役が自分で組む（[`child_args`]）。任意の引数・パス・環境変数を受けない。
//! - 子に付ける fd は 0（標準入力のパイプ）・1（標準出力のパイプ）・3（モデル）だけ（`POSIX_SPAWN_CLOEXEC_DEFAULT`）。
//!   標準エラーは付けない。子の環境変数は固定の空。
//! - 子の数は、1 本の接続につき 1 つまで、檻役の全体でも [`MAX_CHILDREN_TOTAL`] まで。
//! - 檻役は、設定も状態も、自分のコンテナから読まない。
//! - エンジンは、もらった口の先が本物の檻役かを署名要件で確かめるまで、モデルの fd も採点の入力も渡さない。
//!   確かめに失敗したら、その口を捨てて「檻なし」（LM なし）にする。
//! - 取り決めには版（[`CAGE_PROTOCOL_VERSION`]）を持たせ、違えば起動の失敗に数える。
//!
//! 最初のあいさつ（FC-1）: 口の最初の伝言は、fd を付けない `hello`（[`wire::OP_HELLO`]）。エンジンは、`hello` の
//! 返事の送り手が署名要件を通ったときだけ、口を確かめ済み（[`VerifiedCage`]）にする。[`CageSlot`] に入るのは
//! 確かめ済みの口だけで、`launch`（fd つき）は確かめ済みの口からしか送れない。檻役の側も、接続ごとに
//! [`CageSession`] で、`hello` の前の `launch` を断る。
//!
//! 受けた fd の検査（FC-2）: 檻役は起動の前に、受けた fd の種類と開き方を [`check_launch_fds`] で確かめる
//! （モデル = ふつうのファイルで読み取りだけ、標準入力 = パイプの読み口、標準出力 = パイプの書き口）。
//! 違えば `bad-request` で、起動しない。判断はこの表（OS に依存しない）、種類と開き方を取るのは [`probe_fd`]。
//!
//! 判断の置き場所: Rust（いつ起動し、いつ止めるかは今までどおり `scorer`。檻役は実行だけ）。

use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
#[cfg(unix)]
use std::os::fd::BorrowedFd;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::host::{ARG_GPU, ARG_GPU_CPU, ARG_GPU_OFF, ARG_GPU_ON, ARG_MODEL};

// 実装（D-3-mac）: libxpc の呼び出し、檻役の側、エンジンの側。macOS だけ
#[cfg(target_vendor = "apple")]
mod client;
#[cfg(target_vendor = "apple")]
mod service;
#[cfg(target_vendor = "apple")]
mod spawn;
#[cfg(target_vendor = "apple")]
mod xpc;

#[cfg(target_vendor = "apple")]
pub use client::XpcCageClient;
#[cfg(target_vendor = "apple")]
pub use service::{CageCore, CageListener, run_service};

/// 署名要件は、それぞれの束の Info.plist（署名の対象）に build.sh が書く（今のエンジンの `IkariPeerRequirement` と
/// 同じ形）。檻役は bin が読んで [`run_service`] に渡し、エンジンは殻が読んで口と一緒に渡す（ライブラリは
/// CoreFoundation を要らないようにする。エンジンの静的ライブラリのリンクの一覧を増やさない）。
///
/// 檻役の束の Info.plist のキー: 名前の口につないでくる相手（殻、smoke）の署名要件（エンジンの束と同じキー）
pub const INFO_KEY_SHELL_PEER_REQUIREMENT: &str = "IkariPeerRequirement";
/// 檻役の束の Info.plist のキー: 匿名の口につないでくる相手（エンジン）の署名要件
pub const INFO_KEY_ENGINE_PEER_REQUIREMENT: &str = "IkariCageEnginePeerRequirement";
/// エンジンの束の Info.plist のキー: もらった口の先（檻役）の署名要件（FC-1）
pub const INFO_KEY_CAGE_PEER_REQUIREMENT: &str = "IkariCagePeerRequirement";

/// エンジンと檻役の取り決めの版。辞書のキーや語を変えたら上げる
pub const CAGE_PROTOCOL_VERSION: u32 = 1;

/// 檻役の XPC サービスの束の名前（`ikari.app/Contents/XPCServices/` の下。ADR 0077 §3.4）
pub const CAGE_SERVICE_BUNDLE: &str = "IkariLMCage.xpc";
/// 檻役の実行ファイルの名前（Rust の bin）
pub const CAGE_EXE_NAME: &str = "ikari-lm-cage";

/// 子のモデルの fd の番号（ADR 0072 §13.3。親が開いたファイルを付けて `--model /dev/fd/3` で読ませる）
pub const CHILD_MODEL_FD: i32 = 3;
/// 子の `--model` に渡すパス（[`CHILD_MODEL_FD`] と同じ番号）
pub const CHILD_MODEL_PATH: &str = "/dev/fd/3";
/// 子に付ける fd（ほかは `POSIX_SPAWN_CLOEXEC_DEFAULT` で閉じる）
pub const CHILD_FDS: [i32; 3] = [0, 1, CHILD_MODEL_FD];

/// 契約のレーンの仮置き。実装のレーンで計測して PR に書き、ユーザーに確かめる。
/// 檻役の全体で同時に動かしてよい子の数（接続ごとの 1 つとは別の、全体の上限。ADR 0077 §3.2）
pub const MAX_CHILDREN_TOTAL: usize = 2;
/// 契約のレーンの仮置き。実装のレーンで計測して PR に書き、ユーザーに確かめる。
/// 「止める」の返事を待つ上限。過ぎたら、エンジンは接続を捨てる（檻役が子を止める。ADR 0077 §3.3）
pub const STOP_REPLY_DEADLINE: Duration = Duration::from_secs(5);
/// 契約のレーンの仮置き。実装のレーンで計測して PR に書き、ユーザーに確かめる。
/// `hello` の返事を待つ上限。過ぎたら、その口は確かめに失敗したとして捨てる（FC-1）
pub const HELLO_REPLY_DEADLINE: Duration = Duration::from_secs(5);
/// 実装のレーン（D-3-mac）の仮置き。計測して PR に書き、ユーザーに確かめる。
/// `launch` の返事を待つ上限（§13.5 の実測では、頼みから起動まで 1ms 未満）。過ぎたら、その接続を捨てる
pub const LAUNCH_REPLY_DEADLINE: Duration = Duration::from_secs(5);
/// 契約のレーンの仮置き。実装のレーンで計測して PR に書き、ユーザーに確かめる。
/// 子（`ikari-lm-host`）が、親（檻役）が終わったかを見る間隔（固まった子が残らないように。ADR 0077 §3.1）
pub const PARENT_WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// XPC の辞書のキーと、要求の語（エンジン → 檻役）。
///
/// - `hello`: `KEY_OP` と `KEY_VERSION` だけ（fd を付けない）。返事は `KEY_REPLY`（`ok` / `version`）と
///   `KEY_VERSION`。口ごとに最初の伝言で、エンジンはこの返事を確かめるまで、ほかの伝言を送らない。
/// - `launch`: `KEY_OP`、`KEY_VERSION`、`KEY_GPU`、`KEY_STDIN`、`KEY_STDOUT`、`KEY_MODEL`。返事は `KEY_REPLY` と
///   `KEY_PID`。`hello` の返事が `ok` だった接続でだけ受ける。
/// - `stop`: `KEY_OP` と `KEY_VERSION`。返事は `KEY_REPLY`。
pub mod wire {
    /// 取り決めの版（u64）
    pub const KEY_VERSION: &str = "v";
    /// 要求の語（[`OP_HELLO`] / [`OP_LAUNCH`] / [`OP_STOP`]）
    pub const KEY_OP: &str = "op";
    /// GPU の扱いの語（[`super::CageGpu::word`]）
    pub const KEY_GPU: &str = "gpu";
    /// 子の標準入力になるパイプの読み口（fd）
    pub const KEY_STDIN: &str = "stdin";
    /// 子の標準出力になるパイプの書き口（fd）
    pub const KEY_STDOUT: &str = "stdout";
    /// モデルのファイル（fd。読み取りで開いたもの）
    pub const KEY_MODEL: &str = "model";
    /// 返事の語（[`super::CageReply::word`]）
    pub const KEY_REPLY: &str = "reply";
    /// 起動した子の pid（u64。ログと様子の表示にだけ使う。エンジンからは signal を送れない）
    pub const KEY_PID: &str = "pid";

    /// 最初のあいさつ（fd なし。FC-1）
    pub const OP_HELLO: &str = "hello";
    pub const OP_LAUNCH: &str = "launch";
    pub const OP_STOP: &str = "stop";

    /// 名前の口（殻 → 檻役）: 匿名の口をもらう。`KEY_OP` と `KEY_VERSION` だけ。返事は `KEY_REPLY`、
    /// `KEY_VERSION`、`ok` のときだけ `KEY_ENDPOINT`（`xpc_endpoint`）
    pub const OP_ENDPOINT: &str = "endpoint";
    /// 匿名の口（`xpc_endpoint`。[`OP_ENDPOINT`] の返事）
    pub const KEY_ENDPOINT: &str = "endpoint";
}

/// 子の GPU の扱い（`ikari-lm-host` の `--gpu` の値と 1 対 1）。檻役は、これから引数を自分で組む
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CageGpu {
    On,
    Off,
    /// ビルドに関わらず CPU だけ（GPU の子が失敗し続けたあと）
    Cpu,
}

impl CageGpu {
    pub const ALL: [CageGpu; 3] = [CageGpu::On, CageGpu::Off, CageGpu::Cpu];

    /// 取り決めの語（= `--gpu` の値）
    pub fn word(self) -> &'static str {
        match self {
            CageGpu::On => ARG_GPU_ON,
            CageGpu::Off => ARG_GPU_OFF,
            CageGpu::Cpu => ARG_GPU_CPU,
        }
    }

    /// 知らない語は None（檻役は起動しない）
    pub fn from_word(word: &str) -> Option<CageGpu> {
        CageGpu::ALL.into_iter().find(|g| g.word() == word)
    }
}

/// 檻役が子に渡す引数（モデルは常に fd 3）。檻役はこれ以外の引数を付けない
pub fn child_args(gpu: CageGpu) -> Vec<OsString> {
    vec![
        ARG_MODEL.into(),
        CHILD_MODEL_PATH.into(),
        ARG_GPU.into(),
        gpu.word().into(),
    ]
}

/// 檻役の返事の固定の語（檻役 → エンジン）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CageReply {
    /// 起動した / 止めて回収した
    Ok,
    /// 子の数の上限（接続ごと、または全体）に達している
    Busy,
    /// 取り決めの版が違う
    VersionMismatch,
    /// 要求の形が違う（知らない語、fd が足りない、fd の種類か開き方が違う、`hello` の前の `launch`）
    BadRequest,
    /// 子を起動できなかった
    SpawnFailed,
}

impl CageReply {
    pub const ALL: [CageReply; 5] = [
        CageReply::Ok,
        CageReply::Busy,
        CageReply::VersionMismatch,
        CageReply::BadRequest,
        CageReply::SpawnFailed,
    ];

    pub fn word(self) -> &'static str {
        match self {
            CageReply::Ok => "ok",
            CageReply::Busy => "busy",
            CageReply::VersionMismatch => "version",
            CageReply::BadRequest => "bad-request",
            CageReply::SpawnFailed => "spawn-failed",
        }
    }

    pub fn from_word(word: &str) -> Option<CageReply> {
        CageReply::ALL.into_iter().find(|r| r.word() == word)
    }
}

/// 檻役に頼む起動 1 回分（エンジンの側で作る。パイプとモデルはエンジンが開く。ADR 0072 §6）
#[derive(Debug)]
pub struct CageLaunch {
    /// 子の標準入力になる読み口（書き口はエンジンが持つ）
    pub stdin: io::PipeReader,
    /// 子の標準出力になる書き口（読み口はエンジンが持つ）
    pub stdout: io::PipeWriter,
    /// モデル（読み取りで開いたもの。子の fd 3 になる）
    pub model: File,
    pub gpu: CageGpu,
}

/// 檻役が起動した子（エンジンの側の持ち手）。捨てる前に `stop` を呼ぶ（呼ばずに捨てたら、接続を捨てて檻役に止めさせる）
pub trait CageChild: Send + fmt::Debug {
    /// 檻役が知らせた pid（ログと様子だけ。signal は送れない）
    fn pid(&self) -> u32;
    /// 檻役に「止める」を頼み、SIGKILL と回収の返事を待つ（[`STOP_REPLY_DEADLINE`] まで）
    fn stop(&mut self) -> io::Result<()>;
}

/// 檻役への口（エンジンの側。殻から届いた、まだ確かめていないもの）。製品は libxpc の匿名の口（D-3-mac）、
/// テストは差し替える。エンジンは、この口を [`VerifiedCage::handshake`] に通してから使う
pub trait CageClient: Send + Sync + fmt::Debug {
    /// `hello`（fd なし）を送り、返事を [`HELLO_REPLY_DEADLINE`] まで待つ。返事の送り手を署名要件で確かめた
    /// 結果も返す（libxpc の相手の確認で返事が落とされたとき、締め切りを過ぎたときは `Err`）
    fn hello(&self) -> io::Result<HelloAnswer>;
    /// 子を 1 つ起動してもらう。呼ぶのは [`VerifiedCage::launch`] だけ（確かめる前の口に fd を送らない）
    fn launch(&self, launch: CageLaunch) -> io::Result<Box<dyn CageChild>>;
}

/// 返事の送り手の確かめの結果
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerCheck {
    /// 署名要件を満たした（本物の檻役）
    Verified,
    /// 満たさなかった、または確かめられなかった
    Rejected,
}

/// `hello` の返事（[`CageClient::hello`]）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloAnswer {
    pub peer: PeerCheck,
    /// 返事の語（知らない語・無いときは None）
    pub reply: Option<CageReply>,
    /// 檻役の取り決めの版（無いときは None）
    pub version: Option<u64>,
}

/// 口を確かめられなかった理由（ログの固定の語。[`HandshakeError::word`]）
#[derive(Debug)]
pub enum HandshakeError {
    /// 送れない、返事が来ない、相手の確認で返事が落とされた
    Io(io::Error),
    /// 返事の送り手が署名要件を満たさない
    PeerRejected,
    /// 版が違う
    VersionMismatch,
    /// 返事の形が違う（`ok` でも `version` でもない、版が無い）
    BadReply,
}

impl HandshakeError {
    pub fn word(&self) -> &'static str {
        match self {
            HandshakeError::Io(_) => "io",
            HandshakeError::PeerRejected => "peer",
            HandshakeError::VersionMismatch => "version",
            HandshakeError::BadReply => "bad-reply",
        }
    }
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

impl std::error::Error for HandshakeError {}

/// `hello` で確かめた檻役の口。作れるのは [`VerifiedCage::handshake`] だけ
#[derive(Clone, Debug)]
pub struct VerifiedCage {
    client: Arc<dyn CageClient>,
}

impl VerifiedCage {
    /// `hello` を送り、返事の送り手が署名要件を満たし、返事が `ok` で版が同じときだけ確かめ済みにする。
    /// 失敗したら口は捨てる（呼び出し側は [`CageSlot`] に入れない）。`launch` は送らない
    pub fn handshake(client: Arc<dyn CageClient>) -> Result<VerifiedCage, HandshakeError> {
        let answer = client.hello().map_err(HandshakeError::Io)?;
        check_hello(answer)?;
        Ok(VerifiedCage { client })
    }

    /// 子を 1 つ起動してもらう
    pub fn launch(&self, launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
        self.client.launch(launch)
    }
}

/// `hello` の返事を確かめる（送り手が署名要件を満たし、`ok` で版が同じ）。[`VerifiedCage::handshake`] と、
/// libxpc の口が `launch` の接続ごとに送る `hello` の両方が使う
fn check_hello(answer: HelloAnswer) -> Result<(), HandshakeError> {
    if answer.peer != PeerCheck::Verified {
        return Err(HandshakeError::PeerRejected);
    }
    match (answer.reply, answer.version) {
        (Some(CageReply::Ok), Some(v)) if v == u64::from(CAGE_PROTOCOL_VERSION) => Ok(()),
        (Some(CageReply::Ok), Some(_)) | (Some(CageReply::VersionMismatch), _) => {
            Err(HandshakeError::VersionMismatch)
        }
        _ => Err(HandshakeError::BadReply),
    }
}

/// 檻役の側の、接続 1 本の取り決めの状態（OS に依存しない判断。D-3-mac の libxpc のハンドラが使う）。
/// 子の数の上限（`Busy`）は別に数える
#[derive(Debug, Default)]
pub struct CageSession {
    greeted: bool,
}

impl CageSession {
    /// `hello` を受けた。版が同じなら `Ok` を返し、以後この接続で `launch` を受ける
    pub fn on_hello(&mut self, version: Option<u64>) -> CageReply {
        if version == Some(u64::from(CAGE_PROTOCOL_VERSION)) {
            self.greeted = true;
            CageReply::Ok
        } else {
            CageReply::VersionMismatch
        }
    }

    /// `launch` を受けてよいか（fd を見る前に呼ぶ）。`hello` の前は `BadRequest`、版違いは `VersionMismatch`
    pub fn admit_launch(&self, version: Option<u64>) -> Result<(), CageReply> {
        if !self.greeted {
            return Err(CageReply::BadRequest);
        }
        if version != Some(u64::from(CAGE_PROTOCOL_VERSION)) {
            return Err(CageReply::VersionMismatch);
        }
        Ok(())
    }
}

/// 受けた fd の役（FC-2）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdRole {
    /// 子の標準入力（パイプの読み口）
    Stdin,
    /// 子の標準出力（パイプの書き口）
    Stdout,
    /// モデル（ふつうのファイルを読み取りだけで開いたもの）
    Model,
}

/// fd の種類（`fstat` の種類）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdKind {
    Regular,
    Pipe,
    Socket,
    Directory,
    Other,
}

/// fd の開き方（`fcntl(F_GETFL)` の読み書きの別）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FdAccess {
    Read,
    Write,
    ReadWrite,
}

/// fd 1 つの種類と開き方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FdFacts {
    pub kind: FdKind,
    pub access: FdAccess,
}

/// その役に、その種類と開き方の fd を使ってよいか（表は 1 役につき 1 通りだけ）
pub fn fd_allowed(role: FdRole, facts: FdFacts) -> bool {
    let want = match role {
        FdRole::Stdin => FdFacts {
            kind: FdKind::Pipe,
            access: FdAccess::Read,
        },
        FdRole::Stdout => FdFacts {
            kind: FdKind::Pipe,
            access: FdAccess::Write,
        },
        FdRole::Model => FdFacts {
            kind: FdKind::Regular,
            access: FdAccess::Read,
        },
    };
    facts == want
}

/// 檻役が起動の前に、受けた 3 つの fd を確かめる。1 つでも違えば `BadRequest`（起動しない）
pub fn check_launch_fds(stdin: FdFacts, stdout: FdFacts, model: FdFacts) -> Result<(), CageReply> {
    let all = [
        (FdRole::Stdin, stdin),
        (FdRole::Stdout, stdout),
        (FdRole::Model, model),
    ];
    if all.into_iter().all(|(role, facts)| fd_allowed(role, facts)) {
        Ok(())
    } else {
        Err(CageReply::BadRequest)
    }
}

/// fd の種類と開き方を取る（`fstat` と `fcntl(F_GETFL)`。fd の中身は読まない）
#[cfg(unix)]
pub fn probe_fd(fd: BorrowedFd<'_>) -> io::Result<FdFacts> {
    crate::host::os::fd_facts(fd)
}

/// 殻から届いた檻役の口を、確かめたあとで入れておく場所（エンジンに 1 つ）。口は、エンジンが開いたあと
/// （`Hello` のたび、檻役との接続を取り直したあと）に届くので、起動の口（`HostLauncher::Cage`）はこれを引く。
/// 入るのは [`VerifiedCage`] だけ
#[derive(Default)]
pub struct CageSlot {
    client: Mutex<Option<VerifiedCage>>,
}

impl fmt::Debug for CageSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CageSlot")
            .field("attached", &self.current().is_some())
            .finish()
    }
}

impl CageSlot {
    fn guard(&self) -> MutexGuard<'_, Option<VerifiedCage>> {
        self.client.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 確かめた口を入れる（前の口は捨てる）
    pub fn attach(&self, client: VerifiedCage) {
        *self.guard() = Some(client);
    }

    /// 届いた口を確かめ（[`VerifiedCage::handshake`]）、通れば入れる。通らなければ届いた口を捨て、
    /// 今の口はそのまま（殻が偽の口を渡しても、確かめ済みの口は外れない）
    pub fn attach_unverified(&self, client: Arc<dyn CageClient>) -> Result<(), HandshakeError> {
        let verified = VerifiedCage::handshake(client)?;
        self.attach(verified);
        Ok(())
    }

    /// 口を捨てる（接続が切れた）
    pub fn detach(&self) {
        *self.guard() = None;
    }

    /// 今の口（無ければ「檻なし」）
    pub fn current(&self) -> Option<VerifiedCage> {
        self.guard().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_fixed_and_round_trip() {
        for g in CageGpu::ALL {
            assert_eq!(CageGpu::from_word(g.word()), Some(g));
        }
        for r in CageReply::ALL {
            assert_eq!(CageReply::from_word(r.word()), Some(r));
        }
        assert_eq!(CageGpu::from_word("ON"), None);
        assert_eq!(CageReply::from_word(""), None);
    }

    #[test]
    fn child_args_pass_the_model_as_fd_3_only() {
        let args = child_args(CageGpu::Cpu);
        assert_eq!(args, ["--model", "/dev/fd/3", "--gpu", "cpu"]);
        assert_eq!(CHILD_MODEL_PATH, format!("/dev/fd/{CHILD_MODEL_FD}"));
    }

    /// `hello` の返事を決めておき、呼ばれた順を覚える偽の口
    #[derive(Debug)]
    struct Fake {
        answer: Option<HelloAnswer>,
        calls: Mutex<Vec<&'static str>>,
    }

    impl Fake {
        fn new(answer: Option<HelloAnswer>) -> Arc<Fake> {
            Arc::new(Fake {
                answer,
                calls: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CageClient for Fake {
        fn hello(&self) -> io::Result<HelloAnswer> {
            self.calls.lock().unwrap().push(wire::OP_HELLO);
            self.answer
                .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
        }

        fn launch(&self, _launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
            self.calls.lock().unwrap().push(wire::OP_LAUNCH);
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn answer(peer: PeerCheck, reply: Option<CageReply>, version: Option<u64>) -> HelloAnswer {
        HelloAnswer {
            peer,
            reply,
            version,
        }
    }

    const V: Option<u64> = Some(CAGE_PROTOCOL_VERSION as u64);
    const GOOD: HelloAnswer = HelloAnswer {
        peer: PeerCheck::Verified,
        reply: Some(CageReply::Ok),
        version: V,
    };

    #[test]
    fn slot_starts_empty_and_can_be_replaced() {
        let slot = CageSlot::default();
        assert!(slot.current().is_none());
        slot.attach_unverified(Fake::new(Some(GOOD))).unwrap();
        assert!(slot.current().is_some());
        slot.detach();
        assert!(slot.current().is_none());
    }

    #[test]
    fn handshake_accepts_only_a_verified_ok_of_the_same_version() {
        let other = Some(u64::from(CAGE_PROTOCOL_VERSION) + 1);
        let cases = [
            (GOOD, None),
            (
                answer(PeerCheck::Rejected, Some(CageReply::Ok), V),
                Some("peer"),
            ),
            (
                answer(PeerCheck::Verified, Some(CageReply::Ok), other),
                Some("version"),
            ),
            (
                answer(PeerCheck::Verified, Some(CageReply::VersionMismatch), None),
                Some("version"),
            ),
            (
                answer(PeerCheck::Verified, Some(CageReply::Ok), None),
                Some("bad-reply"),
            ),
            (
                answer(PeerCheck::Verified, Some(CageReply::Busy), V),
                Some("bad-reply"),
            ),
            (answer(PeerCheck::Verified, None, V), Some("bad-reply")),
        ];
        for (given, want) in cases {
            let got = VerifiedCage::handshake(Fake::new(Some(given))).err();
            assert_eq!(got.as_ref().map(HandshakeError::word), want, "{given:?}");
        }
        let timeout = VerifiedCage::handshake(Fake::new(None)).err().unwrap();
        assert_eq!(timeout.word(), "io");
    }

    #[test]
    fn engine_sends_nothing_before_the_peer_is_verified() {
        // 確かめに失敗した口には hello しか送らず、確かめ済みの口は外れない
        let slot = CageSlot::default();
        let good = Fake::new(Some(GOOD));
        slot.attach_unverified(good.clone()).unwrap();
        let fake = Fake::new(Some(answer(PeerCheck::Rejected, Some(CageReply::Ok), V)));
        assert!(slot.attach_unverified(fake.clone()).is_err());
        assert_eq!(fake.calls(), [wire::OP_HELLO]);
        assert_eq!(good.calls(), [wire::OP_HELLO]);
        assert!(slot.current().is_some());
    }

    #[test]
    fn the_cage_refuses_launch_before_hello() {
        let mut session = CageSession::default();
        assert_eq!(session.admit_launch(V), Err(CageReply::BadRequest));
        assert_eq!(
            session.on_hello(Some(u64::from(CAGE_PROTOCOL_VERSION) + 1)),
            CageReply::VersionMismatch
        );
        assert_eq!(session.admit_launch(V), Err(CageReply::BadRequest));
        assert_eq!(session.on_hello(V), CageReply::Ok);
        assert_eq!(session.admit_launch(V), Ok(()));
        assert_eq!(session.admit_launch(None), Err(CageReply::VersionMismatch));
    }

    fn facts(kind: FdKind, access: FdAccess) -> FdFacts {
        FdFacts { kind, access }
    }

    #[test]
    fn each_fd_role_accepts_exactly_one_kind_and_access() {
        use FdAccess::*;
        use FdKind::*;
        let kinds = [Regular, Pipe, Socket, Directory, Other];
        let accesses = [Read, Write, ReadWrite];
        let roles = [
            (FdRole::Stdin, facts(Pipe, Read)),
            (FdRole::Stdout, facts(Pipe, Write)),
            (FdRole::Model, facts(Regular, Read)),
        ];
        for (role, want) in roles {
            for kind in kinds {
                for access in accesses {
                    let f = facts(kind, access);
                    assert_eq!(fd_allowed(role, f), f == want, "{role:?} {f:?}");
                }
            }
        }
    }

    #[test]
    fn launch_fds_are_bad_requests_unless_all_three_match() {
        use FdAccess::*;
        use FdKind::*;
        let stdin = facts(Pipe, Read);
        let stdout = facts(Pipe, Write);
        let model = facts(Regular, Read);
        assert_eq!(check_launch_fds(stdin, stdout, model), Ok(()));
        for bad_model in [
            facts(Regular, ReadWrite),
            facts(Socket, ReadWrite),
            facts(Directory, Read),
        ] {
            assert_eq!(
                check_launch_fds(stdin, stdout, bad_model),
                Err(CageReply::BadRequest)
            );
        }
        assert_eq!(
            check_launch_fds(stdout, stdin, model),
            Err(CageReply::BadRequest)
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_reads_the_kind_and_access_of_real_fds() {
        use std::os::fd::AsFd;
        use std::os::unix::net::UnixStream;

        use FdAccess::*;
        use FdKind::*;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");
        std::fs::write(&path, b"gguf").unwrap();
        let ro = File::open(&path).unwrap();
        let rw = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let folder = File::open(dir.path()).unwrap();
        let (sock, _peer) = UnixStream::pair().unwrap();
        let (reader, writer) = io::pipe().unwrap();

        let probe = |fd: BorrowedFd<'_>| probe_fd(fd).unwrap();
        assert_eq!(probe(ro.as_fd()), facts(Regular, Read));
        assert_eq!(probe(rw.as_fd()), facts(Regular, ReadWrite));
        assert_eq!(probe(folder.as_fd()), facts(Directory, Read));
        assert_eq!(probe(sock.as_fd()), facts(Socket, ReadWrite));
        assert_eq!(probe(reader.as_fd()), facts(Pipe, Read));
        assert_eq!(probe(writer.as_fd()), facts(Pipe, Write));

        let stdin = probe(reader.as_fd());
        let stdout = probe(writer.as_fd());
        for bad in [rw.as_fd(), folder.as_fd(), sock.as_fd()] {
            assert_eq!(
                check_launch_fds(stdin, stdout, probe(bad)),
                Err(CageReply::BadRequest)
            );
        }
        assert_eq!(check_launch_fds(stdin, stdout, probe(ro.as_fd())), Ok(()));
    }
}

// 檻役と口の往復（プロセスの中の匿名の口。束も署名も要らない）。偽の子での採点の往復と、檻役が落ちたときの
// 子の終わりは lm/tests/cage_xpc.rs
#[cfg(all(test, target_vendor = "apple"))]
mod xpc_tests;
// スパイク #536（マージしない）
#[cfg(all(test, target_vendor = "apple"))]
mod wait_spike_tests;
