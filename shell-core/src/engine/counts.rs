//! エンジンとの通信の事象の数（ADR 0034 §4.2 の `sh_engine`。移行の前の Swift の `DiagEngineCounts`）。
//! メモリ上の整数だけ。キーは engine の許可リスト（`engine/src/diag_schema.rs`）に合わせる。

use ikari_proto::v1;

/// `DiagEvent.kind`
pub const KIND: &str = "sh_engine";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineCounts {
    /// 返事が時間内に来なかった
    pub timeout: i64,
    /// 相手のプロセスが落ちた（接続は有効なまま）
    pub interrupted: i64,
    /// 接続が無効になった
    pub invalidated: i64,
    /// 変換が失敗して、殻がエンジンなしの候補で補った
    pub fallback_convs: i64,
    /// 連続して落ちて、エンジンを呼ばずに休み始めた
    pub breaker_open: i64,
    /// 接続を作り直した
    pub reconnect: i64,
    /// 落ちたあとに、`Hello` とセッションを送り直した
    pub resync: i64,
}

impl EngineCounts {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 送った分を引く（送っている間に増えた分は残す）
    pub fn subtract(&mut self, sent: &Self) {
        self.timeout -= sent.timeout;
        self.interrupted -= sent.interrupted;
        self.invalidated -= sent.invalidated;
        self.fallback_convs -= sent.fallback_convs;
        self.breaker_open -= sent.breaker_open;
        self.reconnect -= sent.reconnect;
        self.resync -= sent.resync;
    }

    /// 0 でないキーだけを、キーの昇順で数として持つ
    pub fn event(&self, session_id: u64) -> v1::DiagEvent {
        let all = [
            ("breaker_open", self.breaker_open),
            ("fallback_convs", self.fallback_convs),
            ("interrupted", self.interrupted),
            ("invalidated", self.invalidated),
            ("reconnect", self.reconnect),
            ("resync", self.resync),
            ("timeout", self.timeout),
        ];
        v1::DiagEvent {
            session_id,
            kind: KIND.to_owned(),
            nums: all
                .into_iter()
                .filter(|(_, value)| *value != 0)
                .map(|(key, value)| v1::DiagNum {
                    key: key.to_owned(),
                    value,
                })
                .collect(),
            tags: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_zero_keys_are_sent_in_key_order() {
        let counts = EngineCounts {
            timeout: 1,
            fallback_convs: 2,
            ..Default::default()
        };
        let event = counts.event(3);
        assert_eq!(event.kind, KIND);
        assert_eq!(event.session_id, 3);
        let keys: Vec<_> = event
            .nums
            .iter()
            .map(|n| (n.key.as_str(), n.value))
            .collect();
        assert_eq!(keys, vec![("fallback_convs", 2), ("timeout", 1)]);
        assert!(event.tags.is_empty());
    }

    #[test]
    fn subtract_keeps_what_was_added_while_sending() {
        let sent = EngineCounts {
            timeout: 1,
            ..Default::default()
        };
        let mut now = EngineCounts {
            timeout: 2,
            resync: 1,
            ..Default::default()
        };
        now.subtract(&sent);
        assert_eq!(
            now,
            EngineCounts {
                timeout: 1,
                resync: 1,
                ..Default::default()
            }
        );
        assert!(!now.is_empty());
    }
}
