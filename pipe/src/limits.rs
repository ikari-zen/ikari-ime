//! 口ごとの上限と締め切りの表（ADR 0068 §7）。サーバー側（`Server`、`serve_connection`）が使う。
//!
//! 口（エンジン、設定、候補窓）ごとに、同時の接続の数、1 枠の大きさ、枠の途中で止まった相手への締め切り、
//! 書きの締め切りを、この 1 か所で決める。**値はすべて計測なしの仮置き**（ADR 0068 §7 は「何に上限を置くか」だけを
//! 決め、値は計測のあとに決める）。計測したら、ここだけを直す。
//!
//! 断った理由は `Refusal` の固定の語だけで知らせる（相手の内容は持たない。NFR-19）。

use std::time::Duration;

use crate::duplex::WRITE_DEADLINE;
use crate::frame::MAX_FRAME_LEN;

/// パイプの口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Port {
    /// TIP → engine-host
    Engine,
    /// 設定画面 → engine-host
    Settings,
    /// TIP → 候補窓
    Candidate,
}

/// 口 1 つの上限と締め切り。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortLimits {
    /// 同時につながっていてよい接続の数。超えた接続は、受けてすぐ切る
    pub max_connections: usize,
    /// 枠の `len`（種類と通し番号を含む）の上限。超えた枠は、本体を読まずに接続を切る
    pub max_frame_len: u32,
    /// 枠の最初のバイトが届いてから、枠が揃うまでの締め切り。超えたら切る。
    /// 何も送らずにつないだままの接続（枠の境目）には掛けない
    pub frame_read_deadline: Duration,
    /// 書きの締め切り（#392。`Duplex` の実装が守る値。ここでは、表に 1 つにまとめるために持つ）
    pub write_deadline: Duration,
}

/// エンジンの口の同時の接続数（計測なし・仮置き。実機で TIP の接続数を数えて決める）
pub const ENGINE_MAX_CONNECTIONS: usize = 64;
/// 候補窓の口の同時の接続数（計測なし・仮置き。同上）
pub const CANDIDATE_MAX_CONNECTIONS: usize = 64;
/// 設定の口の同時の接続数（計測なし・仮置き。設定画面は 1 つだけが前提の小さい固定の数）
pub const SETTINGS_MAX_CONNECTIONS: usize = 4;
/// 枠の途中の締め切り（計測なし・仮置き。正常な相手は 1 枠を 1 回の書きで送るので、十分に長い値）
pub const FRAME_READ_DEADLINE: Duration = Duration::from_secs(5);

/// 口の上限を引く。
///
/// 枠の大きさは、今の値（`MAX_FRAME_LEN`）のまま全部の口で同じにしてある（計測なし・仮置き。
/// 実際の最大の枠を測って、口ごとに下げる）。
pub const fn limits(port: Port) -> PortLimits {
    let max_connections = match port {
        Port::Engine => ENGINE_MAX_CONNECTIONS,
        Port::Settings => SETTINGS_MAX_CONNECTIONS,
        Port::Candidate => CANDIDATE_MAX_CONNECTIONS,
    };
    PortLimits {
        max_connections,
        max_frame_len: MAX_FRAME_LEN,
        frame_read_deadline: FRAME_READ_DEADLINE,
        write_deadline: WRITE_DEADLINE,
    }
}

/// 断った理由（固定の語。ログにはこれだけを書く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 同時の接続の数の上限を超えた
    TooManyConnections,
    /// 枠が大きさの上限を超えた
    FrameTooLarge,
    /// 枠の途中で止まったまま、締め切りを超えた
    FrameTimeout,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Refusal::TooManyConnections => "too-many-connections",
            Refusal::FrameTooLarge => "frame-too-large",
            Refusal::FrameTimeout => "frame-timeout",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_port_has_positive_limits() {
        for port in [Port::Engine, Port::Settings, Port::Candidate] {
            let l = limits(port);
            assert!(l.max_connections > 0);
            assert!(l.max_frame_len > 0 && l.max_frame_len <= MAX_FRAME_LEN);
            assert!(l.frame_read_deadline > Duration::ZERO);
            assert_eq!(l.write_deadline, WRITE_DEADLINE);
        }
    }

    #[test]
    fn settings_port_allows_fewer_connections_than_the_others() {
        assert!(limits(Port::Settings).max_connections < limits(Port::Engine).max_connections);
    }
}
