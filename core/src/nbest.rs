//! 後ろ向き A* による N-best（crate 内部。ADR 0001 §4.6-5）。
//!
//! ヒューリスティックは前向き Viterbi の `Node::best`（BOS からの正確な最小コスト）なので、
//! パスはコストの昇順に取り出せる。状態を保持するので、`next` を呼ぶたびに続きから再開できる。
//! ノードの間の辺には、前向きと同じ `Lattice::edge_extra` を足す（ADR 0008 §3.1。ヒューリスティックが正確なまま）。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::dict::Grammar;
use crate::lattice::Lattice;

/// 経路の連結リストのアリーナ上の添字
type StepId = u32;
const NO_STEP: StepId = StepId::MAX;

/// 後ろ向き探索の1歩。`node` の次（読みの後ろ側）が `next`。
struct Step {
    node: u32,
    next: StepId,
}

/// 優先度付きキューの要素。`f` が同じなら先に積んだものを先に出す（決定的にするため）。
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct State {
    /// best(node) + g（このノードを通る最良パスの総コスト）
    f: i32,
    seq: u64,
    step: StepId,
    /// node の右側（node への連接、以降の単語、EOS）のコスト。node 自身の単語コストは含まない
    g: i32,
}

/// 取り出した1本のパス。`nodes` は読みの先頭から順。
pub(crate) struct Path {
    pub cost: i32,
    pub nodes: Vec<u32>,
}

/// `NBest::through` の制約（ノードとその区間）
#[derive(Clone, Copy)]
struct Through {
    id: u32,
    start: u32,
    end: u32,
}

pub(crate) struct NBest {
    heap: BinaryHeap<Reverse<State>>,
    steps: Vec<Step>,
    seq: u64,
    through: Option<Through>,
    /// EOS の代わりの右の語の左ID（`with_end`）。同じ区間の `through` を作り直すために持つ
    end_left_id: u16,
}

impl NBest {
    pub fn new(lattice: &Lattice, grammar: &dyn Grammar) -> Self {
        Self::with_end(lattice, grammar, grammar.bos_eos_id())
    }

    /// `with_end` と同じ。ただし、出す経路はノード `through` を必ず通る
    /// （`through` の区間と重なるほかのノードへは進まない）。コストの昇順は保つ（ADR 0048 §3）。
    ///
    /// ヒューリスティック（`best`）は制約なしの最小コストで、制約つきでも下界のままなので A* は正しい。
    /// ただし `through` より後ろ側では `best` が緩く、展開は増える。
    pub fn through(
        lattice: &Lattice,
        grammar: &dyn Grammar,
        end_left_id: u16,
        through: u32,
    ) -> Self {
        let node = lattice.node(through);
        let mut nb = Self::empty(
            Some(Through {
                id: through,
                start: node.start,
                end: node.end,
            }),
            end_left_id,
        );
        nb.push_initial(lattice, grammar, end_left_id);
        nb
    }

    /// `new` の EOS を、右の文節の最初の語の左ID（`end_left_id`）にしたもの（ADR 0006 §3.1-2）。
    /// 右の文節との連接コストがパスのコストに入る。`new(l, g)` は `with_end(l, g, g.bos_eos_id())` と同じ。
    pub fn with_end(lattice: &Lattice, grammar: &dyn Grammar, end_left_id: u16) -> Self {
        let mut nb = Self::empty(None, end_left_id);
        nb.push_initial(lattice, grammar, end_left_id);
        nb
    }

    fn empty(through: Option<Through>, end_left_id: u16) -> Self {
        NBest {
            heap: BinaryHeap::new(),
            steps: Vec::new(),
            seq: 0,
            through,
            end_left_id,
        }
    }

    /// 読みの末尾で終わるノードから探索を始める
    fn push_initial(&mut self, lattice: &Lattice, grammar: &dyn Grammar, end_left_id: u16) {
        for &id in lattice.ending_at(lattice.len_chars() as u32) {
            if !self.allows(lattice.node(id), id) {
                continue;
            }
            let node = lattice.node(id);
            let g = i32::from(grammar.connection_cost(node.right_id, end_left_id));
            let step = self.push_step(id, NO_STEP);
            self.push_state(node.best + g, step, g);
        }
    }

