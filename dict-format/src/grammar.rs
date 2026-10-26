//! `grammar.bin`（連接表と品詞表）のエンコードと解析（ADR 0001 §3.3）。

use std::ops::Range;

use crate::util::{
    Fnv1a64, HeaderReader, align_up, check_preamble, check_section, get_u16, get_u32, pad_to_align,
};
use crate::{FORMAT_VERSION, FormatError, GRAMMAR_HEADER_LEN, GRAMMAR_MAGIC};

const WHAT: &str = "grammar.bin";

// ヘッダのフィールド位置（§3.3）
const OFF_GRAMMAR_ID: usize = 16;
const OFF_PREV_RIGHT_SIZE: usize = 24;
const OFF_NEXT_LEFT_SIZE: usize = 26;
const OFF_BOS_EOS_ID: usize = 28;
const OFF_UNK_LEFT_ID: usize = 30;
const OFF_UNK_RIGHT_ID: usize = 32;
const OFF_UNK_POS_ID: usize = 34;
const OFF_UNK_COST: usize = 36;
const OFF_POS_COUNT: usize = 40;
const OFF_MATRIX_OFFSET: usize = 48;
const OFF_MATRIX_LEN: usize = 56;
const OFF_POS_OFFSET: usize = 64;
const OFF_POS_LEN: usize = 72;
const OFF_CONJ_OFFSET: usize = 80;
const OFF_CONJ_LEN: usize = 88;

/// 連接表の1要素のバイト数（i16）
const COST_SIZE: usize = 2;
/// 品詞表のオフセット配列の1要素のバイト数（u32）
const POS_OFFSET_SIZE: usize = 4;
/// 品詞レコードの固定部（rep_left_id, rep_right_id, user_cost, rep_share_permille）のバイト数
const POS_RECORD_FIXED: usize = 8;
/// 活用規則の節の先頭（rule_count）のバイト数
const CONJ_COUNT_SIZE: usize = 4;
/// 活用規則 1件の固定部（base_pos, target_pos, strip_chars, add_surface_len, add_reading_len, 予約）
const CONJ_RULE_FIXED: usize = 8;
/// pos_id は u16 なので品詞は最大 65536 件
const MAX_POS_COUNT: usize = 1 << 16;

/// 品詞表の1レコード。pos_id は品詞名の辞書順の添字（呼び出し側が整列済みで渡す）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PosRecord<'a> {
    pub rep_left_id: u16,
    pub rep_right_id: u16,
    /// ユーザー辞書の語のコスト（その品詞のシステム辞書のコストの10パーセンタイル）
    pub user_cost: i16,
    /// 代表の左右IDと一致するエントリの割合（‰）
    pub rep_share_permille: u16,
    pub name: &'a str,
}

/// 活用規則 1件。終止形（base_pos）の末尾から `strip_chars` 文字を削って `add_*` を足すと、
/// target_pos の活用形になる（ADR 0002 §3.3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConjRecord<'a> {
    pub base_pos: u16,
    pub target_pos: u16,
    pub strip_chars: u8,
    pub add_surface: &'a str,
    /// ひらがな
    pub add_reading: &'a str,
}

/// `encode_grammar` の入力。grammar_id と各セクションの位置は encode が決める。
#[derive(Debug, Clone)]
pub struct GrammarSpec<'a> {
    pub prev_right_size: u16,
    pub next_left_size: u16,
    pub bos_eos_id: u16,
    pub unk_left_id: u16,
    pub unk_right_id: u16,
    pub unk_pos_id: u16,
    pub unk_cost: i16,
    /// `matrix[next_left * prev_right_size + prev_right]`、長さ `prev_right_size * next_left_size`
    pub matrix: &'a [i16],
    /// 品詞名の昇順（重複なし）
    pub pos: &'a [PosRecord<'a>],
    /// 活用規則。`(base_pos, target_pos)` の昇順（重複なし）
    pub conj: &'a [ConjRecord<'a>],
}

/// `parse_grammar_layout` の結果。範囲は入力 `&[u8]` 内のバイト位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarLayout {
    pub grammar_id: u64,
    pub prev_right_size: u16,
    pub next_left_size: u16,
    pub bos_eos_id: u16,
    pub unk_left_id: u16,
    pub unk_right_id: u16,
    pub unk_pos_id: u16,
    pub unk_cost: i16,
    pub pos_count: u32,
    /// 連接表（i16 LE の密行列）
    pub matrix: Range<usize>,
    /// 品詞表（`u32 offsets[pos_count + 1]` + レコード領域）
    pub pos: Range<usize>,
    /// 活用規則の節（`u32 rule_count` + 規則）
    pub conj: Range<usize>,
}

fn invalid(message: impl Into<String>) -> FormatError {
    FormatError::InvalidInput(message.into())
}

