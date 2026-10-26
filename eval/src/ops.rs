//! 打鍵数のシミュレーション（ADR 0006 §7.1-3、`ikari-eval ops`）。
//!
//! 読みを入力し終えた状態から、目標の表記を確定するまでの最少の打鍵数を、今の UI と文節の UI の両方で数える。
//!
//! - **今の UI**: Space で変換し、読み全体の候補を Space でたどる。表示中のページの番号キー（1〜`page_size`）で選んで確定できる。
//! - **文節の UI**: 上に加えて、←/→（文節の移動）、Shift+←/→（区切り直し）。最初の Space は、今の UI と同じ読み全体の候補を出す
//!   （ADR 0006 §16.6）。ここでは番号キーで全体を確定できる。←/→ か Shift+←/→ を押したときだけ文節のモードに入る
//!   （最初の区切りが 1 文節のときは、最初から文節のモード）。文節のモードの番号キーは、
//!   フォーカス中の文節の候補を選んで次の文節へ進むだけで（§16.1-3）、確定は Enter。
//!   区切り直しの前後で、直した文節より左の選択は残り、直した文節以降の選択は 0 番に戻る（§4.2）。
//!
//! 全体の候補から文節のモードに入る規則（殻の `enterSegmentMode` と同じ。§16.6）:
//! - →: 先頭の文節にフォーカスがあるものとして 2 番目の文節へ。←: 先頭の文節にフォーカス。入るのに 1 打鍵かかる。
//!   いま選んでいる全体の候補の表記は変えない。表記が最初の区切りの文節の候補（各文節の最初のページ）の組み合わせで
//!   作れるときだけ入れて、その選択を引き継ぐ（順位が小さい組を優先）。作れなければ入れない
//! - Shift+←/→: 先頭の文節の区切り直し。選択はすべて 0 番に戻るので、全体の候補の選択は捨てる
//!
//! 状態（区切り、フォーカス、各文節の選択）を幅優先でたどる。
//!
//! 近似（レポートに書く）: 文節の候補の列は、区切りごとに「左の文節はすべて 0 番を選んだ」文脈で一度だけ作る。
//! 実際は左の選択が左文脈に入るが、`Segmented` は最初に作った列を固定するので、この近似で差が出るのは
//! 左の選択を変えてから初めて開く文節だけ。Shift+Space（前の候補）とページの折り返しは使わない。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use ikari_core::SegmentChoice;

use crate::segment_engine::SegmentEngine;
use crate::segments::boundaries_of;
use crate::{EvalCase, SegmentCase, Split};

/// 変換を始める Space と、確定の Enter（または番号キー）
const KEYS_CONVERT: usize = 1;
const KEYS_COMMIT: usize = 1;

/// シミュレーションの設定
#[derive(Clone, Debug)]
pub struct OpsConfig {
    /// 各文節で、この順位までの候補を使う（ADR 0006 §7.1 の `max_rank`）
    pub max_rank: usize,
    /// 候補ウィンドウの 1 ページの候補数（`ConvertOptions.page_size`）
    pub page_size: usize,
    /// 文節の UI の探索の打鍵数の上限。これを超える目標は「届かない」とする
    pub max_keys: usize,
    /// 探索する状態の数の上限（暴走の保護）
    pub max_states: usize,
}

impl Default for OpsConfig {
    fn default() -> Self {
        Self {
            max_rank: 50,
            page_size: 9,
            max_keys: 40,
            max_states: 200_000,
        }
    }
}

/// 打鍵数を数える 1 ケース
#[derive(Clone, Debug)]
pub struct OpsCase {
    pub id: String,
    pub kind: String,
    pub split: Option<Split>,
    pub context: String,
    pub reading: String,
    /// 目標の表記（どれか 1 つに届けばよい）
    pub goals: Vec<String>,
}

impl From<&SegmentCase> for OpsCase {
    fn from(c: &SegmentCase) -> Self {
        OpsCase {
            id: c.case.id.clone(),
            kind: c.case.kind.clone(),
            split: c.case.split,
            context: c.case.context.clone(),
            reading: c.case.reading.clone(),
            goals: vec![c.goal().to_owned()],
        }
    }
}