    /// `through` の制約のもとで、`node`（id は `id`）を経路に入れてよいか。
    /// 後ろ向きに進むので、`through` の右側のノードだけを見ればよい（左側は `through` から先の連接で重ならない）
    fn allows(&self, node: &crate::lattice::Node, id: u32) -> bool {
        match self.through {
            None => true,
            Some(t) => id == t.id || node.start >= t.end || node.end <= t.start,
        }
    }

    /// この探索の EOS 側の連接に使う左ID（`new` なら BOS/EOS のID）
    pub fn end_left_id(&self) -> u16 {
        self.end_left_id
    }

    /// 探索する候補が残っていないか
    pub fn is_exhausted(&self) -> bool {
        self.heap.is_empty()
    }

    /// 次にコストが小さいパスを返す。キューから取り出すたびに `*pops` を1減らし、
    /// 0 になったら（パスが見つかっていなくても）None を返す。再開するには `*pops` を補充して呼び直す。
    pub fn next(
        &mut self,
        lattice: &Lattice,
        grammar: &dyn Grammar,
        pops: &mut usize,
    ) -> Option<Path> {
        while *pops > 0 {
            let Reverse(state) = self.heap.pop()?;
            *pops -= 1;
            let node = lattice.node(self.steps[state.step as usize].node);
            if node.start == 0 {
                // best(node) は BOS からの連接を含むので、f がパス全体のコスト
                return Some(self.finish(state.step, state.f));
            }
            let node_cost = i32::from(node.cost);
            for &p in lattice.ending_at(node.start) {
                let prev = lattice.node(p);
                if !self.allows(prev, p) {
                    continue;
                }
                let g = state.g
                    + node_cost
                    + i32::from(grammar.connection_cost(prev.right_id, node.left_id))
                    + lattice.edge_extra(prev, node);
                let step = self.push_step(p, state.step);
                self.push_state(prev.best + g, step, g);
            }
        }
        None
    }

    fn push_step(&mut self, node: u32, next: StepId) -> StepId {
        let id = self.steps.len() as StepId;
        self.steps.push(Step { node, next });
        id
    }

    fn push_state(&mut self, f: i32, step: StepId, g: i32) {
        let seq = self.seq;
        self.seq += 1;
        self.heap.push(Reverse(State { f, seq, step, g }));
    }

