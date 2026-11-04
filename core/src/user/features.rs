//! 学習の特徴量（ADR 0002 §8.1）。生テキストは持たず、ソルト付きのハッシュだけを持つ。
//!
//! # プライバシー上の残留リスク（ADR 0002 §8.1, §21.1-6）
//! 保存するのは語順のない 64 ビットのハッシュだけで、生テキスト、文、語順は復元できない（NFR-19）。
//! ただし、ソルトは学習ファイル（とエクスポート）に入っており、辞書の語彙は公開されている。
//! 攻撃者がこの 2 つを手に入れると、`Prev`、`PrevContent`、`Bag` の特徴量は語彙を総当たりして
//! 「その位置の直前に出た語」「近くに出た内容語の集合」へ戻せる。文章は復元できないが、
//! 「近くに出た語の集合」は推測されうる。`Bag` は既定で有効（利用者の決定）にしてあるので、
//! 気になる場合は学習ファイルとエクスポートの扱いに注意すること。
//! `Unconditional` は固定値、`Topic` はクラスタ番号（64 通り）だけなので、ここからは何も戻せない。

use crate::analysis::{AnalyzedContext, ContextToken};
use crate::dict::Grammar;
use crate::pos::pos1;
use crate::vectors::WordVectors;

use super::util::{fnv1a64, fnv1a64_continue};

pub const UNCONDITIONAL_WEIGHT: f32 = 0.2;
pub const PREV_WEIGHT: f32 = 1.0;
pub const PREV_CONTENT_WEIGHT: f32 = 0.7;
/// `Bag` の特徴量の重みの合計（1 個あたりは `BAG_TOTAL_WEIGHT / 個数`）
pub const BAG_TOTAL_WEIGHT: f32 = 0.8;
pub const TOPIC_WEIGHT: f32 = 0.8;

/// `PrevContent` が内容語を探すトークン数
const PREV_CONTENT_WINDOW: usize = 10;
/// `Bag` に入れる内容語の最大数
const BAG_MAX_LEMMAS: usize = 20;
/// `Topic` の文脈ベクトルに使う内容語の最大数と、新しいものほど重くする係数
const TOPIC_MAX_TOKENS: usize = 50;
const TOPIC_DECAY: f32 = 0.97;
/// これより内容語が少ないと `Topic` は出さない
const TOPIC_MIN_TOKENS: usize = 3;
/// i8 のベクトルを f32 に戻す係数（ADR 0002 §3.4）
const VECTOR_SCALE: f32 = 127.0;