impl From<&EvalCase> for OpsCase {
    fn from(c: &EvalCase) -> Self {
        OpsCase {
            id: c.id.clone(),
            kind: c.kind.clone(),
            split: c.split,
            context: c.context.clone(),
            reading: c.reading.clone(),
            goals: c.alternatives().into_iter().map(str::to_owned).collect(),
        }
    }
}

/// 評価セットを読む。`expected_segments` 列があれば segments.tsv の形式（`target` を目標にする）、
/// なければ既存のセットの形式（`expected` の別解のどれかを目標にする）
pub fn load_ops_cases(tsv: &str) -> Result<Vec<OpsCase>, crate::EvalSetError> {
    let header = tsv
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .unwrap_or_default();
    if header.split('\t').any(|c| c.trim() == "expected_segments") {
        Ok(crate::load_segment_cases(tsv)?
            .iter()
            .map(OpsCase::from)
            .collect())
    } else {
        Ok(crate::load_cases(tsv)?.iter().map(OpsCase::from).collect())
    }
}

/// 1 ケースの結果。届かなければ None
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsResult {
    pub id: String,
    pub kind: String,
    pub current: Option<usize>,
    pub segmented: Option<usize>,
}

/// 今の UI の打鍵数。`rank0` は読み全体の候補の列の中の位置（0 始まり）。
/// Space で変換（1 打鍵）してから、目標のあるページの先頭まで Space で進み、番号キーで選んで確定する
pub fn current_ui_keys(rank0: usize, page_size: usize) -> usize {
    let page = rank0 / page_size;
    KEYS_CONVERT + page * page_size + KEYS_COMMIT
}

/// 今の UI の打鍵数。読み全体の候補の列に目標がなければ None
pub fn current_keys_for<E: SegmentEngine>(
    engine: &E,
    case: &OpsCase,
    cfg: &OpsConfig,
) -> Option<usize> {
    let mut session = engine.open(&case.reading, &case.context).ok()?;
    let list = engine.whole_surfaces(&mut session, cfg.max_rank);
    let rank0 = list.iter().position(|s| case.goals.contains(s))?;
    Some(current_ui_keys(rank0, cfg.page_size))
}

/// 探索の状態
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct State {
    /// 区切り（境界の読みの文字位置）。文節の数は `layout.len() + 1`
    layout: Vec<usize>,
    focus: usize,
    /// 各文節の選択中の候補の位置
    selected: Vec<usize>,
}

/// 区切りごとの作業（セッションと、そこまでの区切り直しの手順、作った候補の列）
struct LayoutNode<S> {
    session: S,
    path: Vec<(usize, i32)>,
    lists: Vec<Option<Vec<String>>>,
}

struct Simulator<'a, E: SegmentEngine> {
    engine: &'a E,
    case: &'a OpsCase,
    cfg: &'a OpsConfig,
    nodes: HashMap<Vec<usize>, LayoutNode<E::Session>>,
    /// (区切り, フォーカス, delta) → 区切り直しのあとの区切り（変わらなければ None）
    transitions: HashMap<(Vec<usize>, usize, i32), Option<Vec<usize>>>,
}

/// 左の文節はすべて 0 番を選んだものとして渡す（モジュールの注意を参照）
fn zero_left(seg: usize) -> Vec<SegmentChoice> {
    vec![SegmentChoice::Candidate(0); seg]
}

impl<'a, E: SegmentEngine> Simulator<'a, E> {
    fn new(engine: &'a E, case: &'a OpsCase, cfg: &'a OpsConfig) -> Option<(Self, Vec<usize>)> {
        let session = engine.open(&case.reading, &case.context).ok()?;
        let key = layout_key(engine, &session);
        let mut nodes = HashMap::new();
        nodes.insert(key.clone(), new_node(session, Vec::new(), key.len() + 1));
        Some((
            Self {
                engine,
                case,
                cfg,
                nodes,
                transitions: HashMap::new(),
            },
            key,
        ))
    }

    /// 区切り `layout` の文節 `seg` の候補の列（`max_rank` まで）
    fn list(&mut self, layout: &[usize], seg: usize) -> &[String] {
        let node = self.nodes.get_mut(layout).expect("layout node exists");
        if node.lists[seg].is_none() {
            let list = self
                .engine
                .surfaces(&mut node.session, seg, &zero_left(seg), self.cfg.max_rank)
                .unwrap_or_default();
            node.lists[seg] = Some(list);
        }
        node.lists[seg].as_deref().unwrap_or_default()
    }

