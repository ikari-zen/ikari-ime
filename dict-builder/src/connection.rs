//! 連接表の補正（ADR 0054）。格助詞「の」の直後の「件」（ケン）だけを安くする。
//!
//! 「件」の 1 行だけに専用の左 ID を付け、連接表に元の左 ID の列を写した列を足し、
//! 「の」の右 ID からその列への 1 マスだけを δ 下げる。規則（どの語・どの前の語）はここに固定し、
//! 値（δ）は policy に置く（ADR 0002 §4 と同じ分け方）。δ が 0 なら何もしない（grammar.bin は変わらない）。
//! 辞書を更新したときは、語を表記・読み・品詞で探し、見つからない・共有されている・δ が閾値以下なら
//! ビルドを失敗にする（ADR 0054 §6.1）。

use std::fmt::Write as _;

use ikari_dict_format::IME_EXCLUDED;

use crate::lex::Lexicon;
use crate::matrix::Matrix;
use crate::policy::ConnectionDeltas;
use crate::{BOS_EOS_ID, BuildError};

const KEN_SURFACE: &str = "件";
const KEN_READING: &str = "けん";
/// 件の行の品詞（品詞1〜3。助数詞可能の普通名詞）
const KEN_POS_PREFIX: &str = "名詞,普通名詞,助数詞可能";
const NO_SURFACE: &str = "の";
/// 格助詞（品詞1〜2）
const CASE_PARTICLE_POS_PREFIX: &str = "助詞,格助詞";
const RENTAI_POS1: &str = "連体詞";
/// 連体詞のうち、対象にする語（その右 ID は連体詞の全体で共有される。ADR 0054 §3）
const RENTAI_SURFACES: [&str; 4] = ["この", "その", "あの", "どの"];
/// 競合の語の品詞1（読みが同じ名詞）
const RIVAL_POS1: &str = "名詞";
const SIDE_AFTER_NO: &str = "after_no";
const SIDE_AFTER_RENTAI: &str = "after_rentai";

/// 補正の記録（`--policy-report`、メタ情報、Display）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    /// 件の行の元の左 ID と、割り当てた新しい左 ID
    pub ken_left_before: u16,
    pub ken_left_after: u16,
    /// 格助詞「の」の直後（δ が 0 なら `None`）
    pub after_no: Option<SideReport>,
    /// 連体詞の直後（δ が 0 なら `None`）
    pub after_rentai: Option<SideReport>,
}

/// 前の語の種類ごとの記録
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideReport {
    /// 前の語の右 ID の集合 P（昇順）
    pub rights: Vec<u16>,
    pub delta: i32,
    /// 閾値 T（件が競合に勝つのに要る δ の下限。競合がなければ `None`）
    pub threshold: Option<i32>,
    /// 競合の語（T を決めた語。`表記（読み）`）
    pub rival: Option<String>,
}

impl ConnectionReport {
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("\n## 連接表の補正（ADR 0054）\n\n");
        let _ = writeln!(
            out,
            "- 件（ケン）の左 ID: {} → {}（連接表に列を 1 つ足した）",
            self.ken_left_before, self.ken_left_after
        );
        if let Some(side) = &self.after_no {
            let _ = writeln!(
                out,
                "- 格助詞「の」の直後（matter_ken_after_no）: {}",
                side.describe()
            );
        }
        if let Some(side) = &self.after_rentai {
            let _ = writeln!(
                out,
                "- 連体詞の直後（matter_ken_after_rentai）: {}",
                side.describe()
            );
        }
        out
    }

    /// メタ情報の行（`key=value`）
    pub fn meta_lines(&self) -> Vec<(String, String)> {
        let mut lines = vec![(
            "connection.ken_left".to_owned(),
            format!("{}->{}", self.ken_left_before, self.ken_left_after),
        )];
        for (key, side) in [
            (SIDE_AFTER_NO, &self.after_no),
            (SIDE_AFTER_RENTAI, &self.after_rentai),
        ] {
            if let Some(side) = side {
                lines.push((format!("connection.{key}"), side.describe()));
            }
        }
        lines
    }
}

