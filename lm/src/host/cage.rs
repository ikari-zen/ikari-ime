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
//! 判断の置き場所: Rust（いつ起動し、いつ止めるかは今までどおり `scorer`。檻役は実行だけ）。

use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::host::{ARG_GPU, ARG_GPU_CPU, ARG_GPU_OFF, ARG_GPU_ON, ARG_MODEL};

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
/// 子（`ikari-lm-host`）が、親（檻役）が終わったかを見る間隔（固まった子が残らないように。ADR 0077 §3.1）
pub const PARENT_WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// XPC の辞書のキーと、要求の語（エンジン → 檻役）
pub mod wire {
    /// 取り決めの版（u64）
    pub const KEY_VERSION: &str = "v";
    /// 要求の語（[`OP_LAUNCH`] / [`OP_STOP`]）
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

    pub const OP_LAUNCH: &str = "launch";
    pub const OP_STOP: &str = "stop";
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
    /// 要求の形が違う（知らない語、fd が足りない）
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

/// 檻役への口（エンジンの側）。製品は libxpc の匿名の口（D-3-mac）、テストは差し替える
pub trait CageClient: Send + Sync + fmt::Debug {
    /// 子を 1 つ起動してもらう。口の先の署名要件を確かめる前は、`launch` を受けない（`Err`）
    fn launch(&self, launch: CageLaunch) -> io::Result<Box<dyn CageChild>>;
}

/// 殻から届いた檻役の口を入れておく場所（エンジンに 1 つ）。口は、エンジンが開いたあと（`Hello` のたび、
/// 檻役との接続を取り直したあと）に届くので、起動の口（`HostLauncher::Cage`）はこれを引く
#[derive(Default)]
pub struct CageSlot {
    client: Mutex<Option<Arc<dyn CageClient>>>,
}

impl fmt::Debug for CageSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CageSlot")
            .field("attached", &self.current().is_some())
            .finish()
    }
}

impl CageSlot {
    fn guard(&self) -> MutexGuard<'_, Option<Arc<dyn CageClient>>> {
        self.client.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 口を入れる（前の口は捨てる）
    pub fn attach(&self, client: Arc<dyn CageClient>) {
        *self.guard() = Some(client);
    }

    /// 口を捨てる（確かめに失敗した、接続が切れた）
    pub fn detach(&self) {
        *self.guard() = None;
    }

    /// 今の口（無ければ「檻なし」）
    pub fn current(&self) -> Option<Arc<dyn CageClient>> {
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

    #[test]
    fn slot_starts_empty_and_can_be_replaced() {
        #[derive(Debug)]
        struct Never;
        impl CageClient for Never {
            fn launch(&self, _launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            }
        }
        let slot = CageSlot::default();
        assert!(slot.current().is_none());
        slot.attach(Arc::new(Never));
        assert!(slot.current().is_some());
        slot.detach();
        assert!(slot.current().is_none());
    }

    // ---- D-3-mac の骨組み（実装のレーンで中身を書き、`#[ignore]` を外す）

    #[test]
    #[ignore = "D-3-mac: 檻役の子に付く fd が 0・1・3 だけ（POSIX_SPAWN_CLOEXEC_DEFAULT）"]
    fn caged_child_gets_only_fds_0_1_3() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 檻役の子の環境変数が空"]
    fn caged_child_has_an_empty_environment() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 全体の上限（MAX_CHILDREN_TOTAL）を超える起動は Busy"]
    fn launches_beyond_the_total_limit_are_busy() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 接続ごとに子は 1 つまで（2 つ目は Busy）"]
    fn one_child_per_connection() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 止めるは SIGKILL と回収のあとで返事をする"]
    fn stop_replies_after_kill_and_reap() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 接続を捨てたら、檻役が子を止めて回収する"]
    fn dropping_the_connection_reaps_the_child() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 版が違う要求は VersionMismatch で、起動しない"]
    fn version_mismatch_is_refused_without_spawning() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 署名要件を確かめる前は、モデルの fd も採点の入力も渡さない"]
    fn engine_sends_nothing_before_the_peer_is_verified() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 檻役が落ちたら、子は親の見張りで自分で終わる"]
    fn child_exits_when_the_cage_dies() {
        unimplemented!("D-3-mac")
    }
}