    /// いま表示している文字列（各文節の選択中の表記をつないだもの）
    fn surface(&mut self, st: &State) -> Option<String> {
        let mut out = String::new();
        for (seg, &sel) in st.selected.iter().enumerate() {
            out.push_str(self.list(&st.layout, seg).get(sel)?);
        }
        Some(out)
    }

    fn is_goal(&self, surface: &str) -> bool {
        self.case.goals.iter().any(|g| g == surface)
    }

    /// 区切り直しのあとの区切り。手順を最初から作り直して `resize` を呼ぶ（`Segmented` は複製できないため）
    fn resized(&mut self, layout: &[usize], focus: usize, delta: i32) -> Option<Vec<usize>> {
        let cache_key = (layout.to_vec(), focus, delta);
        if let Some(t) = self.transitions.get(&cache_key) {
            return t.clone();
        }
        let result = self.compute_resize(layout, focus, delta);
        self.transitions.insert(cache_key, result.clone());
        result
    }

    fn compute_resize(&mut self, layout: &[usize], focus: usize, delta: i32) -> Option<Vec<usize>> {
        let mut path = self.nodes.get(layout)?.path.clone();
        let mut session = self
            .engine
            .open(&self.case.reading, &self.case.context)
            .ok()?;
        for &(index, d) in &path {
            self.engine
                .resize(&mut session, index, d, &zero_left(index))
                .ok()?;
        }
        self.engine
            .resize(&mut session, focus, delta, &zero_left(focus))
            .ok()?;
        let key = layout_key(self.engine, &session);
        if key == layout {
            return None;
        }
        path.push((focus, delta));
        let segs = key.len() + 1;
        self.nodes
            .entry(key.clone())
            .or_insert_with(|| new_node(session, path, segs));
        Some(key)
    }

    /// 最少の打鍵数。`limit` 打鍵までに届かなければ None
    fn search(&mut self, start_layout: Vec<usize>, limit: usize) -> Option<usize> {
        let n = start_layout.len() + 1;
        // 打鍵数ごとの、まだ調べていない状態（打鍵数は変換の Space を含む）
        let mut frontier: BTreeMap<usize, Vec<State>> = BTreeMap::new();
        let mut limit = limit;
        let mut direct = None;
        if n == 1 {
            let start = State {
                layout: start_layout,
                focus: 0,
                selected: vec![0],
            };
            frontier.entry(KEYS_CONVERT).or_default().push(start);
        } else {
            // 2 文節以上: 最初は読み全体の候補（今の UI と同じ）。番号キーで確定できる
            let whole = {
                let node = self.nodes.get_mut(&start_layout)?;
                self.engine
                    .whole_surfaces(&mut node.session, self.cfg.max_rank)
            };
            direct = whole
                .iter()
                .position(|s| self.is_goal(s))
                .map(|rank0| current_ui_keys(rank0, self.cfg.page_size));
            if let Some(d) = direct {
                limit = limit.min(d.saturating_sub(1));
            }
            self.seed_segment_mode(&start_layout, &whole, limit, &mut frontier);
        }
        let found = self.search_segment_mode(frontier, limit);
        match (direct, found) {
            (Some(d), Some(f)) => Some(d.min(f)),
            (d, f) => d.or(f),
        }
    }

