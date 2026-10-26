//! mmap した連接表と品詞表（ADR 0001 §3.3, §4.1、ADR 0002 §3.3）。
//!
//! 連接表（約71MB）はコピーせず、mmap のページを触れた分だけ常駐させる。
//! 開くときは O(1) の検証と、活用規則の節の読み込み（数百件）だけを行う。
//! grammar_id の再計算（連接表の全走査）は builder の `--verify`。

use std::path::Path;
use std::sync::Arc;

use ikari_dict_format::{
    GrammarLayout, matrix_cost, parse_grammar_layout, read_conj_rules, read_pos_record,
};
use memmap2::Mmap;

use super::system::map_file;
use super::{ConjRule, DictError, Grammar, PosInfo, UnknownParams};

pub struct GrammarFile {
    mmap: Arc<Mmap>,
    layout: GrammarLayout,
    conj_rules: Vec<ConjRule>,
}

impl GrammarFile {
    pub fn open(path: &Path) -> Result<Self, DictError> {
        let mmap = map_file(path, "grammar.bin")?;
        let layout = parse_grammar_layout(&mmap)?;
        let conj_section =
            mmap.get(layout.conj.clone())
                .ok_or(ikari_dict_format::FormatError::BadSection {
                    name: "conj",
                    message: "section is out of the file".to_string(),
                })?;
        let conj_rules = read_conj_rules(conj_section, layout.pos_count)?
            .into_iter()
            .map(|r| ConjRule {
                base_pos: r.base_pos,
                target_pos: r.target_pos,
                strip_chars: r.strip_chars,
                add_surface: r.add_surface.to_owned(),
                add_reading: r.add_reading.to_owned(),
            })
            .collect();
        Ok(Self {
            mmap,
            layout,
            conj_rules,
        })
    }

    /// 品詞の数。
    pub fn pos_count(&self) -> usize {
        self.layout.pos_count as usize
    }

    fn pos_section(&self) -> Option<&[u8]> {
        self.mmap.get(self.layout.pos.clone())
    }
}

impl Grammar for GrammarFile {
    fn connection_cost(&self, prev_right_id: u16, next_left_id: u16) -> i16 {
        // セクションの範囲は open 時に検証済み。万一ずれていても `i16::MAX`（接続禁止）を返す。
        let Some(matrix) = self.mmap.get(self.layout.matrix.clone()) else {
            return i16::MAX;
        };
        matrix_cost(
            matrix,
            self.layout.prev_right_size,
            self.layout.next_left_size,
            prev_right_id,
            next_left_id,
        )
    }

    fn bos_eos_id(&self) -> u16 {
        self.layout.bos_eos_id
    }

    fn unknown(&self) -> UnknownParams {
        UnknownParams {
            left_id: self.layout.unk_left_id,
            right_id: self.layout.unk_right_id,
            pos_id: self.layout.unk_pos_id,
            cost_per_char: self.layout.unk_cost,
        }
    }

    fn pos_name(&self, pos_id: u16) -> Option<&str> {
        let section = self.pos_section()?;
        read_pos_record(section, self.layout.pos_count, pos_id).map(|r| r.name)
    }

    fn grammar_id(&self) -> u64 {
        self.layout.grammar_id
    }

    /// 品詞表は名前の昇順なので二分探索する。
    fn pos_id(&self, name: &str) -> Option<u16> {
        let section = self.pos_section()?;
        let (mut lo, mut hi) = (0u32, self.layout.pos_count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let record = read_pos_record(section, self.layout.pos_count, u16::try_from(mid).ok()?)?;
            match record.name.cmp(name) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return u16::try_from(mid).ok(),
            }
        }
        None
    }

    fn pos_info(&self, pos_id: u16) -> Option<PosInfo> {
        let section = self.pos_section()?;
        read_pos_record(section, self.layout.pos_count, pos_id).map(|r| PosInfo {
            rep_left_id: r.rep_left_id,
            rep_right_id: r.rep_right_id,
            user_cost: r.user_cost,
            rep_share_permille: r.rep_share_permille,
        })
    }

    fn conj_rules(&self) -> &[ConjRule] {
        &self.conj_rules
    }
}