impl SideReport {
    fn describe(&self) -> String {
        let rights: Vec<String> = self.rights.iter().map(u16::to_string).collect();
        let mut out = format!("右 ID {{{}}}、δ = {}", rights.join(","), self.delta);
        match (&self.threshold, &self.rival) {
            (Some(t), Some(rival)) => {
                let _ = write!(out, "、閾値 T = {t}（競合 {rival}）");
            }
            _ => out.push_str("、競合なし"),
        }
        out
    }
}

fn config_error(message: impl Into<String>) -> BuildError {
    BuildError::Config(format!("connection (ADR 0054): {}", message.into()))
}

fn pos_has_prefix(pos: &str, prefix: &str) -> bool {
    pos == prefix
        || pos
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(','))
}

fn pos1(pos: &str) -> &str {
    pos.split(',').next().unwrap_or("")
}

fn pos_name(lexicon: &Lexicon, pos_id: u16) -> &str {
    &lexicon.pos[usize::from(pos_id)].name
}

/// 件の行を探す。ちょうど 1 行でなければ失敗
fn find_ken(lexicon: &Lexicon) -> Result<usize, BuildError> {
    let hits: Vec<usize> = lexicon
        .rows
        .iter()
        .enumerate()
        .filter(|(_, (row, pos_id))| {
            row.surface == KEN_SURFACE
                && row.reading == KEN_READING
                && pos_has_prefix(pos_name(lexicon, *pos_id), KEN_POS_PREFIX)
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        other => Err(config_error(format!(
            "expected exactly one row 件/ケン/{KEN_POS_PREFIX}, found {}",
            other.len()
        ))),
    }
}

/// 前の語の右 ID の集合 P（昇順）。`is_prev(表記, 品詞)` に当たる行の右 ID
fn collect_rights(lexicon: &Lexicon, is_prev: impl Fn(&str, &str) -> bool) -> Vec<u16> {
    let mut rights: Vec<u16> = lexicon
        .rows
        .iter()
        .filter(|(row, pos_id)| is_prev(&row.surface, pos_name(lexicon, *pos_id)))
        .map(|(row, _)| row.right_id)
        .collect();
    rights.sort_unstable();
    rights.dedup();
    rights
}

fn is_no(surface: &str, pos: &str) -> bool {
    surface == NO_SURFACE && pos_has_prefix(pos, CASE_PARTICLE_POS_PREFIX)
}

fn is_rentai(surface: &str, pos: &str) -> bool {
    pos1(pos) == RENTAI_POS1 && RENTAI_SURFACES.contains(&surface)
}

/// 経路「前の語（右 ID r）→ 語 → EOS」の変換用のコスト（連接 + 単語 + ime_penalty + 連接）
fn path_cost(matrix: &Matrix, r: u16, row: &crate::lex::RawRow, penalty: i16) -> i32 {
    i32::from(matrix.cost(r, row.left_id))
        + i32::from(row.cost)
        + i32::from(penalty)
        + i32::from(matrix.cost(row.right_id, BOS_EOS_ID))
}

/// 閾値 T = 件の経路 − 最も安い競合の経路（P の右 ID ごとに計算して最大を取る）。
/// 競合 = 読みがケンの名詞（件を除く。変換に使わない行も除く）。競合がなければ `None`
fn threshold(
    lexicon: &Lexicon,
    penalties: &[i16],
    matrix: &Matrix,
    ken: usize,
    rights: &[u16],
) -> Option<(i32, String)> {
    let mut best: Option<(i32, String)> = None;
    for &r in rights {
        let ken_total = path_cost(matrix, r, &lexicon.rows[ken].0, penalties[ken]);
        let rival = lexicon
            .rows
            .iter()
            .enumerate()
            .filter(|(i, (row, pos_id))| {
                *i != ken
                    && row.reading == KEN_READING
                    && penalties[*i] != IME_EXCLUDED
                    && pos1(pos_name(lexicon, *pos_id)) == RIVAL_POS1
            })
            .map(|(i, (row, _))| {
                (
                    path_cost(matrix, r, row, penalties[i]),
                    format!("{}（{}）", row.surface, row.reading),
                )
            })
            .min_by_key(|(total, _)| *total);
        if let Some((rival_total, name)) = rival {
            let t = ken_total - rival_total;
            if best.as_ref().is_none_or(|(b, _)| t > *b) {
                best = Some((t, name));
            }
        }
    }
    best
}

/// 前の語の種類 1 つぶんの補正（検証済み）
struct Side {
    key: &'static str,
    delta: i32,
    rights: Vec<u16>,
}

/// 連接表と件の行を補正する。どちらの δ も 0 なら何もせず `None`。
/// `penalties` は `lexicon.rows` と同じ並びの `ime_penalty`。`enforce_threshold` が偽なら δ ≤ T でも失敗にしない（掃引用）。
pub(crate) fn apply(
    lexicon: &mut Lexicon,
    penalties: &[i16],
    matrix: &mut Matrix,
    deltas: ConnectionDeltas,
    enforce_threshold: bool,
) -> Result<Option<ConnectionReport>, BuildError> {
    if deltas.matter_ken_after_no == 0 && deltas.matter_ken_after_rentai == 0 {
        return Ok(None);
    }
    let ken = find_ken(lexicon)?;

    // 前の語の右 ID の集合
    let mut sides: Vec<Side> = Vec::new();
    if deltas.matter_ken_after_no != 0 {
        let rights = collect_rights(lexicon, is_no);
        if rights.is_empty() {
            return Err(config_error("no row 格助詞「の」 was found"));
        }
        // 右 ID が「の」以外の行と共有されていたら、補正がほかの語に漏れる。ただし表記が「の」で終わる行
        // （notcore の固有名詞の題名。ひとつ屋根の下の など）は、右 ID が「の」で終わることを表すので、漏れに数えない
        let leaked = lexicon.rows.iter().find(|(row, pos_id)| {
            rights.contains(&row.right_id)
                && !row.surface.ends_with(NO_SURFACE)
                && !is_no(&row.surface, pos_name(lexicon, *pos_id))
        });
        if let Some((row, _)) = leaked {
            return Err(config_error(format!(
                "right id {} of 格助詞「の」 is shared with `{}`",
                row.right_id, row.surface
            )));
        }
        sides.push(Side {
            key: SIDE_AFTER_NO,
            delta: deltas.matter_ken_after_no,
            rights,
        });
    }
    if deltas.matter_ken_after_rentai != 0 {
        let rights = collect_rights(lexicon, is_rentai);
        if rights.is_empty() {
            return Err(config_error(
                "no row 連体詞 (この/その/あの/どの) was found",
            ));
        }
        sides.push(Side {
            key: SIDE_AFTER_RENTAI,
            delta: deltas.matter_ken_after_rentai,
            rights,
        });
    }
    if let [a, b] = sides.as_slice()
        && a.rights.iter().any(|r| b.rights.contains(r))
    {
        return Err(config_error("格助詞「の」 and 連体詞 share a right id"));
    }

    // 閾値の確認（元の連接表で計算する。補正が効かなくなったことを黙って見逃さない）
    let mut side_reports: Vec<(&'static str, SideReport)> = Vec::new();
    for side in &sides {
        let t = threshold(lexicon, penalties, matrix, ken, &side.rights);
        if let Some((t, rival)) = &t
            && enforce_threshold
            && side.delta <= *t
        {
            return Err(config_error(format!(
                "{}: delta {} does not exceed the threshold {t} (rival {rival}); the correction would have no effect",
                side.key, side.delta
            )));
        }
        side_reports.push((
            side.key,
            SideReport {
                rights: side.rights.clone(),
                delta: side.delta,
                threshold: t.as_ref().map(|(t, _)| *t),
                rival: t.map(|(_, name)| name),
            },
        ));
    }

    // 新しい左 ID（列の写し）と、P の各右 ID からの 1 マス
    let ken_left_before = lexicon.rows[ken].0.left_id;
    let ken_left_after = matrix
        .push_left_column_copy(ken_left_before)
        .ok_or_else(|| config_error("left id does not fit in u16"))?;
    for side in &sides {
        for &r in &side.rights {
            let lowered = i32::from(matrix.cost(r, ken_left_after)) - side.delta;
            let clamped = lowered.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            matrix.set_cost(r, ken_left_after, clamped as i16);
        }
    }
    lexicon.rows[ken].0.left_id = ken_left_after;

    let mut report = ConnectionReport {
        ken_left_before,
        ken_left_after,
        after_no: None,
        after_rentai: None,
    };
    for (key, side) in side_reports {
        if key == SIDE_AFTER_NO {
            report.after_no = Some(side);
        } else {
            report.after_rentai = Some(side);
        }
    }
    Ok(Some(report))
}