    /// 全体の候補から文節のモードに入る状態（入る打鍵を含む）を `frontier` に足す
    fn seed_segment_mode(
        &mut self,
        layout: &[usize],
        whole: &[String],
        limit: usize,
        frontier: &mut BTreeMap<usize, Vec<State>>,
    ) {
        let n = layout.len() + 1;
        // 殻は各文節の最初のページ（page_size 件）の候補から組み合わせを探す
        let page_size = self.cfg.page_size;
        let first_pages: Vec<Vec<String>> = (0..n)
            .map(|seg| {
                self.list(layout, seg)
                    .iter()
                    .take(page_size)
                    .cloned()
                    .collect()
            })
            .collect();
        // k 番目の全体の候補まで Space（変換の Space + k）、そこで文節の操作のキーを 1 打鍵
        for (k, shown) in whole.iter().enumerate() {
            let keys = KEYS_CONVERT + k + 1;
            if keys + KEYS_COMMIT > limit {
                break;
            }
            let Some(picks) = decompose(&first_pages, shown) else {
                continue;
            };
            for focus in [0, 1] {
                frontier.entry(keys).or_default().push(State {
                    layout: layout.to_vec(),
                    focus,
                    selected: picks.clone(),
                });
            }
        }
        // Shift+←/→: 先頭の文節の区切り直し。選択は捨てるので、いちばん早い k = 0 だけでよい
        let keys = KEYS_CONVERT + 1;
        for delta in [-1, 1] {
            if let Some(resized) = self.resized(layout, 0, delta) {
                let selected = vec![0; resized.len() + 1];
                frontier.entry(keys).or_default().push(State {
                    layout: resized,
                    focus: 0,
                    selected,
                });
            }
        }
    }

    /// 文節のモードの探索。`frontier` の状態を打鍵数の小さい順に調べる
    fn search_segment_mode(
        &mut self,
        mut frontier: BTreeMap<usize, Vec<State>>,
        limit: usize,
    ) -> Option<usize> {
        let mut seen: HashSet<State> = HashSet::new();
        while let Some((keys, bucket)) = frontier.pop_first() {
            if keys + KEYS_COMMIT > limit {
                break; // 打鍵数の順に調べているので、以降はもっと多い
            }
            for st in bucket {
                if !seen.insert(st.clone()) {
                    continue;
                }
                if let Some(total) = self.expand(&st, keys, &mut frontier) {
                    return Some(total);
                }
                if seen.len() > self.cfg.max_states {
                    return None;
                }
            }
        }
        None
    }

    /// 状態 1 つを調べる。目標に届けば確定までの打鍵数を返す。届かなければ次の状態を `frontier` に足す
    fn expand(
        &mut self,
        st: &State,
        keys: usize,
        frontier: &mut BTreeMap<usize, Vec<State>>,
    ) -> Option<usize> {
        {
            if self.surface(st).is_some_and(|s| self.is_goal(&s)) {
                return Some(keys + KEYS_COMMIT);
            }
            let segs = st.layout.len() + 1;
            // 番号キー: 1 文節なら選んで確定（打鍵数 +1）、2 文節以上なら選んで次の文節へ
            let page_start = st.selected[st.focus] / self.cfg.page_size * self.cfg.page_size;
            let list_len = self.list(&st.layout, st.focus).len();
            for digit in 0..self.cfg.page_size {
                let idx = page_start + digit;
                if idx >= list_len {
                    break;
                }
                let mut next = st.clone();
                next.selected[st.focus] = idx;
                if segs == 1 {
                    if self.surface(&next).is_some_and(|s| self.is_goal(&s)) {
                        return Some(keys + KEYS_COMMIT);
                    }
                } else if idx != st.selected[st.focus] || st.focus + 1 < segs {
                    next.focus = (st.focus + 1).min(segs - 1);
                    Self::push(frontier, next, keys + 1);
                }
            }
            // Space: 次の候補
            if st.selected[st.focus] + 1 < list_len {
                let mut next = st.clone();
                next.selected[st.focus] += 1;
                Self::push(frontier, next, keys + 1);
            }
            // ←/→: 文節の移動
            if st.focus > 0 {
                let mut next = st.clone();
                next.focus -= 1;
                Self::push(frontier, next, keys + 1);
            }
            if st.focus + 1 < segs {
                let mut next = st.clone();
                next.focus += 1;
                Self::push(frontier, next, keys + 1);
            }
            // Shift+←/→: 区切り直し。直した文節以降の選択は 0 番に戻る
            for delta in [-1, 1] {
                if let Some(layout) = self.resized(&st.layout, st.focus, delta) {
                    let mut selected = st.selected[..st.focus.min(layout.len() + 1)].to_vec();
                    selected.resize(layout.len() + 1, 0);
                    let next = State {
                        layout,
                        focus: st.focus,
                        selected,
                    };
                    Self::push(frontier, next, keys + 1);
                }
            }
        }
        None
    }

