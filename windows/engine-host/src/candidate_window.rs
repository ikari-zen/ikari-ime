//! 候補窓（`ikari-candidate.exe`）を起動して見張る入口（ADR 0062 §5.4 の決定 1 と 4。#414）。
//!
//! **入口だけ。今は何もしない。** 起動と、Job Object の「閉じたら終了」で窓の寿命を engine-host のプロセスに
//! つなぐ本実装は、W3 の L4 で入れる（窓の本体がまだないため）。
//!
//! 順序の決定（ここを呼ぶ場所。`main.rs`）: 別のエンジンが動いていないことを確かめたあと、エンジンを開く前に呼ぶ。
//! 窓の寿命は、エンジンが開けたかどうかにつなげない（開けなくても engine-host は常駐し、窓はかなの候補を出す）。
//! 判断の置き場所: Rust。

/// 起動した候補窓。engine-host が終わるまで持つ（落とすと見張りをやめる。本実装では Job Object のハンドルを持つ）。
#[derive(Debug)]
pub struct CandidateWindow {
    _private: (),
}

/// 候補窓を起動して見張る。今は何もしない（W3 の L4 で本実装）。起動に失敗しても engine-host は動き続ける
/// （窓なしでも、下線つきの 1 位で入力を続けられる。ADR 0062 §5.4 の決定 2）。
pub fn launch() -> CandidateWindow {
    CandidateWindow { _private: () }
}