const PREFIX_PREV: &str = "p:";
const PREFIX_PREV_CONTENT: &str = "pc:";
const PREFIX_BAG: &str = "b:";
const PREFIX_TOPIC: &str = "t:";
const POS_SEPARATOR: &str = "/";
const UNKNOWN_POS_PREFIX: &str = "#";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FeatureKind {
    Unconditional,
    Prev,
    PrevContent,
    Bag,
    Topic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feature {
    pub kind: FeatureKind,
    /// FNV-1a 64（ソルトを前置）。`Unconditional` は 0
    pub hash: u64,
    /// 初期値: Unconditional 0.2 / Prev 1.0 / PrevContent 0.7 / Bag 合計 0.8 / Topic 0.8
    pub weight: f32,
}

#[derive(Clone, Debug, Default)]
pub struct ContextFeatures {
    pub features: Vec<Feature>,
}

/// ソルト（little endian）を前置した FNV-1a 64。
fn salted_hash(salt: u64, prefix: &str, body: &str) -> u64 {
    let h = fnv1a64(&salt.to_le_bytes());
    let h = fnv1a64_continue(h, prefix.as_bytes());
    fnv1a64_continue(h, body.as_bytes())
}

/// 解析済みの左文脈から特徴量を作る。`vectors` がなければ Topic は出さない。
///
/// 品詞は grammar を渡されないので、`#<pos_id>` で代用する。品詞名で安定させたいときは
/// [`extract_named`] を使う（`LearningStore::attach_grammar` を呼ぶとそちらが使われる）。
pub fn extract(
    analysis: &AnalyzedContext,
    vectors: Option<&dyn WordVectors>,
    salt: u64,
) -> ContextFeatures {
    extract_named(analysis, None, vectors, salt)
}

/// `extract` の品詞名つき版。`grammar` があれば `Prev` に品詞1（`名詞` など）を使う。
pub fn extract_named(
    analysis: &AnalyzedContext,
    grammar: Option<&dyn Grammar>,
    vectors: Option<&dyn WordVectors>,
    salt: u64,
) -> ContextFeatures {
    let mut features = vec![Feature {
        kind: FeatureKind::Unconditional,
        hash: 0,
        weight: UNCONDITIONAL_WEIGHT,
    }];
    let tokens = &analysis.tokens;

    if let Some(last) = tokens.last() {
        let pos = pos_label(last, grammar);
        let body = format!("{}{POS_SEPARATOR}{pos}", last.lemma);
        features.push(Feature {
            kind: FeatureKind::Prev,
            hash: salted_hash(salt, PREFIX_PREV, &body),
            weight: PREV_WEIGHT,
        });
    }

    let recent = &tokens[tokens.len().saturating_sub(PREV_CONTENT_WINDOW)..];
    if let Some(t) = recent.iter().rev().find(|t| t.is_content) {
        features.push(Feature {
            kind: FeatureKind::PrevContent,
            hash: salted_hash(salt, PREFIX_PREV_CONTENT, &t.lemma),
            weight: PREV_CONTENT_WEIGHT,
        });
    }

    let mut bag: Vec<&str> = Vec::new();
    for t in tokens.iter().rev().filter(|t| t.is_content) {
        if bag.len() >= BAG_MAX_LEMMAS {
            break;
        }
        if !bag.contains(&t.lemma.as_str()) {
            bag.push(&t.lemma);
        }
    }
    if !bag.is_empty() {
        let weight = BAG_TOTAL_WEIGHT / bag.len() as f32;
        for lemma in bag {
            features.push(Feature {
                kind: FeatureKind::Bag,
                hash: salted_hash(salt, PREFIX_BAG, lemma),
                weight,
            });
        }
    }

    if let Some(v) = vectors
        && let Some(cluster) = topic_cluster(tokens, v)
    {
        features.push(Feature {
            kind: FeatureKind::Topic,
            hash: salted_hash(salt, PREFIX_TOPIC, &cluster.to_string()),
            weight: TOPIC_WEIGHT,
        });
    }

    ContextFeatures { features }
}

fn pos_label(token: &ContextToken, grammar: Option<&dyn Grammar>) -> String {
    match grammar.and_then(|g| g.pos_name(token.pos_id)) {
        Some(name) => pos1(name).to_string(),
        None => format!("{UNKNOWN_POS_PREFIX}{}", token.pos_id),
    }
}

/// 最新の内容語のベクトルを `0.97^距離` で重み付けして平均し、最も近いクラスタの番号を返す。
fn topic_cluster(tokens: &[ContextToken], vectors: &dyn WordVectors) -> Option<u16> {
    let dims = vectors.dims();
    if dims == 0 {
        return None;
    }
    let mut sum = vec![0f32; dims];
    let mut used = 0usize;
    let mut weight = 1.0f32;
    for t in tokens.iter().rev().filter(|t| t.is_content) {
        if used >= TOPIC_MAX_TOKENS {
            break;
        }
        let Some(v) = t.lemma_id.and_then(|id| vectors.vector(id)) else {
            continue;
        };
        if v.len() != dims {
            continue;
        }
        for (s, &x) in sum.iter_mut().zip(v) {
            *s += weight * f32::from(x) / VECTOR_SCALE;
        }
        weight *= TOPIC_DECAY;
        used += 1;
    }
    if used < TOPIC_MIN_TOKENS {
        return None;
    }
    let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return None;
    }
    for s in &mut sum {
        *s /= norm;
    }
    vectors.nearest_cluster(&sum)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn token(lemma: &str, content: bool, pos_id: u16, lemma_id: Option<u32>) -> ContextToken {
        ContextToken {
            surface: lemma.to_string(),
            lemma: lemma.to_string(),
            lemma_id,
            word: None,
            left_id: 1,
            right_id: 1,
            pos_id,
            is_content: content,
            char_end: 0,
        }
    }

    pub fn analysis(tokens: Vec<ContextToken>) -> AnalyzedContext {
        let content_count = tokens.iter().filter(|t| t.is_content).count();
        AnalyzedContext {
            tokens,
            bos_right_id: 0,
            bos_alternatives: Vec::new(),
            content_count,
            carried_from: None,
        }
    }

    /// 2 次元、クラスタ 2 つ（x 軸に近ければ 0、y 軸に近ければ 1）。lemma_id 偶数が x、奇数が y
    pub struct FakeVectors;

    impl WordVectors for FakeVectors {
        fn dims(&self) -> usize {
            2
        }
        fn vector(&self, lemma_id: u32) -> Option<&[i8]> {
            static X: [i8; 2] = [127, 0];
            static Y: [i8; 2] = [0, 127];
            if lemma_id.is_multiple_of(2) {
                Some(&X)
            } else {
                Some(&Y)
            }
        }
        fn nearest_cluster(&self, v: &[f32]) -> Option<u16> {
            Some(u16::from(v[1].abs() > v[0].abs()))
        }
        fn rank(&self, lemma_id: u32) -> Option<u32> {
            Some(lemma_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn kinds(f: &ContextFeatures) -> Vec<FeatureKind> {
        f.features.iter().map(|x| x.kind).collect()
    }

    #[test]
    fn empty_context_has_only_unconditional() {
        let f = extract(&AnalyzedContext::default(), None, 7);
        assert_eq!(kinds(&f), vec![FeatureKind::Unconditional]);
        assert_eq!(f.features[0].hash, 0);
        assert_eq!(f.features[0].weight, UNCONDITIONAL_WEIGHT);
    }

    #[test]
    fn full_context_produces_every_kind() {
        let a = analysis(vec![
            token("ユーザー", true, 1, Some(0)),
            token("辞書", true, 1, Some(2)),
            token("単語", true, 1, Some(4)),
            token("を", false, 2, None),
        ]);
        let f = extract(&a, Some(&FakeVectors), 7);
        let k = kinds(&f);
        assert!(k.contains(&FeatureKind::Prev));
        assert!(k.contains(&FeatureKind::PrevContent));
        assert!(k.contains(&FeatureKind::Topic));
        assert_eq!(k.iter().filter(|x| **x == FeatureKind::Bag).count(), 3);
        let bag_sum: f32 = f
            .features
            .iter()
            .filter(|x| x.kind == FeatureKind::Bag)
            .map(|x| x.weight)
            .sum();
        assert!((bag_sum - BAG_TOTAL_WEIGHT).abs() < 1e-5);
    }

    #[test]
    fn topic_needs_vectors_and_enough_content_words() {
        let few = analysis(vec![
            token("a", true, 1, Some(0)),
            token("b", true, 1, Some(2)),
        ]);
        assert!(!kinds(&extract(&few, Some(&FakeVectors), 1)).contains(&FeatureKind::Topic));
        let enough = analysis(vec![
            token("a", true, 1, Some(0)),
            token("b", true, 1, Some(2)),
            token("c", true, 1, Some(4)),
        ]);
        assert!(!kinds(&extract(&enough, None, 1)).contains(&FeatureKind::Topic));
        assert!(kinds(&extract(&enough, Some(&FakeVectors), 1)).contains(&FeatureKind::Topic));
    }

    #[test]
    fn topic_cluster_follows_the_context_vectors() {
        let x = analysis(vec![
            token("a", true, 1, Some(0)),
            token("b", true, 1, Some(2)),
            token("c", true, 1, Some(4)),
        ]);
        let y = analysis(vec![
            token("a", true, 1, Some(1)),
            token("b", true, 1, Some(3)),
            token("c", true, 1, Some(5)),
        ]);
        let hash_of = |a: &AnalyzedContext| {
            extract(a, Some(&FakeVectors), 1)
                .features
                .into_iter()
                .find(|f| f.kind == FeatureKind::Topic)
                .unwrap()
                .hash
        };
        assert_ne!(hash_of(&x), hash_of(&y));
    }

    #[test]
    fn hashes_depend_on_salt_and_content_only() {
        let a = analysis(vec![token("品詞", true, 1, None)]);
        let b = analysis(vec![token("瀕死", true, 1, None)]);
        let hashes = |a: &AnalyzedContext, salt| -> Vec<u64> {
            extract(a, None, salt)
                .features
                .iter()
                .map(|f| f.hash)
                .collect()
        };
        assert_eq!(hashes(&a, 1), hashes(&a, 1));
        assert_ne!(hashes(&a, 1), hashes(&a, 2));
        assert_ne!(hashes(&a, 1), hashes(&b, 1));
    }

    /// 学習の特徴量のハッシュは保存済みの learning.tsv と照合されるので、値が変わってはいけない
    /// （Python で独立に計算した値）。
    #[test]
    fn salted_hash_known_value() {
        assert_eq!(
            salted_hash(0x0123_4567_89ab_cdef, "prev:", "きょう"),
            0x7774_2359_37bb_1865
        );
    }

    #[test]
    fn bag_is_deduplicated_and_limited() {
        let mut tokens = Vec::new();
        for i in 0..30 {
            tokens.push(token(&format!("w{i}"), true, 1, None));
            tokens.push(token(&format!("w{i}"), true, 1, None));
        }
        let f = extract(&analysis(tokens), None, 1);
        let bag = f
            .features
            .iter()
            .filter(|x| x.kind == FeatureKind::Bag)
            .count();
        assert_eq!(bag, BAG_MAX_LEMMAS);
    }

    #[test]
    fn prev_uses_pos_name_when_grammar_given() {
        use crate::dict::{DenseGrammar, UnknownParams};
        let mut g = DenseGrammar::new(
            2,
            2,
            0,
            UnknownParams {
                left_id: 1,
                right_id: 1,
                pos_id: 0,
                cost_per_char: 1,
            },
        );
        let noun = g.add_pos("名詞,普通名詞,一般,*,*,*");
        let other = g.add_pos("名詞,固有名詞,一般,*,*,*");
        let a = analysis(vec![token("x", true, noun, None)]);
        let b = analysis(vec![token("x", true, other, None)]);
        let prev = |a: &AnalyzedContext, g: Option<&dyn Grammar>| {
            extract_named(a, g, None, 1)
                .features
                .into_iter()
                .find(|f| f.kind == FeatureKind::Prev)
                .unwrap()
                .hash
        };
        // 品詞1 が同じ（名詞）なので、品詞名で作ると一致する。pos_id で作ると別
        assert_eq!(prev(&a, Some(&g)), prev(&b, Some(&g)));
        assert_ne!(prev(&a, None), prev(&b, None));
    }
}