    fn finish(&self, head: StepId, cost: i32) -> Path {
        let mut nodes = Vec::new();
        let mut cur = head;
        while cur != NO_STEP {
            let s = &self.steps[cur as usize];
            nodes.push(s.node);
            cur = s.next;
        }
        Path { cost, nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{AdverbNounRule, NounSuffixRule, NounSuruRule, NumeralCoverRule};
    use crate::dict::{DenseGrammar, Dictionary, InMemoryDictionary};
    use crate::lattice::{EdgeRules, build, build_with_starts, testutil::*};

    const UNLIMITED: usize = usize::MAX;

    fn drain(lat: &Lattice, g: &dyn Grammar) -> Vec<Path> {
        let mut nb = NBest::new(lat, g);
        let mut pops = UNLIMITED;
        let mut out = Vec::new();
        while let Some(p) = nb.next(lat, g, &mut pops) {
            out.push(p);
        }
        assert!(nb.is_exhausted());
        out
    }

    /// 全パスを深さ優先で列挙してコストを求める（検証用）
    fn brute_force(lat: &Lattice, g: &dyn Grammar) -> Vec<(i32, Vec<u32>)> {
        // 辺のコスト = 連接コスト + ノードの間の辺の補正（BOS と EOS の辺には補正なし）
        fn rec(
            lat: &Lattice,
            g: &dyn Grammar,
            pos: u32,
            prev: Option<u32>,
            cost: i32,
            acc: &mut Vec<u32>,
            out: &mut Vec<(i32, Vec<u32>)>,
        ) {
            let prev_right = prev.map_or(g.bos_eos_id(), |p| lat.node(p).right_id);
            if pos as usize == lat.len_chars() {
                let total = cost + i32::from(g.connection_cost(prev_right, g.bos_eos_id()));
                out.push((total, acc.clone()));
                return;
            }
            for id in 0..lat.node_count() as u32 {
                let n = lat.node(id);
                if n.start != pos {
                    continue;
                }
                let extra = prev.map_or(0, |p| lat.edge_extra(lat.node(p), n));
                let c = cost
                    + i32::from(g.connection_cost(prev_right, n.left_id))
                    + extra
                    + i32::from(n.cost);
                acc.push(id);
                rec(lat, g, n.end, Some(id), c, acc, out);
                acc.pop();
            }
        }
        let mut out = Vec::new();
        rec(lat, g, 0, None, 0, &mut Vec::new(), &mut out);
        out
    }

    fn sample() -> (DenseGrammar, InMemoryDictionary) {
        let mut g = grammar();
        g.set(ID_NOUN, ID_NOUN, 300);
        g.set(ID_NOUN, ID_VERB_SURU, 50);
        g.set(ID_VERB_SURU, ID_NOUN, 700);
        g.set(ID_PARTICLE, ID_NOUN, 20);
        g.set(0, ID_NOUN, 10);
        g.set(ID_UNK, ID_NOUN, 400);
        g.set(ID_NOUN, ID_UNK, 400);
        g.set(ID_VERB_SURU, 0, 5);
        let mut d = dict_for(&g);
        d.insert(
            "へんかんする",
            "変換する",
            ID_VERB_SURU,
            ID_VERB_SURU,
            9000,
            1,
        );
        d.insert("へんかん", "変換", ID_NOUN, ID_NOUN, 1200, 0);
        d.insert("へんか", "変化", ID_NOUN, ID_NOUN, 1000, 0);
        d.insert("へん", "変", ID_NOUN, ID_NOUN, 2000, 0);
        d.insert("か", "課", ID_NOUN, ID_NOUN, 1500, 0);
        d.insert("かん", "缶", ID_NOUN, ID_NOUN, 1600, 0);
        d.insert("ん", "ん", ID_PARTICLE, ID_PARTICLE, 1800, 2);
        d.insert("する", "する", ID_VERB_SURU, ID_VERB_SURU, 800, 1);
        d.insert("す", "す", ID_VERB_SURU, ID_VERB_SURU, 3000, 1);
        d.insert("る", "る", ID_PARTICLE, ID_PARTICLE, 2500, 2);
        (g, d)
    }

    #[test]
    fn paths_are_ascending_and_match_brute_force() {
        let (g, d) = sample();
        let lat = build("へんかんする", &d, &g, g.bos_eos_id());
        let got = drain(&lat, &g);
        let mut want = brute_force(&lat, &g);
        want.sort();
        assert_eq!(got.len(), want.len());
        let costs: Vec<i32> = got.iter().map(|p| p.cost).collect();
        assert!(
            costs.windows(2).all(|w| w[0] <= w[1]),
            "not ascending: {costs:?}"
        );
        let mut want_costs: Vec<i32> = want.iter().map(|w| w.0).collect();
        want_costs.sort();
        assert_eq!(costs, want_costs);
        // 各パスのコストが、ノードから計算し直した値と一致する
        let mut got_sorted: Vec<(i32, Vec<u32>)> =
            got.into_iter().map(|p| (p.cost, p.nodes)).collect();
        got_sorted.sort();
        assert_eq!(got_sorted, want);
    }

    #[test]
    fn first_path_is_viterbi_best() {
        let (g, d) = sample();
        let lat = build("へんかんする", &d, &g, g.bos_eos_id());
        let mut nb = NBest::new(&lat, &g);
        let mut pops = UNLIMITED;
        let best = nb.next(&lat, &g, &mut pops).unwrap();
        let surface: String = best
            .nodes
            .iter()
            .map(|&id| {
                let n = lat.node(id);
                let e = d.entry(n.word.unwrap()).unwrap();
                e.surface.to_owned()
            })
            .collect();
        // 変換(1200)+する(800)+連接 50+0+10+5 = 2065 が最良
        assert_eq!(surface, "変換する");
        assert_eq!(best.cost, 10 + 1200 + 50 + 800 + 5);
    }

    #[test]
    fn pops_budget_stops_and_resumes() {
        let (g, d) = sample();
        let lat = build("へんかんする", &d, &g, g.bos_eos_id());
        let all = drain(&lat, &g);

        let mut nb = NBest::new(&lat, &g);
        let mut got = Vec::new();
        let mut rounds = 0;
        // 1回の予算を小さくして何度も再開しても、同じ順序で全パスが出る
        while !nb.is_exhausted() {
            let mut pops = 3;
            while let Some(p) = nb.next(&lat, &g, &mut pops) {
                got.push(p.cost);
            }
            rounds += 1;
            assert!(rounds < 10_000);
        }
        let want: Vec<i32> = all.iter().map(|p| p.cost).collect();
        assert_eq!(got, want);
        let mut zero = 0;
        assert!(nb.next(&lat, &g, &mut zero).is_none());
    }

    #[test]
    fn empty_lattice_yields_nothing() {
        let g = grammar();
        let d = dict_for(&g);
        let lat = build("", &d, &g, 0);
        let mut nb = NBest::new(&lat, &g);
        assert!(nb.is_exhausted());
        let mut pops = 10;
        assert!(nb.next(&lat, &g, &mut pops).is_none());
    }

    #[test]
    fn new_is_with_end_of_bos_eos() {
        let (g, d) = sample();
        let lat = build("へんかんする", &d, &g, g.bos_eos_id());
        let a = drain(&lat, &g);
        let mut nb = NBest::with_end(&lat, &g, g.bos_eos_id());
        let mut pops = UNLIMITED;
        let mut b = Vec::new();
        while let Some(p) = nb.next(&lat, &g, &mut pops) {
            b.push((p.cost, p.nodes));
        }
        let a: Vec<(i32, Vec<u32>)> = a.into_iter().map(|p| (p.cost, p.nodes)).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn with_end_adds_the_connection_to_the_next_clause() {
        let (mut g, d) = sample();
        // 右の文節の最初の語の左ID = ID_NOUN。名詞 -> 名詞 の連接は 300、動詞 -> 名詞 は 700
        g.set(ID_PARTICLE, ID_NOUN, 20);
        let lat = build("へんかん", &d, &g, g.bos_eos_id());
        let mut nb = NBest::with_end(&lat, &g, ID_NOUN);
        let mut pops = UNLIMITED;
        let mut got = Vec::new();
        while let Some(p) = nb.next(&lat, &g, &mut pops) {
            // 各パスのコストは、末尾の語から右の文節への連接を含む
            let last = lat.node(*p.nodes.last().unwrap());
            let mut cost = 0;
            let mut prev_right = g.bos_eos_id();
            for &id in &p.nodes {
                let n = lat.node(id);
                cost += i32::from(g.connection_cost(prev_right, n.left_id)) + i32::from(n.cost);
                prev_right = n.right_id;
            }
            cost += i32::from(g.connection_cost(last.right_id, ID_NOUN));
            assert_eq!(p.cost, cost);
            got.push(p.cost);
        }
        assert!(got.windows(2).all(|w| w[0] <= w[1]), "{got:?}");
        // 末尾が「ん」（助詞。ID_PARTICLE -> ID_NOUN = 20）のパスは安く、EOS への連接 0 より高いことを確かめる
        let eos_costs = drain(&lat, &g);
        assert_eq!(eos_costs.len(), got.len());
        assert_ne!(eos_costs.iter().map(|p| p.cost).collect::<Vec<_>>(), got);
    }

    #[test]
    fn with_end_can_change_the_best_path() {
        let (mut g, d) = sample();
        // 末尾が 名詞 のパスと 助詞 のパスで、右の文節への連接だけが違う
        g.set(ID_NOUN, ID_NOUN, 5000);
        g.set(ID_PARTICLE, ID_NOUN, 0);
        let lat = build("かん", &d, &g, g.bos_eos_id());
        let mut pops = UNLIMITED;
        let tail_of = |nb: &mut NBest, pops: &mut usize| {
            let p = nb.next(&lat, &g, pops).unwrap();
            let last = lat.node(*p.nodes.last().unwrap());
            d.entry(last.word.unwrap()).unwrap().surface.to_owned()
        };
        let mut plain = NBest::new(&lat, &g);
        assert_eq!(tail_of(&mut plain, &mut pops), "缶");
        let mut towards_noun = NBest::with_end(&lat, &g, ID_NOUN);
        assert_eq!(tail_of(&mut towards_noun, &mut pops), "ん");
    }

    // ---- 副詞の直後の名詞（ADR 0008）: 前向きと後ろ向きが同じ辺の補正を使う ----

    fn surfaces(lat: &Lattice, d: &dyn Dictionary, p: &Path) -> String {
        p.nodes
            .iter()
            .map(|&id| match lat.node(id).word {
                Some(w) => d.entry(w).unwrap().surface.to_owned(),
                None => lat.slice(lat.node(id)).to_owned(),
            })
            .collect::<Vec<_>>()
            .join("+")
    }

    #[test]
    fn adverb_noun_rule_keeps_astar_exact_and_ascending() {
        let (g, adverb) = grammar_with_adverb();
        let mut d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        // 名詞と副詞がいくつも並ぶ読み（そうじき＋そうじき）で、全パスを総当たりと比べる
        d.add("き", "機", ID_NOUN, ID_NOUN, 2500, POS_ID_NOUN);
        d.add("そうじ", "掃除", ID_NOUN, ID_NOUN, 1500, POS_ID_NOUN);
        let r = EdgeRules::new(
            &g,
            AdverbNounRule::default(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let lat = build_with_starts("そうじきそうじき", &d, &g, &[(g.bos_eos_id(), 0)], &r);
        let got = drain(&lat, &g);
        let costs: Vec<i32> = got.iter().map(|p| p.cost).collect();
        assert!(costs.windows(2).all(|w| w[0] <= w[1]), "{costs:?}");
        let mut want = brute_force(&lat, &g);
        want.sort();
        let mut got_sorted: Vec<(i32, Vec<u32>)> =
            got.into_iter().map(|p| (p.cost, p.nodes)).collect();
        got_sorted.sort();
        assert_eq!(got_sorted, want);
    }

    #[test]
    fn first_path_equals_viterbi_and_the_penalized_path_stays_in_the_list() {
        let (g, adverb) = grammar_with_adverb();
        let d = adverb_dict(&g, adverb, 4000, POS_ID_NOUN);
        let r = EdgeRules::new(
            &g,
            AdverbNounRule::default(),
            NounSuruRule::disabled(),
            NounSuffixRule::disabled(),
            NumeralCoverRule::disabled(),
        );
        let lat = build_with_starts("そうじき", &d, &g, &[(g.bos_eos_id(), 0)], &r);
        let viterbi = lat
            .ending_at(lat.len_chars() as u32)
            .iter()
            .map(|&id| {
                let n = lat.node(id);
                n.best + i32::from(g.connection_cost(n.right_id, g.bos_eos_id()))
            })
            .min()
            .unwrap();
        let paths = drain(&lat, &g);
        assert_eq!(paths[0].cost, viterbi);
        assert_eq!(surfaces(&lat, &d, &paths[0]), "掃除機");
        // そう＋時期 は消えずに、加算したコスト（1000 + 1000 + 6000）で後ろに残る
        let sou_jiki = paths
            .iter()
            .find(|p| surfaces(&lat, &d, p) == "そう+時期")
            .expect("adverb + noun path is kept");
        assert_eq!(sou_jiki.cost, 8000);

        // 規則なしなら そう＋時期 が1位
        let plain = build_with_starts(
            "そうじき",
            &d,
            &g,
            &[(g.bos_eos_id(), 0)],
            &EdgeRules::none(),
        );
        let paths = drain(&plain, &g);
        assert_eq!(surfaces(&plain, &d, &paths[0]), "そう+時期");
        assert_eq!(paths[0].cost, 2000);
    }
}