    fn push(frontier: &mut BTreeMap<usize, Vec<State>>, state: State, keys: usize) {
        frontier.entry(keys).or_default().push(state);
    }
}

/// 各文節の候補（`lists[i]`）から 1 つずつ選んでつなぐと `surface` になる組み合わせ（候補の位置の列）。
/// 順位の小さいものを優先する（辞書順で最小）。作れなければ None
fn decompose(lists: &[Vec<String>], surface: &str) -> Option<Vec<usize>> {
    fn search(lists: &[Vec<String>], rest: &str, picks: &mut Vec<usize>) -> bool {
        let Some((first, others)) = lists.split_first() else {
            return rest.is_empty();
        };
        for (index, candidate) in first.iter().enumerate() {
            if let Some(tail) = rest.strip_prefix(candidate.as_str()) {
                picks.push(index);
                if search(others, tail, picks) {
                    return true;
                }
                picks.pop();
            }
        }
        false
    }
    let mut picks = Vec::new();
    search(lists, surface, &mut picks).then_some(picks)
}

fn new_node<S>(session: S, path: Vec<(usize, i32)>, segs: usize) -> LayoutNode<S> {
    LayoutNode {
        session,
        path,
        lists: vec![None; segs],
    }
}

fn layout_key<E: SegmentEngine>(engine: &E, session: &E::Session) -> Vec<usize> {
    boundaries_of(
        engine
            .layout(session)
            .spans
            .iter()
            .map(|s| s.reading.as_str()),
    )
}

/// 文節の UI の打鍵数。`cfg.max_keys` 打鍵までに届かなければ None
pub fn segmented_keys_for<E: SegmentEngine>(
    engine: &E,
    case: &OpsCase,
    cfg: &OpsConfig,
) -> Option<usize> {
    let (mut sim, start) = Simulator::new(engine, case, cfg)?;
    sim.search(start, cfg.max_keys)
}

/// 全ケースの打鍵数を数える
pub fn evaluate_ops<E: SegmentEngine>(
    engine: &E,
    cases: &[OpsCase],
    cfg: &OpsConfig,
) -> Vec<OpsResult> {
    cases
        .iter()
        .map(|c| OpsResult {
            id: c.id.clone(),
            kind: c.kind.clone(),
            current: current_keys_for(engine, c, cfg),
            segmented: segmented_keys_for(engine, c, cfg),
        })
        .collect()
}

/// 集計 1 行分。平均は「両方の UI で届いたケース」だけで取る（同じ集合で比べるため）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpsGroup {
    pub cases: usize,
    pub both: usize,
    pub mean_current: f64,
    pub mean_segmented: f64,
    pub better: usize,
    pub worse: usize,
    /// 今の UI では候補の列に出ない（max_rank まで）が、文節の UI で届いた
    pub only_segmented: usize,
    /// 今の UI で届くのに、文節の UI で届かなかった
    pub only_current: usize,
    /// どちらでも届かなかった
    pub neither: usize,
}

fn group_of<'a>(results: impl Iterator<Item = &'a OpsResult>) -> OpsGroup {
    let mut g = OpsGroup::default();
    let (mut sum_c, mut sum_s) = (0usize, 0usize);
    for r in results {
        g.cases += 1;
        match (r.current, r.segmented) {
            (Some(c), Some(s)) => {
                g.both += 1;
                sum_c += c;
                sum_s += s;
                g.better += usize::from(s < c);
                g.worse += usize::from(s > c);
            }
            (None, Some(_)) => g.only_segmented += 1,
            (Some(_), None) => g.only_current += 1,
            (None, None) => g.neither += 1,
        }
    }
    if g.both > 0 {
        g.mean_current = sum_c as f64 / g.both as f64;
        g.mean_segmented = sum_s as f64 / g.both as f64;
    }
    g
}

/// 種別ごとと全体の集計
pub fn summarize_ops(results: &[OpsResult]) -> (OpsGroup, BTreeMap<String, OpsGroup>) {
    let mut kinds: BTreeMap<String, Vec<&OpsResult>> = BTreeMap::new();
    for r in results {
        kinds.entry(r.kind.clone()).or_default().push(r);
    }
    (
        group_of(results.iter()),
        kinds
            .into_iter()
            .map(|(k, v)| (k, group_of(v.into_iter())))
            .collect(),
    )
}