fn bad_section(name: &'static str, message: impl Into<String>) -> FormatError {
    FormatError::BadSection {
        name,
        message: message.into(),
    }
}

/// grammar_id = FNV-1a 64 を `prev_right_size, next_left_size, bos_eos_id`（LE）、連接表セクション、
/// 品詞表セクション、活用規則セクションのバイト列の順にかけた値。
pub fn compute_grammar_id(
    prev_right_size: u16,
    next_left_size: u16,
    bos_eos_id: u16,
    matrix_section: &[u8],
    pos_section: &[u8],
    conj_section: &[u8],
) -> u64 {
    let mut h = Fnv1a64::new();
    h.update(&prev_right_size.to_le_bytes());
    h.update(&next_left_size.to_le_bytes());
    h.update(&bos_eos_id.to_le_bytes());
    h.update(matrix_section);
    h.update(pos_section);
    h.update(conj_section);
    h.finish()
}

/// ヘッダの grammar_id が、実際のセクションの内容から計算した値と一致するか確かめる。
/// 連接表の全体を読むので O(ファイルサイズ)。open 時ではなく、builder の `--verify` で使う。
pub fn verify_grammar_id(bytes: &[u8], layout: &GrammarLayout) -> Result<(), FormatError> {
    let (Some(matrix), Some(pos), Some(conj)) = (
        bytes.get(layout.matrix.clone()),
        bytes.get(layout.pos.clone()),
        bytes.get(layout.conj.clone()),
    ) else {
        return Err(bad_section("grammar", "layout does not match the bytes"));
    };
    let actual = compute_grammar_id(
        layout.prev_right_size,
        layout.next_left_size,
        layout.bos_eos_id,
        matrix,
        pos,
        conj,
    );
    if actual != layout.grammar_id {
        return Err(bad_section(
            "grammar_id",
            format!(
                "header says {:#018x} but contents hash to {actual:#018x}",
                layout.grammar_id
            ),
        ));
    }
    Ok(())
}

