//! 取り込みの本体（ADR 0002 §11.3, §11.4）。検証済みの文を、アライメントして初期学習にする。
//!
//! - 評価用（`FNV64(表記) % 5 == 0`）の文は何も学習しない。
//! - 未知語を使った文は学習に使わず、(表記, 推定した読み) を登録候補として数える。
//! - 学習用の文は、内容語ごとに `initial` 層の `word` 記録を作る（同音異義語がある語と、
//!   正規化表記と書き方が違う語だけ）。文そのもの、語順、左文脈の語は保存しない。

use std::collections::{BTreeMap, HashMap};

use ikari_core::pos::is_content_word;
use ikari_core::user::{LearningStore, Profile, ProfileWeight, SegmentKey, extract_named};
use ikari_core::{AnalyzedContext, ContextToken, Dictionary, Grammar, WordVectors};

use crate::align::{AlignedToken, TokenKind, align};
use crate::holdout::is_holdout;
use crate::schema::{DropCounts, DropReason, Parsed};

/// 特徴量の計算に使う、直前のトークンの数（`Bag` は内容語 20 個までなので十分に長く取る）
const FEATURE_WINDOW_TOKENS: usize = 100;
/// 走査中に持つトークンの上限（これを超えたら古い方を捨てる）
const MAX_RUNNING_TOKENS: usize = FEATURE_WINDOW_TOKENS * 4;
/// 同音異義語とみなす、同じ読みの表記の数
const MIN_HOMOPHONE_SURFACES: usize = 2;
/// 分野の推定で、`often` / `sometimes` の候補にする最大数
const MAX_SUGGESTED_DOMAINS: usize = 3;
/// 最良の分野の何割以上なら `sometimes` にするか
const SOMETIMES_RATIO: f32 = 0.5;
/// 分野の最低スコア（これ未満は提案しない）
const MIN_COSINE: f32 = 0.25;
const MIN_OVERLAP: f32 = 0.03;

/// 取り込みが使う辞書、文法、（あれば）単語ベクトル。ユーザー辞書を含む辞書を渡すと、
/// ユーザー辞書の語も「辞書にある語」として扱われる。
pub struct Env<'a> {
    pub dict: &'a dyn Dictionary,
    pub grammar: &'a dyn Grammar,
    pub vectors: Option<&'a dyn WordVectors>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreMethod {
    /// 内容語ベクトルの平均と、種語の重心のコサイン類似度
    Cosine,
    /// 内容語のうち、種語に一致した割合（vectors.bin がないとき）
    Overlap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DomainScore {
    pub id: String,
    pub name: String,
    pub score: f32,
    pub method: ScoreMethod,
}