/// 文節の UI のほうが多くかかった（または届かなくなった）ケース
pub fn worse_cases(results: &[OpsResult]) -> Vec<&OpsResult> {
    results
        .iter()
        .filter(|r| match (r.current, r.segmented) {
            (Some(c), Some(s)) => s > c,
            (Some(_), None) => true,
            _ => false,
        })
        .collect()
}

fn show(keys: Option<usize>) -> String {
    keys.map_or("-".to_owned(), |k| k.to_string())
}

/// Markdown の表と、悪くなったケースの一覧
pub fn render_ops(results: &[OpsResult]) -> String {
    let (all, by_kind) = summarize_ops(results);
    let mut out = String::new();
    out.push_str("## 打鍵数（読みを入力し終えてから確定まで）\n\n");
    out.push_str(
        "| 種別 | 件数 | 両方で届く | 今の UI の平均 | 文節の UI の平均 | 少ない | 多い | 文節だけで届く | 今だけで届く | どちらも届かない |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    let row = |out: &mut String, name: &str, g: &OpsGroup| {
        let _ = writeln!(
            out,
            "| {name} | {} | {} | {:.4} | {:.4} | {} | {} | {} | {} | {} |",
            g.cases,
            g.both,
            g.mean_current,
            g.mean_segmented,
            g.better,
            g.worse,
            g.only_segmented,
            g.only_current,
            g.neither
        );
    };
    for (kind, g) in &by_kind {
        row(&mut out, kind, g);
    }
    row(&mut out, "全体", &all);

    let worse = worse_cases(results);
    if !worse.is_empty() {
        out.push_str("\n### 文節の UI のほうが打鍵が多い（または届かない）ケース\n\n| id | 今の UI | 文節の UI |\n|---|---:|---:|\n");
        for r in worse {
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                r.id,
                show(r.current),
                show(r.segmented)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment_engine::mock::MockEngine;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn case(reading: &str, goal: &str) -> OpsCase {
        OpsCase {
            id: "t".into(),
            kind: "伸縮".into(),
            split: None,
            context: String::new(),
            reading: reading.into(),
            goals: vec![goal.into()],
        }
    }

    /// 「りぽじとりめい」を 1 文節で出す。「りぽじとり」「めい」に区切ると目標の候補がある
    fn repo_engine() -> MockEngine {
        let mut e = MockEngine::default();
        e.initial
            .insert("りぽじとりめい".into(), strings(&["りぽじとりめい"]));
        e.candidates.insert(
            "りぽじとりめい".into(),
            strings(&["リポジトリ名", "リポジトリメイ"]),
        );
        e.candidates
            .insert("りぽじとりめ".into(), strings(&["リポジトリ目"]));
        e.candidates
            .insert("りぽじとり".into(), strings(&["リポジトリ", "りぽじとり"]));
        e.candidates
            .insert("めい".into(), strings(&["名", "命", "銘", "メイ"]));
        e.candidates.insert("い".into(), strings(&["意", "胃"]));
        e.whole.insert(
            "りぽじとりめい".into(),
            strings(&["リポジトリ名", "リポジトリメイ"]),
        );
        e
    }

    /// 「あいう」を あい｜う の 2 文節で出す。全体の候補の 1 番（哀卯）は文節の候補の組み合わせで作れ、2 番（未知）は作れない
    fn two_segment_engine() -> MockEngine {
        let mut e = MockEngine::default();
        e.initial.insert("あいう".into(), strings(&["あい", "う"]));
        e.candidates
            .insert("あい".into(), strings(&["愛", "哀", "藍"]));
        e.candidates
            .insert("う".into(), strings(&["卯", "宇", "鵜"]));
        e.whole.insert(
            "あいう".into(),
            strings(&["愛卯", "哀卯", "未知", "愛宇宙"]),
        );
        e
    }

    #[test]
    fn the_first_space_shows_the_whole_candidates_even_with_two_segments() {
        let e = two_segment_engine();
        let cfg = OpsConfig::default();
        // 全体の候補の 0 番、1 番、3 番: 今の UI と同じ 2 打鍵（番号キーで確定できる）
        for goal in ["愛卯", "哀卯", "愛宇宙"] {
            let c = case("あいう", goal);
            assert_eq!(current_keys_for(&e, &c, &cfg), Some(2), "{goal}");
            assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(2), "{goal}");
        }
    }

    #[test]
    fn entering_the_segment_mode_costs_one_key() {
        let e = two_segment_engine();
        let cfg = OpsConfig::default();
        // 全体の候補にない。Space、←（入る）、2（哀を選んで次へ）、3（鵜）、Enter
        let c = case("あいう", "哀鵜");
        assert_eq!(current_keys_for(&e, &c, &cfg), None);
        assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(5));
        // 右の文節だけ: Space、→（入って 2 番目へ）、3（鵜）、Enter
        let c = case("あいう", "愛鵜");
        assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(4));
    }

    #[test]
    fn a_whole_selection_the_segments_can_form_is_carried_in() {
        let e = two_segment_engine();
        let cfg = OpsConfig::default();
        // Space、Space（哀卯 = 哀 + 卯）、→（入って 2 番目へ。哀は引き継ぐ）、2（宇）、Enter
        let c = case("あいう", "哀宇");
        assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(5));
        // 作れない全体の候補（未知）からは入れない。入る経路は 0 番か 1 番から
        let lists = vec![strings(&["愛", "哀"]), strings(&["卯", "宇"])];
        assert_eq!(decompose(&lists, "哀卯"), Some(vec![1, 0]));
        assert_eq!(decompose(&lists, "未知"), None);
        assert_eq!(decompose(&lists, "哀"), None, "all segments are required");
        assert_eq!(decompose(&lists, "愛卯卯"), None, "no leftovers");
    }

    #[test]
    fn decompose_prefers_the_lowest_ranks() {
        let lists = vec![strings(&["あ", "あい"]), strings(&["いう", "う"])];
        // あ + いう と あい + う の両方で作れる: 先頭の文節の順位が小さいほう
        assert_eq!(decompose(&lists, "あいう"), Some(vec![0, 0]));
    }

    #[test]
    fn current_ui_cost_follows_the_page() {
        // Space で変換して、同じページの番号キーで確定: 2 打鍵
        assert_eq!(current_ui_keys(0, 9), 2);
        assert_eq!(current_ui_keys(8, 9), 2);
        // 2 ページ目: ページの先頭まで 9 回の Space
        assert_eq!(current_ui_keys(9, 9), 11);
        assert_eq!(current_ui_keys(17, 9), 11);
        assert_eq!(current_ui_keys(18, 9), 20);
    }

    #[test]
    fn default_candidate_costs_two_keys_in_both_uis() {
        let e = repo_engine();
        let c = case("りぽじとりめい", "リポジトリ名");
        let cfg = OpsConfig::default();
        assert_eq!(current_keys_for(&e, &c, &cfg), Some(2));
        assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(2));
    }

    #[test]
    fn resizing_reaches_a_partial_target_the_whole_list_lacks() {
        let e = repo_engine();
        let c = case("りぽじとりめい", "リポジトリ銘");
        let cfg = OpsConfig::default();
        assert_eq!(
            current_keys_for(&e, &c, &cfg),
            None,
            "not in the whole list"
        );
        // Space、Shift+← ×2（りぽじとり｜めい）、→、番号 3（銘を選んで最後の文節に留まる）、Enter
        assert_eq!(segmented_keys_for(&e, &c, &cfg), Some(6));
        assert!(e.resizes.get() > 0);
    }

    #[test]
    fn the_whole_list_wins_when_it_has_the_target() {
        let mut e = repo_engine();
        e.whole.insert(
            "りぽじとりめい".into(),
            strings(&["リポジトリ名", "リポジトリ銘"]),
        );
        e.candidates.insert(
            "りぽじとりめい".into(),
            strings(&["リポジトリ名", "リポジトリ銘"]),
        );
        let c = case("りぽじとりめい", "リポジトリ銘");
        let r = evaluate_ops(&e, &[c], &OpsConfig::default());
        // どちらも番号キーで届く。文節の UI は同じ候補の列を使える（1 文節の候補は全体の候補そのもの）
        assert_eq!((r[0].current, r[0].segmented), (Some(2), Some(2)));
    }

    #[test]
    fn unreachable_targets_and_failures_are_none() {
        let e = repo_engine();
        let cfg = OpsConfig::default();
        let r = evaluate_ops(
            &e,
            &[case("りぽじとりめい", "存在しない"), case("みちの", "x")],
            &cfg,
        );
        assert_eq!((r[0].current, r[0].segmented), (None, None));
        assert_eq!((r[1].current, r[1].segmented), (None, None), "open fails");
    }

    #[test]
    fn page_keys_apply_only_to_the_displayed_page() {
        let mut e = MockEngine::default();
        e.initial.insert("あ".into(), strings(&["あ"]));
        let many: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
        e.candidates.insert("あ".into(), many.clone());
        e.whole.insert("あ".into(), many);
        let cfg = OpsConfig::default();
        let at = |goal: &str| {
            let c = case("あ", goal);
            (
                current_keys_for(&e, &c, &cfg),
                segmented_keys_for(&e, &c, &cfg),
            )
        };
        assert_eq!(at("c8"), (Some(2), Some(2)));
        assert_eq!(at("c9"), (Some(11), Some(11)));
        assert_eq!(at("c20"), (Some(20), Some(20)));
    }

    #[test]
    fn summary_pairs_cases_and_lists_the_worse_ones() {
        let rs = vec![
            OpsResult {
                id: "a".into(),
                kind: "部分".into(),
                current: Some(5),
                segmented: Some(3),
            },
            OpsResult {
                id: "b".into(),
                kind: "部分".into(),
                current: Some(2),
                segmented: Some(4),
            },
            OpsResult {
                id: "c".into(),
                kind: "伸縮".into(),
                current: None,
                segmented: Some(6),
            },
            OpsResult {
                id: "d".into(),
                kind: "伸縮".into(),
                current: Some(2),
                segmented: None,
            },
            OpsResult {
                id: "e".into(),
                kind: "伸縮".into(),
                current: None,
                segmented: None,
            },
        ];
        let (all, by_kind) = summarize_ops(&rs);
        assert_eq!((all.cases, all.both), (5, 2));
        assert!((all.mean_current - 3.5).abs() < 1e-9);
        assert!((all.mean_segmented - 3.5).abs() < 1e-9);
        assert_eq!((all.better, all.worse), (1, 1));
        assert_eq!(
            (all.only_segmented, all.only_current, all.neither),
            (1, 1, 1)
        );
        assert_eq!(by_kind["部分"].cases, 2);
        let ids: Vec<&str> = worse_cases(&rs).iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["b", "d"]);
        let md = render_ops(&rs);
        assert!(
            md.contains("| 全体 | 5 | 2 | 3.5000 | 3.5000 | 1 | 1 | 1 | 1 | 1 |"),
            "{md}"
        );
        assert!(md.contains("| d | 2 | - |"), "{md}");
    }

    #[test]
    fn loads_both_set_formats() {
        let seg = load_ops_cases(
            "id\tkind\tcontext\treading\texpected\texpected_segments\ttarget\n\
             a\t伸縮\t\tりぽじとりめい\tリポジトリ名\tりぽじとりめい\tリポジトリ銘\n\
             b\t区切り\t\tはしる\t走る\tはしる\t\n",
        )
        .unwrap();
        assert_eq!(seg[0].goals, ["リポジトリ銘"]);
        assert_eq!(seg[1].goals, ["走る"], "empty target -> expected");
        let plain =
            load_ops_cases("id\tkind\tcontext\treading\texpected\nx\t話題\t\tひんし\t品詞|瀕死\n")
                .unwrap();
        assert_eq!(plain[0].goals, ["品詞", "瀕死"]);
        let bundled = load_ops_cases(include_str!("../sets/segments.tsv")).unwrap();
        assert!(bundled.len() >= 120);
    }

    #[test]
    fn the_search_respects_the_key_limit() {
        let e = repo_engine();
        let c = case("りぽじとりめい", "リポジトリ銘");
        let cfg = OpsConfig {
            max_keys: 5,
            ..OpsConfig::default()
        };
        assert_eq!(segmented_keys_for(&e, &c, &cfg), None);
    }
}