/// grammar.bin のバイト列を作る。grammar_id（FNV-1a 64）もここで計算する。
pub fn encode_grammar(spec: &GrammarSpec<'_>) -> Result<Vec<u8>, FormatError> {
    let prev = usize::from(spec.prev_right_size);
    let next = usize::from(spec.next_left_size);
    if prev == 0 || next == 0 {
        return Err(invalid("matrix size must be non-zero"));
    }
    if spec.matrix.len() != prev * next {
        return Err(invalid(format!(
            "matrix has {} elements but {prev} x {next} = {} expected",
            spec.matrix.len(),
            prev * next
        )));
    }
    if spec.pos.len() > MAX_POS_COUNT {
        return Err(invalid(format!(
            "too many parts of speech: {}",
            spec.pos.len()
        )));
    }
    let left_in_range = |id: u16| usize::from(id) < next;
    let right_in_range = |id: u16| usize::from(id) < prev;
    if !left_in_range(spec.bos_eos_id) || !right_in_range(spec.bos_eos_id) {
        return Err(invalid(format!(
            "bos_eos_id {} is out of range",
            spec.bos_eos_id
        )));
    }
    if !left_in_range(spec.unk_left_id) || !right_in_range(spec.unk_right_id) {
        return Err(invalid("unknown-word left/right id is out of range"));
    }
    if usize::from(spec.unk_pos_id) >= spec.pos.len() {
        return Err(invalid(format!(
            "unk_pos_id {} is out of range",
            spec.unk_pos_id
        )));
    }
    for pair in spec.pos.windows(2) {
        if pair[0].name >= pair[1].name {
            return Err(invalid(format!(
                "part-of-speech names must be strictly ascending: {:?} then {:?}",
                pair[0].name, pair[1].name
            )));
        }
    }
    for rec in spec.pos {
        if !left_in_range(rec.rep_left_id) || !right_in_range(rec.rep_right_id) {
            return Err(invalid(format!(
                "representative id of {:?} is out of range",
                rec.name
            )));
        }
    }

    // 品詞表: u32 offsets[pos_count + 1] + レコード領域
    let offsets_len = (spec.pos.len() + 1) * POS_OFFSET_SIZE;
    let mut offsets = Vec::with_capacity(offsets_len);
    let mut records = Vec::new();
    for rec in spec.pos {
        let offset = u32::try_from(records.len())
            .map_err(|_| invalid("part-of-speech table is too large"))?;
        offsets.extend_from_slice(&offset.to_le_bytes());
        records.extend_from_slice(&rec.rep_left_id.to_le_bytes());
        records.extend_from_slice(&rec.rep_right_id.to_le_bytes());
        records.extend_from_slice(&rec.user_cost.to_le_bytes());
        records.extend_from_slice(&rec.rep_share_permille.to_le_bytes());
        records.extend_from_slice(rec.name.as_bytes());
    }
    let end =
        u32::try_from(records.len()).map_err(|_| invalid("part-of-speech table is too large"))?;
    offsets.extend_from_slice(&end.to_le_bytes());
    let mut pos_section = offsets;
    pos_section.extend_from_slice(&records);

    let conj_section = encode_conj(spec.conj, spec.pos.len())?;

    let mut matrix_section = Vec::with_capacity(spec.matrix.len() * COST_SIZE);
    for cost in spec.matrix {
        matrix_section.extend_from_slice(&cost.to_le_bytes());
    }

    let grammar_id = compute_grammar_id(
        spec.prev_right_size,
        spec.next_left_size,
        spec.bos_eos_id,
        &matrix_section,
        &pos_section,
        &conj_section,
    );

    let matrix_offset = GRAMMAR_HEADER_LEN;
    let pos_offset = align_up(matrix_offset + matrix_section.len());
    let conj_offset = align_up(pos_offset + pos_section.len());

    let mut out = Vec::with_capacity(conj_offset + conj_section.len());
    out.extend_from_slice(&GRAMMAR_MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(GRAMMAR_HEADER_LEN as u32).to_le_bytes());
    out.extend_from_slice(&grammar_id.to_le_bytes());
    out.extend_from_slice(&spec.prev_right_size.to_le_bytes());
    out.extend_from_slice(&spec.next_left_size.to_le_bytes());
    out.extend_from_slice(&spec.bos_eos_id.to_le_bytes());
    out.extend_from_slice(&spec.unk_left_id.to_le_bytes());
    out.extend_from_slice(&spec.unk_right_id.to_le_bytes());
    out.extend_from_slice(&spec.unk_pos_id.to_le_bytes());
    out.extend_from_slice(&spec.unk_cost.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // 予約
    out.extend_from_slice(&(spec.pos.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 予約
    out.extend_from_slice(&(matrix_offset as u64).to_le_bytes());
    out.extend_from_slice(&(matrix_section.len() as u64).to_le_bytes());
    out.extend_from_slice(&(pos_offset as u64).to_le_bytes());
    out.extend_from_slice(&(pos_section.len() as u64).to_le_bytes());
    out.extend_from_slice(&(conj_offset as u64).to_le_bytes());
    out.extend_from_slice(&(conj_section.len() as u64).to_le_bytes());
    debug_assert_eq!(out.len(), GRAMMAR_HEADER_LEN);

    out.extend_from_slice(&matrix_section);
    pad_to_align(&mut out);
    debug_assert_eq!(out.len(), pos_offset);
    out.extend_from_slice(&pos_section);
    pad_to_align(&mut out);
    debug_assert_eq!(out.len(), conj_offset);
    out.extend_from_slice(&conj_section);
    pad_to_align(&mut out);
    Ok(out)
}

/// 活用規則の節を作る。規則は `(base_pos, target_pos)` の昇順で重複なし、品詞は `pos_count` 未満。
fn encode_conj(rules: &[ConjRecord<'_>], pos_count: usize) -> Result<Vec<u8>, FormatError> {
    let count = u32::try_from(rules.len()).map_err(|_| invalid("too many conjugation rules"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_le_bytes());
    let mut prev: Option<(u16, u16)> = None;
    for rule in rules {
        let key = (rule.base_pos, rule.target_pos);
        if usize::from(rule.base_pos) >= pos_count || usize::from(rule.target_pos) >= pos_count {
            return Err(invalid(format!(
                "conjugation rule {key:?} refers to an unknown part of speech"
            )));
        }
        if prev.is_some_and(|p| p >= key) {
            return Err(invalid(format!(
                "conjugation rules must be strictly ascending by (base_pos, target_pos): {key:?}"
            )));
        }
        prev = Some(key);
        let surface_len = u8::try_from(rule.add_surface.len())
            .map_err(|_| invalid("add_surface of a conjugation rule is too long"))?;
        let reading_len = u8::try_from(rule.add_reading.len())
            .map_err(|_| invalid("add_reading of a conjugation rule is too long"))?;
        out.extend_from_slice(&rule.base_pos.to_le_bytes());
        out.extend_from_slice(&rule.target_pos.to_le_bytes());
        out.push(rule.strip_chars);
        out.push(surface_len);
        out.push(reading_len);
        out.push(0); // 予約
        out.extend_from_slice(rule.add_surface.as_bytes());
        out.extend_from_slice(rule.add_reading.as_bytes());
    }
    Ok(out)
}

/// 活用規則の節（`GrammarLayout::conj` の範囲のスライス）をすべて読む。
/// 壊れた節（件数が長さに合わない、UTF-8 不正、品詞が範囲外、昇順でない）はエラー。
pub fn read_conj_rules(
    conj_section: &[u8],
    pos_count: u32,
) -> Result<Vec<ConjRecord<'_>>, FormatError> {
    let bad = |message: String| bad_section("conj", message);
    let count = get_u32(conj_section, 0).ok_or_else(|| bad("section is too short".to_string()))?;
    // 規則 1件は固定部だけでも 8 バイトある。件数が長さと矛盾していれば、確保の前に弾く。
    if u64::from(count) * CONJ_RULE_FIXED as u64 > conj_section.len() as u64 {
        return Err(bad(format!("rule_count {count} does not fit the section")));
    }
    let mut rules = Vec::with_capacity(count as usize);
    let mut at = CONJ_COUNT_SIZE;
    let mut prev: Option<(u16, u16)> = None;
    for i in 0..count {
        let truncated = || bad(format!("rule {i} is truncated"));
        let fixed_end = at.checked_add(CONJ_RULE_FIXED).ok_or_else(truncated)?;
        let fixed = conj_section.get(at..fixed_end).ok_or_else(truncated)?;
        let base_pos = get_u16(fixed, 0).ok_or_else(truncated)?;
        let target_pos = get_u16(fixed, 2).ok_or_else(truncated)?;
        let (strip_chars, surface_len, reading_len) = (fixed[4], fixed[5], fixed[6]);
        let reading_start = fixed_end + usize::from(surface_len);
        let end = reading_start + usize::from(reading_len);
        let text = |from: usize, to: usize| {
            conj_section
                .get(from..to)
                .and_then(|b| std::str::from_utf8(b).ok())
                .ok_or_else(|| bad(format!("rule {i} has a truncated or non-UTF-8 string")))
        };
        let add_surface = text(fixed_end, reading_start)?;
        let add_reading = text(reading_start, end)?;
        let key = (base_pos, target_pos);
        if u32::from(base_pos) >= pos_count || u32::from(target_pos) >= pos_count {
            return Err(bad(format!("rule {i}: part of speech is out of range")));
        }
        if prev.is_some_and(|p| p >= key) {
            return Err(bad(format!("rule {i}: rules are not strictly ascending")));
        }
        prev = Some(key);
        rules.push(ConjRecord {
            base_pos,
            target_pos,
            strip_chars,
            add_surface,
            add_reading,
        });
        at = end;
    }
    Ok(rules)
}

/// grammar.bin のヘッダとセクション範囲を検証して返す。壊れた入力でもパニックしない。
///
/// O(1) の検証だけを行う（連接表の全体は読まない）。grammar_id の再計算は `verify_grammar_id`。
pub fn parse_grammar_layout(bytes: &[u8]) -> Result<GrammarLayout, FormatError> {
    check_preamble(
        bytes,
        WHAT,
        &GRAMMAR_MAGIC,
        FORMAT_VERSION,
        GRAMMAR_HEADER_LEN,
    )?;
    let r = HeaderReader::new(bytes, WHAT);

    let grammar_id = r.u64(OFF_GRAMMAR_ID)?;
    let prev_right_size = r.u16(OFF_PREV_RIGHT_SIZE)?;
    let next_left_size = r.u16(OFF_NEXT_LEFT_SIZE)?;
    let bos_eos_id = r.u16(OFF_BOS_EOS_ID)?;
    let unk_left_id = r.u16(OFF_UNK_LEFT_ID)?;
    let unk_right_id = r.u16(OFF_UNK_RIGHT_ID)?;
    let unk_pos_id = r.u16(OFF_UNK_POS_ID)?;
    let unk_cost = r.u16(OFF_UNK_COST)? as i16;
    let pos_count = r.u32(OFF_POS_COUNT)?;

    if prev_right_size == 0 || next_left_size == 0 {
        return Err(bad_section("header", "matrix size is zero"));
    }
    if bos_eos_id >= prev_right_size || bos_eos_id >= next_left_size {
        return Err(bad_section("header", "bos_eos_id is out of the matrix"));
    }
    if unk_left_id >= next_left_size || unk_right_id >= prev_right_size {
        return Err(bad_section(
            "header",
            "unknown-word id is out of the matrix",
        ));
    }
    if pos_count as usize > MAX_POS_COUNT || u32::from(unk_pos_id) >= pos_count {
        return Err(bad_section(
            "header",
            "pos_count or unk_pos_id is out of range",
        ));
    }

    let matrix_len = r.u64(OFF_MATRIX_LEN)?;
    let expected = u64::from(prev_right_size) * u64::from(next_left_size) * COST_SIZE as u64;
    if matrix_len != expected {
        return Err(bad_section(
            "matrix",
            format!("length {matrix_len} does not match {prev_right_size} x {next_left_size} x 2"),
        ));
    }
    let matrix = check_section(
        "matrix",
        WHAT,
        bytes.len(),
        r.u64(OFF_MATRIX_OFFSET)?,
        matrix_len,
        GRAMMAR_HEADER_LEN,
    )?;
    let pos = check_section(
        "pos",
        WHAT,
        bytes.len(),
        r.u64(OFF_POS_OFFSET)?,
        r.u64(OFF_POS_LEN)?,
        matrix.end,
    )?;
    let conj = check_section(
        "conj",
        WHAT,
        bytes.len(),
        r.u64(OFF_CONJ_OFFSET)?,
        r.u64(OFF_CONJ_LEN)?,
        pos.end,
    )?;
    if conj.len() < CONJ_COUNT_SIZE {
        return Err(bad_section("conj", "section is shorter than rule_count"));
    }
    let offsets_len = (pos_count as usize + 1) * POS_OFFSET_SIZE;
    if pos.len() < offsets_len {
        return Err(bad_section(
            "pos",
            format!(
                "length {} is shorter than the offset table ({offsets_len})",
                pos.len()
            ),
        ));
    }

    Ok(GrammarLayout {
        grammar_id,
        prev_right_size,
        next_left_size,
        bos_eos_id,
        unk_left_id,
        unk_right_id,
        unk_pos_id,
        unk_cost,
        pos_count,
        matrix,
        pos,
        conj,
    })
}

/// 品詞表セクション（`GrammarLayout::pos` の範囲のスライス）から pos_id のレコードを引く。
/// 範囲外や壊れたレコード（UTF-8 不正、オフセット逆転）なら None。
pub fn read_pos_record(pos_section: &[u8], pos_count: u32, pos_id: u16) -> Option<PosRecord<'_>> {
    if u32::from(pos_id) >= pos_count {
        return None;
    }
    let index = usize::from(pos_id);
    let offsets_len = (pos_count as usize)
        .checked_add(1)?
        .checked_mul(POS_OFFSET_SIZE)?;
    let records = pos_section.get(offsets_len..)?;
    let start = get_u32(pos_section, index * POS_OFFSET_SIZE)? as usize;
    let end = get_u32(pos_section, (index + 1) * POS_OFFSET_SIZE)? as usize;
    if end < start.checked_add(POS_RECORD_FIXED)? {
        return None;
    }
    let record = records.get(start..end)?;
    let rep_left_id = get_u16(record, 0)?;
    let rep_right_id = get_u16(record, 2)?;
    let user_cost = get_u16(record, 4)? as i16;
    let rep_share_permille = get_u16(record, 6)?;
    let name = std::str::from_utf8(record.get(POS_RECORD_FIXED..)?).ok()?;
    Some(PosRecord {
        rep_left_id,
        rep_right_id,
        user_cost,
        rep_share_permille,
        name,
    })
}

/// 連接表の1要素を引く。ID が範囲外（または表が小さすぎる）なら `i16::MAX`。
pub fn matrix_cost(
    matrix_section: &[u8],
    prev_right_size: u16,
    next_left_size: u16,
    prev_right_id: u16,
    next_left_id: u16,
) -> i16 {
    if prev_right_id >= prev_right_size || next_left_id >= next_left_size {
        return i16::MAX;
    }
    let index =
        usize::from(next_left_id) * usize::from(prev_right_size) + usize::from(prev_right_id);
    get_u16(matrix_section, index * COST_SIZE).map_or(i16::MAX, |v| v as i16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pos() -> Vec<PosRecord<'static>> {
        vec![
            PosRecord {
                rep_left_id: 0,
                rep_right_id: 1,
                user_cost: -120,
                rep_share_permille: 1000,
                name: "助詞,格助詞,*,*,*,*",
            },
            PosRecord {
                rep_left_id: 1,
                rep_right_id: 1,
                user_cost: 4500,
                rep_share_permille: 962,
                name: "動詞,一般,*,*,五段-ラ行,終止形-一般",
            },
            PosRecord {
                rep_left_id: 2,
                rep_right_id: 0,
                user_cost: 3000,
                rep_share_permille: 0,
                name: "名詞,普通名詞,一般,*,*,*",
            },
        ]
    }

    /// prev_right_size=2, next_left_size=3 の非正方行列。
    fn sample_spec<'a>(matrix: &'a [i16], pos: &'a [PosRecord<'a>]) -> GrammarSpec<'a> {
        GrammarSpec {
            prev_right_size: 2,
            next_left_size: 3,
            bos_eos_id: 0,
            unk_left_id: 2,
            unk_right_id: 1,
            unk_pos_id: 1,
            unk_cost: 10000,
            matrix,
            pos,
            conj: &SAMPLE_CONJ,
        }
    }

    const SAMPLE_CONJ: [ConjRecord<'static>; 2] = [
        ConjRecord {
            base_pos: 1,
            target_pos: 1,
            strip_chars: 1,
            add_surface: "り",
            add_reading: "り",
        },
        ConjRecord {
            base_pos: 1,
            target_pos: 2,
            strip_chars: 2,
            add_surface: "",
            add_reading: "",
        },
    ];

    const MATRIX: [i16; 6] = [10, -20, 30, 40, -32768, 32767];

    fn encoded() -> Vec<u8> {
        let pos = sample_pos();
        encode_grammar(&sample_spec(&MATRIX, &pos)).unwrap()
    }

    #[test]
    fn roundtrip_all_fields() {
        let bytes = encoded();
        assert_eq!(bytes.len() % 8, 0);
        let layout = parse_grammar_layout(&bytes).unwrap();
        assert_eq!(layout.prev_right_size, 2);
        assert_eq!(layout.next_left_size, 3);
        assert_eq!(layout.bos_eos_id, 0);
        assert_eq!(layout.unk_left_id, 2);
        assert_eq!(layout.unk_right_id, 1);
        assert_eq!(layout.unk_pos_id, 1);
        assert_eq!(layout.unk_cost, 10000);
        assert_eq!(layout.pos_count, 3);
        assert_eq!(layout.matrix.start % 8, 0);
        assert_eq!(layout.pos.start % 8, 0);
        verify_grammar_id(&bytes, &layout).unwrap();

        // matrix[next_left * prev_right_size + prev_right]。転置していないこと。
        let m = &bytes[layout.matrix.clone()];
        for next_left in 0..3u16 {
            for prev_right in 0..2u16 {
                let expected = MATRIX[usize::from(next_left) * 2 + usize::from(prev_right)];
                assert_eq!(matrix_cost(m, 2, 3, prev_right, next_left), expected);
            }
        }

        let p = &bytes[layout.pos.clone()];
        for (i, expected) in sample_pos().iter().enumerate() {
            assert_eq!(
                read_pos_record(p, layout.pos_count, i as u16).as_ref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn matrix_cost_out_of_range_is_max() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        let m = &bytes[layout.matrix];
        assert_eq!(matrix_cost(m, 2, 3, 2, 0), i16::MAX);
        assert_eq!(matrix_cost(m, 2, 3, 0, 3), i16::MAX);
        assert_eq!(matrix_cost(m, 2, 3, u16::MAX, u16::MAX), i16::MAX);
        // 表が実際より小さいスライスでもパニックしない
        assert_eq!(matrix_cost(&m[..4], 2, 3, 1, 2), i16::MAX);
        assert_eq!(matrix_cost(&[], 2, 3, 0, 0), i16::MAX);
    }

    #[test]
    fn read_pos_record_out_of_range_is_none() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        let p = &bytes[layout.pos];
        assert!(read_pos_record(p, 3, 3).is_none());
        assert!(read_pos_record(p, 3, u16::MAX).is_none());
        assert!(read_pos_record(&p[..6], 3, 0).is_none());
        assert!(read_pos_record(&[], 3, 0).is_none());
        // pos_count を実際より大きく偽っても落ちない
        for id in 0..10 {
            let _ = read_pos_record(p, 1000, id);
        }
    }

    #[test]
    fn read_pos_record_rejects_corrupt_records() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        let mut p = bytes[layout.pos.clone()].to_vec();
        // offsets[1] を offsets[2] より後ろにする（逆転）
        p[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_pos_record(&p, 3, 0).is_none());
        assert!(read_pos_record(&p, 3, 1).is_none());
        // UTF-8 不正
        let mut p = bytes[layout.pos].to_vec();
        let last = p.len() - 1;
        p[last] = 0xff;
        assert!(read_pos_record(&p, 3, 2).is_none());
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(encoded(), encoded());
    }

    #[test]
    fn grammar_id_depends_on_contents() {
        let pos = sample_pos();
        let base = parse_grammar_layout(&encoded()).unwrap().grammar_id;
        let mut other_matrix = MATRIX;
        other_matrix[5] = 0;
        let other = encode_grammar(&sample_spec(&other_matrix, &pos)).unwrap();
        assert_ne!(parse_grammar_layout(&other).unwrap().grammar_id, base);

        let mut other_pos = sample_pos();
        other_pos[2].name = "名詞,普通名詞,一般,*,*,二";
        let other = encode_grammar(&sample_spec(&MATRIX, &other_pos)).unwrap();
        assert_ne!(parse_grammar_layout(&other).unwrap().grammar_id, base);

        // unk_cost はハッシュに入らない（§3.3: 連接表と品詞表のバージョンだけ）
        let mut spec = sample_spec(&MATRIX, &pos);
        spec.unk_cost = 1;
        let other = encode_grammar(&spec).unwrap();
        assert_eq!(parse_grammar_layout(&other).unwrap().grammar_id, base);
    }

    #[test]
    fn fnv1a_known_vectors() {
        let hash = |s: &[u8]| {
            let mut h = Fnv1a64::new();
            h.update(s);
            h.finish()
        };
        assert_eq!(hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn verify_grammar_id_detects_tampering() {
        let mut bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        bytes[layout.matrix.start] ^= 1;
        assert!(verify_grammar_id(&bytes, &layout).is_err());
    }

    #[test]
    fn bad_magic_and_version() {
        let mut bytes = encoded();
        bytes[0] = b'X';
        assert!(matches!(
            parse_grammar_layout(&bytes),
            Err(FormatError::BadMagic { .. })
        ));

        let mut bytes = encoded();
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            parse_grammar_layout(&bytes),
            Err(FormatError::UnsupportedVersion { found: 1, .. })
        ));

        let mut bytes = encoded();
        bytes[12..16].copy_from_slice(&80u32.to_le_bytes());
        assert!(matches!(
            parse_grammar_layout(&bytes),
            Err(FormatError::BadHeaderLen { .. })
        ));
    }

    #[test]
    fn every_truncation_is_an_error_without_panic() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        // 末尾のパディングを除いた「実際に使う長さ」より短ければ必ずエラー
        let used = layout.conj.end;
        for len in 0..used {
            assert!(
                parse_grammar_layout(&bytes[..len]).is_err(),
                "len {len} should fail"
            );
        }
        assert!(parse_grammar_layout(&bytes[..used]).is_ok());
    }

    #[test]
    fn out_of_range_offsets_are_errors() {
        let field_cases: [(usize, u64); 6] = [
            (OFF_MATRIX_OFFSET, u64::MAX),
            (OFF_MATRIX_OFFSET, 1_000_000),
            (OFF_MATRIX_OFFSET, 84), // 8 バイト境界でない
            (OFF_POS_OFFSET, u64::MAX - 3),
            (OFF_POS_OFFSET, 8), // ヘッダ・連接表と重なる
            (OFF_POS_LEN, u64::MAX),
        ];
        for (off, value) in field_cases {
            let mut bytes = encoded();
            bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
            assert!(
                parse_grammar_layout(&bytes).is_err(),
                "offset {off} value {value}"
            );
        }
        // matrix_len がサイズと合わない
        let mut bytes = encoded();
        bytes[OFF_MATRIX_LEN..OFF_MATRIX_LEN + 8].copy_from_slice(&8u64.to_le_bytes());
        assert!(parse_grammar_layout(&bytes).is_err());
        // pos_count が大きすぎて offsets が pos セクションに収まらない
        let mut bytes = encoded();
        bytes[OFF_POS_COUNT..OFF_POS_COUNT + 4].copy_from_slice(&60000u32.to_le_bytes());
        assert!(parse_grammar_layout(&bytes).is_err());
        // unk_pos_id が範囲外
        let mut bytes = encoded();
        bytes[OFF_UNK_POS_ID..OFF_UNK_POS_ID + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(parse_grammar_layout(&bytes).is_err());
    }

    #[test]
    fn single_byte_flips_never_panic() {
        let bytes = encoded();
        for i in 0..bytes.len() {
            for mask in [0x01u8, 0x80, 0xff] {
                let mut b = bytes.clone();
                b[i] ^= mask;
                if let Ok(layout) = parse_grammar_layout(&b) {
                    let m = &b[layout.matrix.clone()];
                    let p = &b[layout.pos.clone()];
                    let _ = matrix_cost(m, layout.prev_right_size, layout.next_left_size, 1, 2);
                    for id in 0..layout.pos_count.min(8) {
                        let _ = read_pos_record(p, layout.pos_count, id as u16);
                    }
                    let _ = read_conj_rules(&b[layout.conj.clone()], layout.pos_count);
                    let _ = verify_grammar_id(&b, &layout);
                }
            }
        }
    }

    #[test]
    fn v2_pos_fields_and_conj_rules_roundtrip() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        let p = &bytes[layout.pos.clone()];
        let rec = read_pos_record(p, layout.pos_count, 1).unwrap();
        assert_eq!((rec.user_cost, rec.rep_share_permille), (4500, 962));
        let rec = read_pos_record(p, layout.pos_count, 0).unwrap();
        assert_eq!((rec.user_cost, rec.rep_share_permille), (-120, 1000));

        let rules = read_conj_rules(&bytes[layout.conj.clone()], layout.pos_count).unwrap();
        assert_eq!(rules, SAMPLE_CONJ.to_vec());
        assert_eq!(layout.conj.start % 8, 0);
    }

    #[test]
    fn grammar_id_depends_on_conj_rules() {
        let pos = sample_pos();
        let base = parse_grammar_layout(&encoded()).unwrap().grammar_id;
        let other_conj = [SAMPLE_CONJ[0].clone()];
        let mut spec = sample_spec(&MATRIX, &pos);
        spec.conj = &other_conj;
        let other = encode_grammar(&spec).unwrap();
        assert_ne!(parse_grammar_layout(&other).unwrap().grammar_id, base);

        let mut spec = sample_spec(&MATRIX, &pos);
        spec.conj = &[];
        let empty = encode_grammar(&spec).unwrap();
        let layout = parse_grammar_layout(&empty).unwrap();
        assert_ne!(layout.grammar_id, base);
        verify_grammar_id(&empty, &layout).unwrap();
        assert!(
            read_conj_rules(&empty[layout.conj.clone()], layout.pos_count)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn encode_rejects_invalid_conj_rules() {
        let pos = sample_pos();
        let check = |rules: &[ConjRecord<'_>]| {
            let mut spec = sample_spec(&MATRIX, &pos);
            spec.conj = rules;
            encode_grammar(&spec)
        };
        let rule = |base_pos, target_pos| ConjRecord {
            base_pos,
            target_pos,
            strip_chars: 0,
            add_surface: "",
            add_reading: "",
        };
        assert!(check(&[rule(0, 1), rule(1, 0)]).is_ok());
        assert!(check(&[rule(1, 0), rule(0, 1)]).is_err()); // 昇順でない
        assert!(check(&[rule(1, 1), rule(1, 1)]).is_err()); // 重複
        assert!(check(&[rule(3, 0)]).is_err()); // 品詞が範囲外
        assert!(check(&[rule(0, 3)]).is_err());
        let long = "あ".repeat(100); // 300 バイト > u8::MAX
        let too_long = ConjRecord {
            add_surface: &long,
            ..rule(0, 1)
        };
        assert!(check(&[too_long]).is_err());
    }

    #[test]
    fn read_conj_rules_rejects_corrupt_sections() {
        let bytes = encoded();
        let layout = parse_grammar_layout(&bytes).unwrap();
        let section = bytes[layout.conj.clone()].to_vec();
        assert!(read_conj_rules(&section, layout.pos_count).is_ok());
        // 件数が巨大
        let mut broken = section.clone();
        broken[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_conj_rules(&broken, layout.pos_count).is_err());
        // 途中で切れている
        for len in 0..section.len() {
            let _ = read_conj_rules(&section[..len], layout.pos_count);
        }
        assert!(read_conj_rules(&section[..8], layout.pos_count).is_err());
        // 品詞が範囲外
        assert!(read_conj_rules(&section, 1).is_err());
        // UTF-8 不正（add_surface の先頭バイト）
        let mut broken = section.clone();
        broken[4 + CONJ_RULE_FIXED] = 0xff;
        assert!(read_conj_rules(&broken, layout.pos_count).is_err());
        // 全バイトの反転でパニックしない
        for i in 0..section.len() {
            let mut b = section.clone();
            b[i] ^= 0xff;
            let _ = read_conj_rules(&b, layout.pos_count);
        }
    }

    #[test]
    fn v1_files_are_rejected() {
        let mut bytes = encoded();
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            parse_grammar_layout(&bytes),
            Err(FormatError::UnsupportedVersion {
                found: 1,
                expected: 2
            })
        ));
    }

    #[test]
    fn encode_rejects_invalid_specs() {
        let pos = sample_pos();
        let mut spec = sample_spec(&MATRIX, &pos);
        spec.matrix = &MATRIX[..5];
        assert!(matches!(
            encode_grammar(&spec),
            Err(FormatError::InvalidInput(_))
        ));

        let mut spec = sample_spec(&MATRIX, &pos);
        spec.unk_pos_id = 3;
        assert!(encode_grammar(&spec).is_err());

        let mut spec = sample_spec(&MATRIX, &pos);
        spec.bos_eos_id = 2; // prev_right_size=2 の範囲外
        assert!(encode_grammar(&spec).is_err());

        let mut spec = sample_spec(&MATRIX, &pos);
        spec.prev_right_size = 0;
        assert!(encode_grammar(&spec).is_err());

        // 品詞名が昇順でない／重複
        let mut unsorted = sample_pos();
        unsorted.swap(0, 1);
        assert!(encode_grammar(&sample_spec(&MATRIX, &unsorted)).is_err());
        let mut dup = sample_pos();
        dup[1].name = dup[0].name;
        assert!(encode_grammar(&sample_spec(&MATRIX, &dup)).is_err());

        // 代表 ID が範囲外
        let mut bad_rep = sample_pos();
        bad_rep[0].rep_right_id = 2; // prev_right_size=2 の範囲外
        assert!(encode_grammar(&sample_spec(&MATRIX, &bad_rep)).is_err());
    }
}