impl DomainScore {
    fn min_score(&self) -> f32 {
        match self.method {
            ScoreMethod::Cosine => MIN_COSINE,
            ScoreMethod::Overlap => MIN_OVERLAP,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IngestOutcome {
    pub prompt_version: String,
    /// スキーマの検証で捨てた行と、アライメントできなかった文の理由別の件数
    pub drops: DropCounts,
    /// 検証を通った文の数
    pub sentences_valid: u32,
    /// アライメントできた文の数（`sentences_ok`）
    pub aligned: u32,
    /// 評価用に取り分けた文の数（学習しない）
    pub holdout: u32,
    /// 初期学習に使った文の数
    pub learned: u32,
    /// 未知語を使っていて、学習には使わなかった文の数
    pub with_unknown: u32,
    /// 作った `initial` 層の記録の数（語単位。dry-run では「作るはずの数」）
    pub initial_records: u32,
    /// 登録候補: (読み, 表記, 件数)。件数の多い順
    pub candidates: Vec<(String, String, u32)>,
    /// 分野の推定（スコアの高い順）
    pub domains: Vec<DomainScore>,
}

impl IngestOutcome {
    pub fn sentences_dropped(&self) -> u32 {
        self.drops.total()
    }
}

/// 検証済みの取り込みを処理する。`store` が None なら何も記録しない（dry-run）。
/// `source` は取り込み元の番号、`today` は UNIX 日。
pub fn ingest(
    parsed: &Parsed,
    env: &Env<'_>,
    store: Option<&LearningStore>,
    source: u32,
    today: u32,
) -> IngestOutcome {
    let mut out = IngestOutcome {
        prompt_version: parsed.prompt_version.clone(),
        drops: parsed.drops.clone(),
        sentences_valid: parsed.sentences.len() as u32,
        ..IngestOutcome::default()
    };
    let salt = store.map_or(0, LearningStore::salt);
    // 特徴量の品詞名は、学習ストアに結びつけた文法で作る（Ranker と食い違わないように）
    let feature_grammar = store.and_then(LearningStore::grammar);

    let mut candidates: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut running: Vec<ContextToken> = Vec::new();
    let mut current_doc = None;
    let mut domain_acc = DomainAccumulator::default();

    for sentence in &parsed.sentences {
        if current_doc != Some(sentence.doc) {
            current_doc = Some(sentence.doc);
            running.clear();
        }
        let Some(alignment) = align(env.dict, env.grammar, &sentence.surface, &sentence.reading)
        else {
            out.drops.add(DropReason::NotAlignable);
            continue;
        };
        out.aligned += 1;
        if is_holdout(&sentence.surface) {
            out.holdout += 1;
            continue;
        }
        if alignment.uses_unknown() {
            out.with_unknown += 1;
            for t in alignment
                .tokens
                .iter()
                .filter(|t| t.kind == TokenKind::Unknown)
            {
                *candidates
                    .entry((t.reading.clone(), t.surface.clone()))
                    .or_default() += 1;
            }
            continue;
        }
        out.learned += 1;

        for tok in &alignment.tokens {
            let ctx_token = context_token(tok, env);
            if ctx_token.is_content {
                domain_acc.add(&ctx_token, env);
                if is_worth_learning(tok, &ctx_token, env) {
                    out.initial_records += 1;
                    if let Some(store) = store {
                        let analysis = window_analysis(&running);
                        let feats = extract_named(&analysis, feature_grammar, env.vectors, salt);
                        let key = SegmentKey {
                            surface: tok.surface.clone(),
                            reading: tok.reading.clone(),
                            pos_name: env.grammar.pos_name(tok.pos_id).unwrap_or("*").to_string(),
                        };
                        store.add_initial(
                            source,
                            &tok.reading,
                            std::slice::from_ref(&key),
                            &feats,
                            today,
                        );
                    }
                }
            }
            running.push(ctx_token);
        }
        if running.len() > MAX_RUNNING_TOKENS {
            running.drain(..running.len() - FEATURE_WINDOW_TOKENS);
        }
    }

    out.candidates = candidates
        .into_iter()
        .map(|((reading, surface), n)| (reading, surface, n))
        .collect();
    out.candidates.sort_by_key(|c| std::cmp::Reverse(c.2));
    out.domains = domain_acc.score(env);
    out
}

/// 学習の特徴量用の左文脈（直前のトークンの窓）。
fn window_analysis(running: &[ContextToken]) -> AnalyzedContext {
    let tokens: Vec<ContextToken> =
        running[running.len().saturating_sub(FEATURE_WINDOW_TOKENS)..].to_vec();
    let content_count = tokens.iter().filter(|t| t.is_content).count();
    AnalyzedContext {
        bos_right_id: tokens.last().map_or(0, |t| t.right_id),
        tokens,
        bos_alternatives: Vec::new(),
        content_count,
    }
}

fn context_token(tok: &AlignedToken, env: &Env<'_>) -> ContextToken {
    let (lemma, lemma_id) = tok
        .word
        .and_then(|w| env.dict.lemma(w))
        .map_or((tok.surface.clone(), None), |l| (l.text.to_string(), l.id));
    let is_content = tok.kind == TokenKind::Dictionary
        && env
            .grammar
            .pos_name(tok.pos_id)
            .is_some_and(is_content_word);
    ContextToken {
        surface: tok.surface.clone(),
        lemma,
        lemma_id,
        word: tok.word,
        left_id: tok.left_id,
        right_id: tok.right_id,
        pos_id: tok.pos_id,
        is_content,
        char_end: 0,
    }
}

/// 同音異義語がある語、または正規化表記と書き方が違う語（表記の好み）だけを学習する。
fn is_worth_learning(tok: &AlignedToken, ctx: &ContextToken, env: &Env<'_>) -> bool {
    if tok.kind != TokenKind::Dictionary {
        return false;
    }
    let mut surfaces: Vec<String> = Vec::new();
    env.dict.lookup_exact(&tok.reading, &mut |e| {
        if !surfaces.iter().any(|s| s == e.surface) {
            surfaces.push(e.surface.to_string());
        }
    });
    surfaces.len() >= MIN_HOMOPHONE_SURFACES || ctx.lemma != ctx.surface
}

/// 分野の推定のための集計（学習に使った文の内容語）。
#[derive(Default)]
struct DomainAccumulator {
    lemmas: HashMap<String, u32>,
    vector_sum: Vec<f32>,
    vector_count: u32,
    content_total: u32,
}

impl DomainAccumulator {
    fn add(&mut self, tok: &ContextToken, env: &Env<'_>) {
        self.content_total += 1;
        *self.lemmas.entry(tok.lemma.clone()).or_default() += 1;
        if let (Some(v), Some(id)) = (env.vectors, tok.lemma_id)
            && let Some(row) = v.vector(id)
        {
            add_i8(&mut self.vector_sum, row);
            self.vector_count += 1;
        }
    }

    fn score(&self, env: &Env<'_>) -> Vec<DomainScore> {
        if self.content_total == 0 {
            return Vec::new();
        }
        let mut scores: Vec<DomainScore> = Profile::presets()
            .into_iter()
            .map(|d| {
                let cosine = env
                    .vectors
                    .filter(|_| self.vector_count > 0)
                    .and_then(|v| centroid(env, v, &d.seeds))
                    .map(|c| cosine(&self.vector_sum, &c));
                let (score, method) = match cosine {
                    Some(c) => (c, ScoreMethod::Cosine),
                    None => {
                        let hits: u32 = d.seeds.iter().filter_map(|s| self.lemmas.get(s)).sum();
                        (
                            hits as f32 / self.content_total as f32,
                            ScoreMethod::Overlap,
                        )
                    }
                };
                DomainScore {
                    id: d.id,
                    name: d.name,
                    score,
                    method,
                }
            })
            .collect();
        scores.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        scores
    }
}

fn add_i8(sum: &mut Vec<f32>, row: &[i8]) {
    if sum.is_empty() {
        sum.resize(row.len(), 0.0);
    }
    if sum.len() != row.len() {
        return;
    }
    for (s, &x) in sum.iter_mut().zip(row) {
        *s += f32::from(x);
    }
}

/// 種語のベクトルの平均。引けた種語がなければ None。
fn centroid(env: &Env<'_>, vectors: &dyn WordVectors, seeds: &[String]) -> Option<Vec<f32>> {
    let mut sum: Vec<f32> = Vec::new();
    let mut n = 0;
    for seed in seeds {
        if let Some(id) = seed_lemma_id(env.dict, seed)
            && let Some(row) = vectors.vector(id)
        {
            add_i8(&mut sum, row);
            n += 1;
        }
    }
    (n > 0).then_some(sum)
}

/// 辞書の lemma 番号を、lemma の文字列から逆引きする（表記の索引から、表記 = lemma のエントリを探す）。
fn seed_lemma_id(dict: &dyn Dictionary, seed: &str) -> Option<u32> {
    let mut found = None;
    dict.common_prefix_search_surface(seed, &mut |len, e| {
        if len == seed.len()
            && found.is_none()
            && let Some(l) = dict.lemma(e.id)
            && l.text == seed
        {
            found = l.id;
        }
    });
    found
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let (na, nb) = (
        a.iter().map(|x| x * x).sum::<f32>().sqrt(),
        b.iter().map(|x| x * x).sum::<f32>().sqrt(),
    );
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// 分野のスコアから `profile.tsv` に書く内容を提案する。最良が `often`、
/// それに近いもの（最良の半分以上）が `sometimes`。最低スコア未満なら何も提案しない。
pub fn suggest_profile(scores: &[DomainScore]) -> Vec<(String, ProfileWeight)> {
    let Some(best) = scores.first() else {
        return Vec::new();
    };
    if best.score < best.min_score() {
        return Vec::new();
    }
    let mut out = vec![(best.id.clone(), ProfileWeight::OFTEN)];
    for s in &scores[1..] {
        if out.len() >= MAX_SUGGESTED_DOMAINS {
            break;
        }
        if s.score >= s.min_score() && s.score >= best.score * SOMETIMES_RATIO {
            out.push((s.id.clone(), ProfileWeight::SOMETIMES));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ikari_core::user::{Layer, LearningRecord, Unit, export_text};
    use ikari_core::{UserDir, user::LearningStore};

    use super::*;
    use crate::schema::parse_jsonl;
    use crate::test_support::*;

    const TODAY: u32 = 20_000;

    fn parsed(sentences: &[(String, String)]) -> Parsed {
        parse_jsonl(jsonl(sentences).as_bytes()).unwrap()
    }

    fn open_store(dir: &tempfile::TempDir, m: &Mini) -> Arc<LearningStore> {
        let store = LearningStore::open(&dir.path().join("learning.tsv")).unwrap();
        store.attach_grammar(Arc::new(clone_grammar(m)));
        Arc::new(store)
    }

    /// `attach_grammar` に渡す、`mini()` と同じ品詞表の文法
    fn clone_grammar(_m: &Mini) -> ikari_core::DenseGrammar {
        mini().grammar
    }

    fn env(m: &Mini) -> Env<'_> {
        Env {
            dict: &m.dict,
            grammar: &m.grammar,
            vectors: None,
        }
    }

    fn word_surfaces(store: &LearningStore) -> Vec<String> {
        let mut v: Vec<String> = store
            .records()
            .into_iter()
            .filter(|r| r.feature == 0)
            .map(|r| r.surface)
            .collect();
        v.sort();
        v.dedup();
        v
    }

    #[test]
    fn learns_homophones_and_preferred_spellings_from_training_sentences_only() {
        let m = mini();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &m);
        let p = parsed(&[
            // 初めて は同音異義語（始めて）、ユーザ は lemma（ユーザー）と表記が違う
            training("その話を初めて知った", "そのはなしをはじめてしった"),
            training("ユーザ辞書", "ゆーざじしょ"),
            // 評価用: 学習しない
            holdout("単語を始めて登録する", "たんごをはじめてとうろくする"),
        ]);
        let out = ingest(&p, &env(&m), Some(&store), 7, TODAY);
        assert_eq!(out.sentences_valid, 3);
        assert_eq!(out.aligned, 3);
        assert_eq!(out.holdout, 1);
        assert_eq!(out.learned, 2);
        assert_eq!(out.with_unknown, 0);
        assert_eq!(out.initial_records, 2);
        assert_eq!(out.drops.total(), 0);
        assert_eq!(word_surfaces(&store), ["ユーザ", "初めて"]);
        // 評価用の「始めて」は学習されない
        assert!(word_surfaces(&store).iter().all(|s| s != "始めて"));
        for r in store.records() {
            assert_eq!(r.layer, Layer::Initial);
            assert_eq!(r.source, Some(7));
            assert_eq!(r.unit, Unit::Word);
            assert_eq!(r.last_day, TODAY);
        }
    }

    #[test]
    fn dry_run_counts_but_records_nothing() {
        let m = mini();
        let p = parsed(&[training(
            "その話を初めて知った",
            "そのはなしをはじめてしった",
        )]);
        let out = ingest(&p, &env(&m), None, 1, TODAY);
        assert_eq!(out.initial_records, 1);
        assert_eq!(out.learned, 1);
    }

    #[test]
    fn features_carry_the_preceding_context_of_the_document() {
        let m = mini();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &m);
        let p = parsed(&[training(
            "その話を初めて知った",
            "そのはなしをはじめてしった",
        )]);
        ingest(&p, &env(&m), Some(&store), 1, TODAY);
        let recs: Vec<LearningRecord> = store.lookup("はじめて");
        // Unconditional + Prev(を) + PrevContent(話) + Bag(話, その)
        let kinds = recs.len();
        assert!(kinds >= 4, "{kinds}");
        assert!(recs.iter().any(|r| r.feature == 0));
        // 文脈が違えば、特徴量のハッシュは変わる
        let dir2 = tempfile::tempdir().unwrap();
        let store2 = open_store(&dir2, &m);
        let p2 = parsed(&[training("辞書を初めて知った", "じしょをはじめてしった")]);
        ingest(&p2, &env(&m), Some(&store2), 1, TODAY);
        let h1: Vec<u64> = recs.iter().map(|r| r.feature).collect();
        let h2: Vec<u64> = store2
            .lookup("はじめて")
            .iter()
            .map(|r| r.feature)
            .collect();
        // ソルトが違うので全く別だが、念のため個数の違いも見る（Bag の語数が違う）
        assert_ne!(h1.len(), h2.len());
    }

    #[test]
    fn unknown_words_become_candidates_and_the_sentence_is_not_learned() {
        let m = mini();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &m);
        let p = parsed(&[
            training("怒里を登録する", "いかりをとうろくする"),
            training("怒里に単語", "いかりにたんご"),
        ]);
        let out = ingest(&p, &env(&m), Some(&store), 1, TODAY);
        assert_eq!(out.with_unknown, 2);
        assert_eq!(out.learned, 0);
        assert!(store.is_empty());
        assert_eq!(
            out.candidates,
            vec![("いかり".to_string(), "怒里".to_string(), 2)]
        );
    }

    #[test]
    fn unalignable_sentences_are_dropped_and_counted() {
        let m = mini();
        let p = parsed(&[
            training("辞書に単語", "じしょにたんご"),
            // LLM の読み違い（辞書にある表記に別の読み）
            training("辞書に単語", "たんごにじしょ"),
            training("辞書を", "じしょに"),
        ]);
        let out = ingest(&p, &env(&m), None, 1, TODAY);
        assert_eq!(out.drops.get(DropReason::NotAlignable), 2);
        assert_eq!(out.sentences_dropped(), 2);
        assert_eq!(out.aligned, 1);
    }

    #[test]
    fn learning_file_and_export_do_not_contain_the_sentences() {
        let m = mini();
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &m);
        let sentences = [
            training("その話を初めて知った", "そのはなしをはじめてしった"),
            training(
                "ユーザ辞書に単語を登録する",
                "ゆーざじしょにたんごをとうろくする",
            ),
            training(
                "ユーザー辞書を初めて登録する",
                "ゆーざーじしょをはじめてとうろくする",
            ),
        ];
        let p = parsed(&sentences);
        let out = ingest(&p, &env(&m), Some(&store), 1, TODAY);
        assert_eq!(out.learned, 3);
        store.save().unwrap();

        let learning = std::fs::read_to_string(dir.path().join("learning.tsv")).unwrap();
        let export = export_text(&UserDir::new(dir.path())).unwrap();
        for (surface, _) in &sentences {
            let chars: Vec<char> = surface.chars().collect();
            let a =
                crate::align(&m.dict, &m.grammar, surface, &parse_reading(&p, surface)).unwrap();
            // 語の境界（文字位置）
            let mut bounds = vec![0usize];
            for t in &a.tokens {
                bounds.push(bounds.last().unwrap() + t.surface.chars().count());
            }
            for text in [&learning, &export] {
                assert!(!text.contains(surface.as_str()), "文がそのまま入っている");
                // 語をまたぐ 3 文字以上の並びは、どこにもない（語順が保存されない）
                for start in 0..chars.len().saturating_sub(2) {
                    let end = start + 3;
                    let within_one_token = bounds.windows(2).any(|w| w[0] <= start && end <= w[1]);
                    if within_one_token {
                        continue;
                    }
                    let window: String = chars[start..end].iter().collect();
                    assert!(
                        !text.contains(&window),
                        "語をまたぐ並び {window:?} が保存されている"
                    );
                }
            }
        }
    }

    fn parse_reading(p: &Parsed, surface: &str) -> String {
        p.sentences
            .iter()
            .find(|s| s.surface == surface)
            .unwrap()
            .reading
            .clone()
    }

    #[test]
    fn domain_scores_prefer_the_matching_preset() {
        let m = mini();
        let p = parsed(&[
            training(
                "ユーザー辞書に単語を登録する",
                "ゆーざーじしょにたんごをとうろくする",
            ),
            training("ユーザー辞書を登録する", "ゆーざーじしょをとうろくする"),
        ]);
        let out = ingest(&p, &env(&m), None, 1, TODAY);
        assert_eq!(out.domains.len(), 4);
        assert_eq!(out.domains[0].id, "it");
        assert_eq!(out.domains[0].method, ScoreMethod::Overlap);
        let s = suggest_profile(&out.domains);
        assert_eq!(s[0], ("it".to_string(), ProfileWeight::OFTEN));
    }

    #[test]
    fn suggest_profile_needs_a_minimum_score() {
        let weak = vec![DomainScore {
            id: "it".into(),
            name: "IT".into(),
            score: 0.001,
            method: ScoreMethod::Overlap,
        }];
        assert!(suggest_profile(&weak).is_empty());
        assert!(suggest_profile(&[]).is_empty());
        let mixed: Vec<DomainScore> = [
            ("it", 0.4),
            ("infra", 0.3),
            ("novel", 0.1),
            ("business", 0.05),
        ]
        .iter()
        .map(|(id, score)| DomainScore {
            id: id.to_string(),
            name: id.to_string(),
            score: *score,
            method: ScoreMethod::Overlap,
        })
        .collect();
        let s = suggest_profile(&mixed);
        assert_eq!(
            s,
            vec![
                ("it".to_string(), ProfileWeight::OFTEN),
                ("infra".to_string(), ProfileWeight::SOMETIMES)
            ]
        );
    }
}
