# ADR 0001: フェーズ0のアーキテクチャと crate 間の契約

- 状態: 承認（2026-10-11 リード）
- 日付: 2026-10-11
- 対象: フェーズ0（spec「フェーズ0の作業」）。フェーズ1（文書内キャッシュ、文脈付き学習、ユーザー辞書、Rewriter）を、作り直しなしで足せる形にする
- 前提: `docs/ikari-ime_spec.md` v0.1、SudachiDict Core 20260723（`data/raw/sudachi-20260723/`）

---

## 0. 現状の把握（設計の根拠になった事実）

### リポジトリ
- workspace は `core` / `cli` / `dict-builder` / `eval` の4つ。中身はまだ空。edition 2024。
- `.gitignore` は `/data/raw/` と `/data/build/` を除外している。ビルドした辞書の出力先は `data/build/dict/` にする。

### SudachiDict CSV（実データで確認した）
- 全行19列。列は `0:索引用見出し(正規化済み) 1:左ID 2:右ID 3:コスト 4:表記 5-10:品詞1..4,活用型,活用形 11:読み(カタカナ) 12:正規化表記 13:辞書形ID 14:分割タイプ 15:A分割 16:B分割 17:語構成 18:同義語グループ`
- **表記は列4を使う**。列0は小文字化などの正規化がされている（例: 列0 `& co.,ltd.` に対し列4 `& Co.,Ltd.`）。
- カンマは `,` / `,` でエスケープされているので、`split(',')` で19列に分けられる。表記と読みに残る `\uXXXX` は builder で戻す（約480行）。
- 左ID = -1 の行が 49,286 件ある（例: `.com`）。これらは分割要素専用で、Sudachi も索引に入れていない。**除外する**。
- 単語コストは -14945..=32767、連接コストは -16961..=8327。どちらも **i16 に収まる**。
- 読みがカタカナ以外を含む行が 8,752 件ある（`キ・タ・ナ・イ`、`profile` など）。読みの最大長は612文字。
- 品詞の6列の組は 1,558 種類。品詞1の分布（small_lex）では動詞が43万件と最も多い（活用形ごとに1行ずつあるため）。空白は1件、補助記号は833件。
- (表記, 読み, 左, 右) で重複を除くと 1,629,907 行が 1,619,908 行になる（同じ組でコストだけ違う行がある）。
- 「ひんし」の候補: 品詞 7392、瀕死 7403、浜死 7403、賓師 7404、ひん死 8758、瀕し（動詞の未然形・連用形）、貧し、ひんし（瀕するのひらがな表記）。どちらも名詞・普通名詞・一般（ID 5146）。**完了条件の「品詞と瀕死が候補に出る」は、ベースのコストだけで満たせる見込み。**
- 「しよう」には `しよう`（為る 意志推量形 1820/7517）と `しよう`（表記はひらがな、名詞「仕様」の別表記、8626）と `仕様` 6587 がある。
- `matrix.def` の先頭行は `5981 5981`。以降の行 `a b c` は「前の語の右ID a → 次の語の左ID b の連接コスト c」（MeCab と Sudachi の規約）。BOS/EOS は ID 0。i16 で持つと 5981² × 2B ≈ **71.5MB**。

---

## 1. 決定の要約

| 項目 | 決定 |
| --- | --- |
| crate 構成 | **`ikari-dict-format` を新設する**（`dict-format/`）。core と dict-builder の両方がこれに依存し、互いには依存しない |
| 索引 | **fst crate**（0.4.7）。読みはひらがなに正規化した UTF-8 をキーにし、値は「エントリ配列の範囲」 |
| ファイル | 辞書ディレクトリに `grammar.bin`（連接表と品詞表）と `system.dict`（索引、エントリ、文字列）を置く |
| 読みの正規化 | **する**。カタカナをひらがなに変換して索引を作る。クエリ側も同じ関数を通す |
| 語ID | `WordId { dict: u8, index: u32 }`。index は辞書ファイル内のエントリ配列の添字。永続化には使わない（spec どおり） |
| 変換API | `Converter::convert(reading, &Context) -> Conversion`、`Conversion::page(i)`。候補はページ単位で遅延生成し、返したページは固定する |
| Ranker | `score` を調整する trait。並べ替えは core が行う。ベース実装の `CostRanker` は何もしない |
| 抽象化 | `dyn Dictionary` と `dyn Grammar`（ジェネリクスにはしない） |
| 依存 | fst、memmap2、clap（bin のみ）、tempfile（dev のみ）。これ以外は入れない |

---

## 2. crate 構成と依存の向き

```
ikari-dict-format   (dict-format/)  バイナリ形式の定義・検証・エンコード・読みの正規化。IOなし。依存: fst
   ^          ^
   |          |
ikari-core   ikari-dict-builder (lib + bin "ikari-dict-builder")
   ^   ^          ^ (dev-dependency)
   |   |          |
   |  ikari-eval (lib + bin "ikari-eval")
   |
ikari-cli (bin "ikari")  -- dev-dependency: ikari-dict-builder（統合テストで fixture 辞書を作る）
```

- **判断: `ikari-dict-format` を切る。** 理由は3つ。
  1. 形式の契約が1か所にまとまる。dict-builder は変換エンジン（ラティスなど）を必要としない。
  2. spec の「辞書の更新プロセス」（通信をする独立プロセス。ダウンロードしてビルドする）と、スマホ版がそのまま再利用できる。更新プロセスが core に依存すると、キー入力を扱うコードと通信するコードが同じバイナリに入りうる。spec の分離方針に反する。
  3. 並行実装でレーンB（形式）とレーンC（core のラティス）の編集範囲が分かれる。
- 循環依存はない。core は dict-builder に依存しない。

---

## 3. バイナリ辞書フォーマット（dict-builder と core の共有契約）

### 3.1 共通規約
- **リトルエンディアン固定**。読み書きは `u16::from_le_bytes` などのアクセサで行い、構造体の transmute（`unsafe`）は使わない。bytemuck と zerocopy も入れない。
- 各セクションの開始位置は **8バイト境界**。隙間は 0 で埋める。
- 同じ入力からは**バイト単位で同じファイル**を出す（決定的にする）。ファイルにタイムスタンプは入れない。
- **バージョン互換は持たない**。`format_version` が一致しなければエラーにして、ビルドし直してもらう。辞書は再生成できるので、互換コードという負債を持たない。

### 3.2 定数（`ikari_dict_format`）
```rust
pub const DICT_MAGIC: [u8; 8] = *b"IKARIDIC";
pub const GRAMMAR_MAGIC: [u8; 8] = *b"IKARIGRM";
pub const FORMAT_VERSION: u32 = 1;
pub const DICT_HEADER_LEN: usize = 96;
pub const GRAMMAR_HEADER_LEN: usize = 80;
pub const ENTRY_SIZE: usize = 16;
pub const SECTION_ALIGN: usize = 8;
pub const MAX_READING_CHARS: usize = 32;   // 索引に入れる読みの最大文字数。ラティスの最大語長も同じ値
pub const GRAMMAR_FILE_NAME: &str = "grammar.bin";
pub const SYSTEM_DICT_FILE_NAME: &str = "system.dict";
pub const DICT_KIND_SYSTEM: u32 = 0;
pub const DICT_KIND_EXTENSION: u32 = 1;    // 分野別の語彙パック用（フェーズ1以降）
```

### 3.3 `grammar.bin`（連接表と品詞表）
**判断: 連接表と品詞表を辞書本体から分けて1ファイルにする。** 左右IDと品詞IDは「文法のID空間」で、システム辞書、拡張語彙パック、ユーザー辞書がこれを共有する。辞書ごとに 71MB の表を複製しないため。

ヘッダ（80バイト）:

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | [u8;8] | magic `IKARIGRM` |
| 8 | u32 | format_version = 1 |
| 12 | u32 | header_len = 80 |
| 16 | u64 | **grammar_id**（品詞表と連接表のバージョン。後述） |
| 24 | u16 | prev_right_size（matrix.def 先頭行の1つ目） |
| 26 | u16 | next_left_size（先頭行の2つ目） |
| 28 | u16 | bos_eos_id（Sudachi では 0） |
| 30 | u16 | unk_left_id |
| 32 | u16 | unk_right_id |
| 34 | u16 | unk_pos_id |
| 36 | i16 | unk_cost（未知語1文字あたりのコスト） |
| 38 | u16 | 予約 0 |
| 40 | u32 | pos_count |
| 44 | u32 | 予約 0 |
| 48 | u64 | matrix_offset |
| 56 | u64 | matrix_len（= prev_right_size × next_left_size × 2） |
| 64 | u64 | pos_offset |
| 72 | u64 | pos_len |

- **連接表**: i16 LE の密行列。`cost(prev_right, next_left) = matrix[next_left * prev_right_size + prev_right]`（MeCab と同じ並び）。
  - matrix.def の検証（builder）: 行数が `A*B` とちょうど一致すること、各行で `a < A`、`b < B`、組に重複がないこと（ビットマップで確認）。ダウンロードしたファイルは信用しない入力として扱う、という spec の方針に合わせる。
- **品詞表**: `u32 offsets[pos_count + 1]`（レコード領域の先頭からの相対位置）を置き、続けてレコード領域を置く。レコードは `u16 rep_left_id, u16 rep_right_id, UTF-8 品詞名`。
  - 品詞名は CSV の5〜10列を `,` で結合したもの（例: `名詞,普通名詞,一般,*,*,*`、`動詞,一般,*,*,五段-ラ行,終止形-一般`）。活用型と活用形も含める。
  - pos_id は品詞名を辞書順に並べたときの添字。品詞の集合は、除外した行を含む**全入力行**から作る（拡張パックでも同じ ID になるように）。
  - `rep_left_id` と `rep_right_id` は、その品詞で最も頻度の高い (左, 右) の組（同数なら小さい方）。**フェーズ1のユーザー辞書で、品詞の指定から左右IDを決めるのに使う。** 今入れておけば形式バージョンを上げずに済み、builder にとっては数行の追加で済む。
- **grammar_id** = FNV-1a 64 を `prev_right_size, next_left_size, bos_eos_id`（LE）、連接表セクション、品詞表セクションのバイト列の順にかけた値。builder が一度だけ計算する。FNV は10行程度なので依存は増やさない。
- **未知語のパラメータ**: builder は `--unk-pos`（既定 `名詞,普通名詞,一般,*,*,*`）の pos_id と代表左右IDを使う。`--unk-cost` の既定は 10000（フェーズ1で評価して調整）。指定した品詞が見つからなければビルドを失敗させる。

### 3.4 `system.dict`（索引、エントリ、文字列）
ヘッダ（96バイト）:

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | [u8;8] | magic `IKARIDIC` |
| 8 | u32 | format_version = 1 |
| 12 | u32 | header_len = 96 |
| 16 | u64 | grammar_id（対応する grammar.bin と一致しなければ読み込まない） |
| 24 | u32 | entry_count |
| 28 | u32 | kind（0=system, 1=extension） |
| 32 / 40 | u64 / u64 | index_offset / index_len（fst Map のバイト列） |
| 48 / 56 | u64 / u64 | entries_offset / entries_len（= entry_count × 16） |
| 64 / 72 | u64 / u64 | strings_offset / strings_len（表記の UTF-8 を連結したもの） |
| 80 / 88 | u64 / u64 | meta_offset / meta_len（UTF-8 の `key=value\n`。source, builder_version, 件数など。日時は入れない） |

**索引（fst::Map）**
- キー: 正規化した読み（ひらがなの UTF-8 バイト列）。重複はない。
- 値: `(first_entry_index as u64) << 32 | count as u64`。エントリは `(読み, cost, 表記, left, right, pos)` の昇順に並べるので、同じ読みのエントリは連続した範囲になり、範囲内はコスト昇順。

**エントリ（16バイト固定）**

| off | 型 | フィールド |
| --- | --- | --- |
| 0 | u32 | surface_offset（strings セクション内の位置） |
| 4 | u16 | surface_len（バイト数） |
| 6 | u16 | left_id |
| 8 | u16 | right_id |
| 10 | i16 | cost |
| 12 | u16 | pos_id |
| 14 | u16 | flags（予約、0） |

- 語ID はエントリの添字。読みはキーから分かるので、エントリには持たせない。
- 同じ表記の文字列は1回だけ格納する（builder 内で HashMap を使い、最初に出た位置を使う。順序が決まっているので出力は決定的）。
- 規模の見積もり: 約157万エントリで、エントリ部 約25MB、文字列 約15〜20MB、fst 約10MB。

### 3.5 索引に fst を選んだ理由（crawdad との比較）

| 比較軸 | fst | crawdad |
| --- | --- | --- |
| mmap でのゼロコピー | `fst::Map<D: AsRef<[u8]>>` に mmap の部分スライスをそのまま渡せる | 逆シリアライズでノード配列を所有 Vec にコピーする。常駐メモリが増える |
| 完全一致・共通接頭辞検索 | 完全一致は `get`。共通接頭辞検索は `raw::Fst` のノードをバイト単位でたどって実装（数十行） | 両方とも組み込み |
| 前方一致（スマホの補完） | `Automaton`（`Str::starts_with`）でできる | 弱い |
| 値 | u64。範囲を詰められる | u32 |
| 速度 | 1遷移あたりはダブル配列より遅い | 速い |

**判断: fst を使う。** 速度の差は問題にならない。1回の変換で引くのは「開始位置数（≤読みの長さ）× 最大96バイト」程度で、マイクロ秒単位に収まる。spec が重視しているのは mmap と iOS のメモリ上限で、そこで決定的に有利なのは fst。

### 3.6 読みの正規化（`ikari_dict_format::reading`、索引の契約の一部）
```rust
/// カタカナ U+30A1..=U+30F6 を -0x60 してひらがなにする（ヴ→ゔ、ヵ→ゕ、ヶ→ゖ を含む）。
/// ヽ→ゝ、ヾ→ゞ。長音記号「ー」(U+30FC) はそのまま。それ以外の文字（記号、ASCII）もそのまま。
/// 全域関数で、失敗しない。クエリ側（core::Converter）でも必ずこれを通す。
pub fn normalize_key(s: &str) -> String;

/// 索引に入れられる読みか。1..=MAX_READING_CHARS 文字で、全文字が ぁ..=ゖ、ゝ、ゞ、ー のいずれか。
pub fn is_indexable(s: &str) -> bool;

/// builder 専用。読みから区切り記号「・」(U+30FB)、「＝」(U+FF1D)、「゠」(U+30A0)、空白(U+0020, U+3000) を取り除く。
pub fn strip_reading_separators(s: &str) -> String;
```
- **判断: 読みはひらがなに正規化して索引を作る。** 入力（ローマ字やかな）が出すのはひらがなで、カタカナ語も「せぶんいれぶん」と打つため。正規化の関数を format crate に置くのは、「キーの作り方」が builder と core の両方に効く契約だから。

### 3.7 dict-builder の行フィルタ（決め切った規則）
上から順に適用し、理由ごとに件数を BuildReport に記録する。
1. 列数が19未満、または数値列が parse できない → エラー（行番号付き）でビルドを止める。
2. `left_id == -1` → 除外（分割要素専用の行）。
3. 品詞1が `空白` または `補助記号` → 除外。句読点などが「てん」→「、」のように一般語と競合するのを防ぐ。記号は「読みから記号」の Rewriter（フェーズ1以降）で扱う。`記号` は残す。
4. 表記と読みの `\uXXXX` を戻す。読みに `strip_reading_separators` を適用し、続けて `normalize_key` を適用する。
5. `!is_indexable(読み)` → 除外（英字を含む読み、33文字以上の読み）。
6. コストは i16 のまま使う。左右IDが grammar の範囲外ならエラーにする。
7. (読み, 表記, left, right, pos_id) で重複を除き、最小コストの行を残す。

`--lex` は複数指定でき、small と core を連結して1つの system.dict にする。Full 版の notcore を拡張パックとして別ファイルにするのはフェーズ1以降。

---

## 4. core のモジュール構成と公開API

```
core/src/
  lib.rs              [契約] mod 宣言と re-export
  context.rs          [契約] Context, Commit
  candidate.rs        [契約] Candidate, Segment, CandidateOrigin
  dict/mod.rs         [契約] Dictionary, Grammar, WordEntry, WordId, UnknownParams, DictError, re-export
  dict/system.rs      [B] SystemDictionary（mmap + fst）, open_dir, LoadedDict
  dict/grammar_file.rs[B] GrammarFile（mmap した連接表と品詞表）
  dict/set.rs         [B] DictionarySet（システム辞書とユーザー辞書などを合成）
  dict/memory.rs      [C] InMemoryDictionary, DenseGrammar（テスト用。フェーズ1のユーザー辞書の土台にもなる）
  input/mod.rs        [A] re-export
  input/romaji.rs     [A] RomajiTable, RomajiComposer
  input/kana_jis.rs   [A] JisKey, JisKanaComposer
  lattice.rs          [C] ラティス構築と前向き Viterbi（crate 内部）
  nbest.rs            [C] 後ろ向き A* による N-best（crate 内部）
  ranker/mod.rs       [契約] Ranker trait, RankInput
  ranker/cost.rs      [C] CostRanker
  converter.rs        [契約の型シグネチャ → 本体は C] Converter, Conversion, Page, ConvertOptions, ConvertError
core/assets/romaji.tsv [A] 同梱の既定ローマ字表
```
（[契約] のファイルはリードが「契約コミット」で作り、以後は凍結する。§9）

### 4.1 dict（契約）
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WordId { pub dict: u8, pub index: u32 }   // 単体の辞書は dict=0 で返し、DictionarySet が付け替える

#[derive(Clone, Copy, Debug)]
pub struct WordEntry<'a> {
    pub id: WordId,
    pub surface: &'a str,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
    pub pos_id: u16,
}

pub trait Dictionary: Send + Sync {
    /// key の先頭から始まり、読みが key の接頭辞と完全一致するエントリをすべて列挙する。
    /// f(一致した読みのバイト長, entry)。同じ読みの中ではコスト昇順。読みは MAX_READING_CHARS 以下。
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>));
    /// 読みが完全一致するエントリを列挙する。
    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>));
    /// 語IDからエントリを引く。範囲外や壊れたエントリなら None。
    fn entry(&self, id: WordId) -> Option<WordEntry<'_>>;
    fn grammar_id(&self) -> u64;
}

#[derive(Clone, Copy, Debug)]
pub struct UnknownParams { pub left_id: u16, pub right_id: u16, pub pos_id: u16, pub cost_per_char: i16 }

pub trait Grammar: Send + Sync {
    /// ID が範囲外なら i16::MAX を返す（その接続は事実上禁止。パニックしない）。
    fn connection_cost(&self, prev_right_id: u16, next_left_id: u16) -> i16;
    fn bos_eos_id(&self) -> u16;
    fn unknown(&self) -> UnknownParams;
    fn pos_name(&self, pos_id: u16) -> Option<&str>;   // "名詞,普通名詞,一般,*,*,*"
    fn grammar_id(&self) -> u64;
}

#[derive(Debug)]
pub enum DictError {
    Io(std::io::Error),
    Format(ikari_dict_format::FormatError),   // マジック、バージョン、範囲外のオフセット、fst の破損など
    GrammarMismatch { dict: u64, grammar: u64 },
}
// std::error::Error と Display は手書きする（thiserror は使わない）

pub use system::{open_dir, LoadedDict, SystemDictionary};
pub use grammar_file::GrammarFile;
pub use set::DictionarySet;
pub use memory::{InMemoryDictionary, DenseGrammar};
```

**B の実装（シグネチャを固定する）**
```rust
pub struct LoadedDict { pub dict: Arc<SystemDictionary>, pub grammar: Arc<GrammarFile> }
pub fn open_dir(dir: &Path) -> Result<LoadedDict, DictError>;   // grammar.bin と system.dict を開き、grammar_id を照合する
impl SystemDictionary { pub fn open(path: &Path) -> Result<Self, DictError>; pub fn meta(&self) -> &str; }
impl GrammarFile      { pub fn open(path: &Path) -> Result<Self, DictError>; }
impl DictionarySet    { pub fn new() -> Self; pub fn push(&mut self, d: Arc<dyn Dictionary>) -> Result<u8, DictError>; }
// DictionarySet は Dictionary を実装する。push 時に grammar_id を照合し、検索結果を全辞書分合わせて返し、WordId.dict を付け替える
```
- 実装上の注意: fst の Map は mmap の部分範囲を所有する必要がある。`struct Section { mmap: Arc<Mmap>, range: Range<usize> } impl AsRef<[u8]>` を作り、`fst::Map<Section>` にする（自己参照を避けるため）。
- 開くときの検証は O(1) のものだけにする（ヘッダ、セクション範囲、`entries_len == entry_count*16`、`fst::Map::new` の成否）。エントリは読むたびに範囲付きスライスと `str::from_utf8` で確かめ、不正なエントリは飛ばす。157万件を全部走査する検証は builder 側（`--verify`）に置く。
- mmap は `unsafe`。安全性の前提「マップ中のファイルは変更されない」をコメントに書く。辞書の更新は別ファイルを作って rename で切り替える（spec の切り替え方針と一致する）。

**C の実装（テスト用、兼フェーズ1の土台）**
```rust
impl InMemoryDictionary {
    pub fn new(grammar_id: u64) -> Self;
    pub fn insert(&mut self, reading: &str, surface: &str, left_id: u16, right_id: u16, cost: i16, pos_id: u16);
}
impl DenseGrammar {
    pub fn new(prev_right_size: u16, next_left_size: u16, default_cost: i16, unknown: UnknownParams) -> Self; // bos_eos_id=0
    pub fn set(&mut self, prev_right: u16, next_left: u16, cost: i16);
    pub fn add_pos(&mut self, name: &str) -> u16;
}
```

### 4.2 input（レーンA）
```rust
// ローマ字表: Mozc 互換の TSV。「入力<TAB>出力[<TAB>次の未確定]」。# で始まる行と空行は無視する
pub struct RomajiTable { /* 入力文字列のトライまたはソート済み Vec */ }
impl RomajiTable {
    pub fn parse(tsv: &str) -> Result<Self, TableError>;   // 列数の不正や入力の重複はエラー（行番号付き）
    pub fn builtin() -> Self;                              // include_str!("../../assets/romaji.tsv")
}
#[derive(Debug)] pub struct TableError { pub line: usize, pub message: String }

pub struct RomajiComposer<'t> { /* table, kana: String, pending: String */ }
impl<'t> RomajiComposer<'t> {
    pub fn new(table: &'t RomajiTable) -> Self;
    pub fn push(&mut self, c: char);
    pub fn backspace(&mut self) -> bool;   // まず pending から1文字削る。空なら kana の末尾を削る
    pub fn kana(&self) -> &str;            // 確定したかな
    pub fn pending(&self) -> &str;         // 未確定のローマ字（FR-07: 変換対象にしない）
    /// 変換開始時に呼ぶ。pending が表の入力と完全一致すれば出力に置き換える（例: 末尾の "n" → ん）。
    /// 一致しなければ pending を残す（"へんk" の "k"）。
    pub fn flush(&mut self);
}
pub struct Composition { pub kana: String, pub pending: String }
pub fn romaji_to_kana(table: &RomajiTable, input: &str) -> Composition;   // push をすべて行ってから flush する

// JISかな入力
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JisKey { Digit1, /* .. */ Digit0, Minus, Caret, Yen, At, BracketLeft, Semicolon, Colon,
                  BracketRight, Comma, Period, Slash, Backslash /* ろ */, A, /* .. */ Z }
impl JisKey { pub fn from_legend(c: char) -> Option<JisKey>; }   // JIS 配列の無シフト刻印（'¥' はー、'\\' はろ）
pub struct JisKanaComposer { /* kana: String */ }
impl JisKanaComposer {
    pub fn new() -> Self;
    pub fn push(&mut self, key: JisKey, shift: bool);   // ゛(@) と ゜([) は直前のかなと合成する（か→が、は→ぱ、う→ゔ）
    pub fn backspace(&mut self) -> bool;
    pub fn kana(&self) -> &str;
}
```
**ローマ字の変換規則**（Mozc と同じ。未確定バッファを p、次の文字を c、s = p + c とする）
1. 表に s で始まり s より長い入力がある → p = s（まだ決めない）。ただし s 自体も完全一致する場合も、より長い入力があれば待つ（"n" と "na"）。
2. s が完全一致する → 出力を確定し、p = その行の「次の未確定」（"kk" → っ、p="k"）。
3. どれにも当たらない → p が完全一致すれば p の出力を確定して c を処理し直す（"nk" → ん + "k"）。そうでなければ p の先頭1文字を ASCII のまま確定して、残りを処理し直す。

- 既定の表（`core/assets/romaji.tsv`）には、一般的な Mozc の表に加えて `- → ー`、`, → 、`、`. → 。`、`[ → 「`、`] → 」`、`/ → ・`、`~ → 〜` と、**z＋キーの行** `zh ←`、`zj ↓`、`zk ↑`、`zl →`、`z. …`、`z- 〜`、`z/ ・`、`z[ 『`、`z] 』` を入れる。
- ユーザーの表は**丸ごと置き換える**（マージしない）。同梱の表をテンプレートとしてコピーしてもらう。Mozc と同じ挙動で、予測しやすい。
- 大文字（Shift＋英字で英数）はフェーズ2の殻で扱う。フェーズ0では表に当たらない文字として ASCII のまま通す。
- JIS かなでは z＋キーを使えない（spec どおり）。JIS の配列表はコードの定数にする（標準配列なので、ユーザーが編集できる必要はない）。

### 4.3 文脈と候補（契約）
```rust
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub left_text: String,       // カーソルより前の文脈（周辺テキストと確定履歴を合わせたもの）。末尾がカーソルの直前
    pub right_text: String,      // 補助（フェーズ0では使わない）
    pub history: Vec<Commit>,    // 同じ入力欄・同じセッションの確定履歴（古い順）
    pub secure: bool,            // パスワード欄。true なら Converter が文脈を空にしてから Ranker に渡す（NFR-18）
}
impl Context {
    pub fn new() -> Self;
    pub fn with_left_text(self, s: impl Into<String>) -> Self;
    pub fn with_history(self, h: Vec<Commit>) -> Self;
    pub fn with_secure(self, secure: bool) -> Self;
}
#[derive(Clone, Debug)] pub struct Commit { pub reading: String, pub surface: String }

#[derive(Clone, Debug)]
pub struct Segment {
    pub reading: String,
    pub surface: String,
    pub word: Option<WordId>,   // 未知語ノードなら None
    pub left_id: u16,
    pub right_id: u16,
    pub pos_id: u16,            // 未知語は UnknownParams.pos_id
}
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOrigin { Dictionary, Hiragana, Katakana }   // フェーズ1で Rewriter { name: &'static str, format: u16 } を追加する

#[derive(Clone, Debug)]
pub struct Candidate {
    pub surface: String,
    pub segments: Vec<Segment>,   // 語の分割（FR-06。「へんかんする」なら 変換 + する）
    pub base_cost: i32,           // パス全体のコスト（単語コストと連接コスト）
    pub score: i32,               // Ranker が付けた最終スコア。小さいほど上位。初期値は base_cost
    pub origin: CandidateOrigin,
}
```
- 判断: 学習とプロファイルの制約（「内容語だけ加点する」）には品詞が必要なので、`Segment` に pos_id と左右IDを持たせる。

### 4.4 Ranker（契約と C）
```rust
pub struct RankInput<'a> {
    pub reading: &'a str,
    pub context: &'a Context,   // secure なら空にしたもの
    pub grammar: &'a dyn Grammar,
}
pub trait Ranker: Send + Sync {
    /// cands の score を調整する。並べ替えは Converter が score の安定ソートで行う。
    /// 呼ばれるのは「まだ表示していない候補の塊」だけ（表示済みのページは固定する。NFR-05）。
    fn rank(&self, input: &RankInput<'_>, cands: &mut [Candidate]);
    fn name(&self) -> &str;
}
pub struct CostRanker;   // 何もしない（score = base_cost のまま）
```
- **判断: Ranker は「並べ替えた結果を返す」のではなく「score を調整する」形にする。** フェーズ1で `CompositeRanker`（文書内キャッシュ → 学習 → プロファイル加点を順に適用）を作るとき、各 Ranker が加減点を足し込むだけで合成できる。表示済みページの固定（NFR-05）の責任を core の1か所にまとめられる。
- プロファイル加点の上限や「文法で決まる箇所は覆さない」といった制約は、Ranker の実装側で segments の pos_id を見て守る（フェーズ1）。

### 4.5 Converter（契約の型。本体は C）
```rust
#[derive(Clone, Debug)]
pub struct ConvertOptions {
    pub page_size: usize,       // 1..=9。既定 9（番号ラベル 1〜9）
    pub pool_size: usize,       // 最初に生成して Ranker に渡す候補の数。既定 30
    pub chunk_size: usize,      // 2ページ目以降で追加生成する単位。既定 page_size
    pub max_astar_pops: usize,  // 1回の追加生成で A* が展開する上限。既定 20_000（遅延の保護）
}
#[derive(Debug)] pub enum ConvertError { GrammarMismatch { dict: u64, grammar: u64 } }

pub struct Converter { /* Arc<dyn Dictionary>, Arc<dyn Grammar>, Arc<dyn Ranker>, ConvertOptions */ }
impl Converter {
    pub fn new(dict: Arc<dyn Dictionary>, grammar: Arc<dyn Grammar>, ranker: Arc<dyn Ranker>,
               opts: ConvertOptions) -> Result<Self, ConvertError>;
    /// reading は normalize_key を通す。空なら候補0件の Conversion を返す。
    pub fn convert(&self, reading: &str, ctx: &Context) -> Conversion;
}
pub struct Conversion { /* Arc の複製を持つので 'static かつ Send。ラティス、A* の状態、確定済みの候補列、重複チェック用の表記集合 */ }
impl Conversion {
    pub fn reading(&self) -> &str;
    /// index ページ目（0始まり）。必要な分だけ遅延生成する。範囲外なら None。一度返したページの中身は二度と変わらない。
    pub fn page(&mut self, index: usize) -> Option<Page<'_>>;
    pub fn generated(&self) -> &[Candidate];
}
pub struct Page<'a> { pub index: usize, pub candidates: &'a [Candidate], pub has_next: bool }
```
- **判断: `Conversion` は Arc を持つ所有型にして、ライフタイム引数を持たせない。** フェーズ2の常駐エンジンで、IPC をまたいで変換状態を保持したり、読みの入力中に先行計算した結果をキャッシュしたり（NFR-03）するため。ラティスのノードは `WordId` と数値だけを持ち、表記は候補を作るときに `dict.entry(id)` で引く。
- **判断: ジェネリクス（`Converter<D, G>`）ではなく dyn にする。** 型引数が CLI、eval、エンジン、IPC 層まで広がるのを避けたい。仮想呼び出しの回数はラティス1つあたり数千回程度で、16ms の予算に対して無視できる。

### 4.6 候補生成のアルゴリズム（C が実装する仕様）
1. **キーの準備**: `key = normalize_key(reading)`。文字位置とバイト位置の対応表を作る。
2. **ラティス**: 各文字位置 i について、`common_prefix_search(&key[byte(i)..])` で辞書ノードを追加し、さらに**1文字の未知語ノードを必ず1つ**追加する（表記は元の文字のまま。ASCII や `←` もそのまま通る）。これでどの入力でも EOS まで必ずつながる。
3. **BOS**: `lattice::build(key, dict, grammar, bos_right_id)` のように引数で受け取る。フェーズ0では `grammar.bos_eos_id()` を渡す。**フェーズ1では左文脈の末尾を解析して、BOS の右IDを文脈の最後の語の右IDにする**（「〜に」+「なった」、「〜と」+「しよう」の文法の連接が効くようになる。§4.8 の ContextAnalyzer）。
4. **前向き Viterbi**: 各ノードについて、BOS からの最小コスト（i32）を求める。
5. **N-best**: EOS から後ろ向きに A* で探す。ヒューリスティックは前向きのコストで、正確な値なので取り出す順はコストの昇順になる。BOS に達したパスを1件の候補にし、**表記で重複を除く**（先に出た安い方を残す）。
6. **FR-06**: パスは必ず読み全体を覆うので、「へんか」から「変換」は構造上出ない。語の分割は複数ノードのパスとして自然に出る。
7. **最初のプール**: 辞書の候補を `pool_size` 件まで生成し、全体のひらがなとカタカナの候補（まだ出ていなければ）を `base_cost = プール内の最大 base_cost + 1` で加える。`score = base_cost` に初期化して Ranker に渡し、安定ソートしたものを確定済みの候補列にする。
   - ひらがなとカタカナの候補も Ranker を通すので、フェーズ1で機能語のひらがなを上げられる（FR-05）。フェーズ0では、機能語のひらがな候補は辞書にあるもの（例: 助詞「の」）として出る。
8. **次のページ**: 確定済みの候補列が足りなければ、A* を再開して `chunk_size` 件を追加生成し、その塊だけを rank してソートし、**末尾に追加する**。既存の順位は変えない（NFR-05）。A* が尽きるか `max_astar_pops` に達して新しい候補がなければ、その時点で打ち切る（`has_next = false`）。候補の総数そのものには上限を設けない（spec どおり）。
9. 最後のページでさらにスペースを押したときの挙動は殻が決める（spec の未決事項）。core は `None` を返すだけ。

### 4.7 その他の判断
- 変換の単位は読み全体の1文節（文節伸縮はしない）。候補はすべて読み全体を覆う表記になる。
- 未知語のコストは grammar のパラメータ（1文字あたり）。カタカナ語のまとまった未知語ノードはフェーズ1以降に回し、それまではカタカナ全体の候補で補う。

### 4.8 フェーズ1以降の差し込み口（名前と位置付けだけ）

| 名前 | 置く場所 | 位置付け |
| --- | --- | --- |
| `ContextAnalyzer` trait | `core/src/context_analyzer.rs` | 左文脈の末尾を解析して BOS の右IDを決める。表層から引く索引か sudachi.rs を trait で包む（spec の未決事項） |
| `CompositeRanker` | `ranker/composite.rs` | Ranker を順に適用する |
| `DocumentCacheRanker` | `ranker/doc_cache.rs` | 文書内キャッシュ |
| `LearningRanker` | `ranker/learning.rs` | 文脈付きの学習 |
| `ProfileRanker` | `ranker/profile.rs` | プロファイル加点 |
| `Rewriter` trait | `core/src/rewriter/` | `fn rewrite(&self, reading: &str, ctx: &Context, cands: &mut Vec<Candidate>)`。最初のプールを rank した後に呼び、辞書の上位候補より後ろに差し込む（spec どおり）。日付、記号、数字、電卓をそれぞれ独立に実装する。origin に `Rewriter { name, format }` を入れて、学習は書式IDで行う |
| `Conversion::commit(rank)` | 学習用のイベントを返す | `Learner` に渡す |
| ユーザー辞書 | `InMemoryDictionary` | TSV のローダーで作り、`DictionarySet` に push する。品詞の指定から左右IDを決めるのに `rep_left_id` / `rep_right_id` を使う |

---

## 5. CLI（`ikari-cli`、バイナリ名 `ikari`）

```
ikari [OPTIONS] <READING>

  --dict <DIR>          辞書ディレクトリ（grammar.bin と system.dict）。省略時は $IKARI_DICT_DIR、それもなければ ./data/build/dict
  --context <TEXT>      左文脈（確定済みのテキスト）。既定は ""
  --secure              パスワード欄として扱う（文脈を使わない）
  --input <MODE>        auto | romaji | kana | jis。既定は auto
                          auto:   READING がすべて ASCII なら romaji、そうでなければ kana
                          kana:   READING をそのまま読みとして使う（カタカナも可。normalize_key を通す）
                          jis:    JIS の無シフト刻印の列。'+' を前に付けると次のキーをシフト（例: "+e" → ぃ）
  --romaji-table <PATH> ローマ字表の TSV（省略時は同梱の表）
  -n, --num <N>         出力する候補の数（既定 9）。必要なページを順に取得する
  --page-size <N>       1..=9（既定 9）
  --format <FMT>        text | tsv（既定 text）
  --segments            text 形式で分割と品詞も表示する
  --timing              変換開始から1ページ目までの時間と、辞書を開く時間を stderr に出す
```
例: `ikari --dict data/build/dict --context "ユーザー辞書に単語を登録するときは" ひんし -n 10`

**text 形式（stdout）**
```
読み: ひんし
文脈: ユーザー辞書に単語を登録するときは
 1  品詞
 2  瀕死
 3  浜死
...
```
`--segments` を付けると、各行に `  [品詞/ひんし 名詞,普通名詞,一般,*,*,*]`（語ごとに ` + ` で連結）を付ける。ページの境目に `-- page 2 --` を入れる。

**tsv 形式（stdout。テストと eval 向け。ヘッダ行は出さない）**
```
rank<TAB>surface<TAB>score<TAB>base_cost<TAB>origin<TAB>segments
1	品詞	7392	7392	dictionary	品詞/ひんし/名詞,普通名詞,一般,*,*,*
```
segments は `表記/読み/品詞名` を ` + ` で連結する。origin は `dictionary` | `hiragana` | `katakana`。

- 未確定のローマ字が残ったら、stderr に `未確定: k（変換対象外）` と出す（FR-07 の確認用）。
- 終了コード: 0 は成功（候補0件も含む）、1 は辞書の読み込みエラーなど、2 は引数の誤り（clap の既定）。
- JSON 出力は今は作らない。serde を入れずに済ませる。必要になったら tsv から足す。

---

## 6. eval

### 6.1 評価セット（`eval/sets/homophones.tsv`、UTF-8、TSV）
- 1行目はヘッダで必須。**列は名前で対応付ける**ので、列の順序は自由で、知らない列は無視する（フェーズ1で列を足しても壊れない）。`#` で始まる行と空行は無視する。
- 必須の列は `id, kind, context, reading, expected`。任意の列は `tags`（カンマ区切り）、`profile`（空ならプロファイルなし）、`note`。値にタブと改行は入れられない。

```
id	kind	tags	profile	context	reading	expected	note
h-01	話題			ユーザー辞書に単語を登録するときは	ひんし	品詞
h-02	話題			敵の攻撃を受けて兵士は	ひんし	瀕死
h-03	話題			システムの	こうせい	構成
h-04	話題			原稿の誤字を	こうせい	校正
h-05	話題			質問への	かいとう	回答
h-06	話題			冷凍した肉を	かいとう	解凍
h-07	否定			要件をまとめてドキュメント化	しよう	しよう	文法優先。プロファイルで覆らないこと
h-08	プロファイル		it	外部APIの	しよう	仕様
h-09	機能語			この	の	の
a-01	話題	怒り		IMEの変換では単語の	ひんし	品詞	Google日本語入力は瀕死
a-02	学習	怒り		その話は	はじめてしった	初めて知った	学習が文脈を見ず別の文の選択が残った（学習の再現はフェーズ1）
a-03	文法	怒り		やっと春に	なった	なった	確定で左文脈が切れ「鳴った」になった
a-04	分割	怒り		一語ずつ確定しながら	かきすすめる	書き進める	辞書にない複合動詞を名詞＋動詞に分けた
a-05	否定	怒り		今日はここまでと	しよう	しよう	助詞の後ろの意志形を名詞で上書きした
```
- 怒りの記録の「ひんし」は、spec の評価セット表の h-01 とは文脈を変えた別のケース（a-01）として入れる。合計14件。
- 怒りの記録の文脈（「やっと春に」など）は、spec の「（〜に）」「（〜と）」を満たすように作った文。必要ならリードかユーザーが差し替える。

### 6.2 API（`ikari-eval` の lib）
```rust
pub struct EvalCase { pub id: String, pub kind: String, pub tags: Vec<String>, pub profile: Option<String>,
                      pub context: String, pub reading: String, pub expected: String, pub note: String }
pub fn load_cases(tsv: &str) -> Result<Vec<EvalCase>, EvalSetError>;   // EvalSetError { line, message }
pub struct EvalConfig { pub max_rank: usize /* 50 */, pub repeat: usize /* 20 */ }
pub struct CaseResult { pub case: EvalCase, pub rank: Option<usize> /* 1始まり */, pub top1: Option<String>,
                        pub cold: Duration, pub warm: Vec<Duration> }
pub fn run(conv: &Converter, cases: &[EvalCase], cfg: &EvalConfig) -> Vec<CaseResult>;
pub struct Summary { /* 全体、kind 別、tag 別の Group { n, top1, mrr }、遅延の統計 */ }
pub fn summarize(results: &[CaseResult]) -> Summary;
pub fn render_markdown(summary: &Summary, results: &[CaseResult], meta: &ReportMeta) -> String;
pub fn render_cases_tsv(results: &[CaseResult]) -> String;
```
- **計測する区間** = `convert()` から `page(0)` が返るまで（NFR-02 の「スペースから第一候補の表示まで」に相当）。各ケースの初回を cold として別に記録し、続けて `repeat` 回の warm を測る。
- **順位**: ページを順にたどって `expected` と表記が完全一致する位置を探す。`max_rank` を超えたら「見つからない」。
- **指標**: 1位正解率（全体、kind 別、tag 別）、MRR（見つからなければ 0）、遅延の p50、p99、最悪値（warm と cold それぞれ）。p99 は nearest-rank 法（昇順に並べて `ceil(0.99*n)-1` 番目）で求める。
- フェーズ0では `profile` 列を使わずに実行し、レポートに「profile 未対応」と書く。常駐メモリは測らず、その旨もレポートに書く。
- bin: `ikari-eval --dict <DIR> --set eval/sets/homophones.tsv [--repeat 20] [--max-rank 50] [--out-cases <path>]`。レポート（Markdown）は stdout に出す。debug ビルドで動いたら、遅延は参考値だと警告する。フェーズ0では正解率を問わないので、終了コードは常に 0（`--min-top1` による回帰判定はフェーズ1）。

### 6.3 レポートの形式
```markdown
# ikari-eval report
- set: eval/sets/homophones.tsv (14 cases) / dict: <meta の source> grammar_id=0x… / ranker: cost
- build: release / repeat: 20 / max_rank: 50 / profile: 未対応 / 常駐メモリ: 未計測

## 正解率
| 区分 | 件数 | 1位正解 | 1位正解率 | MRR |
|---|---|---|---|---|
| 全体 | 14 | 6 | 42.9% | 0.583 |
| kind:話題 | 7 | … | … | … |
| tag:怒り | 5 | … | … | … |

## 応答時間（convert〜1ページ目, ms）
| 対象 | サンプル数 | p50 | p99 | 最悪 |
|---|---|---|---|---|
| warm | 280 | … | … | … |
| cold（各ケースの初回） | 14 | … | … | … |

## ケース別
| id | kind | 文脈 | 読み | 期待 | 1位 | 順位 | warm p99 |
```

---

## 7. 依存クレート

| crate | 使う crate | 用途 | 理由 |
| --- | --- | --- | --- |
| `fst = "0.4.7"` | dict-format、core | 索引 | §3.5 |
| `memmap2 = "0.9"` | core | mmap | 事実上の標準。`advise` も使える（性能の節を参照） |
| `clap = { version = "4.6", features = ["derive"] }` | cli、dict-builder、eval の bin のみ | 引数解析 | bin が3つあり、オプションも多い。手書きの解析はバグを生む。core には入れない |
| `tempfile = "3"` | dev のみ | テストで辞書を一時ディレクトリに作る | |

使わないもの: serde、csv（Sudachi の CSV はクォートを使わないので split で足りる）、thiserror と anyhow（エラー型は手書きし、bin では `Box<dyn Error>` を使う）、bytemuck と zerocopy、criterion（フェーズ0の遅延は eval で測る）。

workspace の `Cargo.toml` に `[workspace.dependencies]` を作ってバージョンをまとめる。**依存はすべて契約コミットで追加し、Cargo.lock も確定させる**（並行作業で Cargo.lock が衝突するのを防ぐ）。

---

## 8. dict-builder（lib + bin）

```rust
pub struct BuildConfig { pub lex: Vec<PathBuf>, pub matrix: PathBuf, pub out_dir: PathBuf,
                         pub unk_pos: String, pub unk_cost: i16, pub label: String }
pub struct BuildReport { pub rows_read: usize, pub skipped: BTreeMap<&'static str, usize>,
                         pub entries: usize, pub readings: usize, pub pos_count: usize,
                         pub grammar_bytes: u64, pub dict_bytes: u64 }
pub fn build(cfg: &BuildConfig) -> Result<BuildReport, BuildError>;
```
bin: `ikari-dict-builder --lex data/raw/sudachi-20260723/small_lex.csv --lex data/raw/sudachi-20260723/core_lex.csv --matrix data/raw/sudachi-20260723/matrix.def --out data/build/dict [--unk-cost 10000] [--label sudachi-core-20260723]`

- 出力は `<out>/*.tmp` に書いてから rename する（途中で失敗しても壊れたファイルを残さない）。
- ビルド後の検証モード（`--verify <DIR>`）では全エントリを読み、範囲と UTF-8 を確かめる。
- エンコードは `ikari_dict_format::{encode_grammar, encode_dict}` を呼ぶ。format crate が「エンコード（`Vec<u8>` を返す）と、レイアウトの解析・検証（`&[u8]` から範囲を返す）」の両方を持つので、往復のテストを format crate の中で完結できる。

---

## 9. 並行実装の作業分割

### 9.1 契約コミット（ブランチを切る前にリードが入れる）
1. workspace に `dict-format` を追加する。§7 の依存を各 crate の `Cargo.toml` に書き、`[[bin]] name = "ikari"`（cli）、dict-builder と eval の lib と bin の構成も設定して、Cargo.lock を確定させる。
2. §4 で [契約] としたファイルを、本 ADR のシグネチャどおりに書く。`lib.rs` の mod 宣言もすべて入れる。
3. レーンが持つファイルにはシグネチャだけを置き、本体は `todo!()` にする。`converter.rs`、`dict/system.rs`、`dict/grammar_file.rs`、`dict/set.rs`、`dict/memory.rs`、`input/*`、`dict-format/src/lib.rs`（定数と型）、`dict-builder/src/lib.rs` が対象。
4. `ikari_dict_format::reading::{normalize_key, is_indexable, strip_reading_separators}` だけは**中身まで実装する**（20行程度。B と C の両方がすぐに使うため）。
5. `cargo build --workspace` が通ること（todo! のままでもよい）。

**ルール**: [契約] のファイルを変えたくなったら、リードに相談する（他のレーンを壊さないため）。依存を追加したいときも同じ。

### 9.2 レーンごとの持ち物

| レーン | 持つファイル（ここだけ編集する） | 最初に置くもの |
| --- | --- | --- |
| **A**: input + cli | `core/src/input/**`、`core/assets/romaji.tsv`、`cli/**` | 既定のローマ字表。CLI は「引数の解析」と「候補の整形（`&[Candidate]` → text/tsv）」を純粋関数（`cli/src/args.rs`、`cli/src/output.rs`）にして、手で作った `Candidate` でテストする。`main.rs` の配線（`open_dir` → `Converter`）は契約のシグネチャどおりに書いておく |
| **B**: 形式 + builder + 読み込み | `dict-format/**`、`dict-builder/**`、`core/src/dict/{system,grammar_file,set}.rs`、`testdata/sudachi-mini/**` | **fixture を最初に作る**（§10.2）。次に format の往復テスト |
| **C**: ラティス、Viterbi、Ranker + eval | `core/src/{lattice,nbest,converter}.rs`、`core/src/ranker/cost.rs`、`core/src/dict/memory.rs`、`eval/**` | **`InMemoryDictionary` と `DenseGrammar` を最初に作る**。これで B を待たずにすべてテストできる。eval の bin の `open_dir` 呼び出しはシグネチャどおりに書いておく |

### 9.3 マージの順序
1. B と C を先にマージする（編集するファイルが重ならないので順序は問わない）。
2. A は input を先に仕上げ、B と C のマージ後に rebase して、CLI の配線と統合テスト（§10.3）を通す。
3. リードがフェーズ0の完了条件を確認する（§10.4）。

---

## 10. テスト方針

### 10.1 レーンごとの単体テスト
- **A**
  - ローマ字: `ka→か`、`kka→っか`、`kanji→かんじ`、`kan'i→かんい`、`nn→ん`、`hen` を flush すると `へん`、`henk` は kana が `へん` で pending が `k`（FR-07）、`zh→←`、`z.→…`、`-→ー`。表の不正（列数、入力の重複）は行番号付きのエラーになること。backspace。
  - JIS: 基本のキー、シフトの小書き（`+e`→ぃ）、゛と゜の合成（か→が、は→ぱ、う→ゔ）、合成できない文字の後の ゛ はそのまま残る。
  - CLI: 引数の解析（auto の判定）、text と tsv の整形。
- **B**
  - format: 往復テスト（エンコード → 解析 → 全フィールドが一致）。マジック違い、バージョン違い、ファイルの切り詰め、範囲外のオフセットが**パニックせずエラーになる**こと。
  - builder: 各フィルタ規則（-1 の行、`,` の復元、カタカナ→ひらがな、`・` の除去、英字の読みの除外、33文字以上の除外、空白と補助記号の除外、最小コストでの重複除去）をインラインの CSV で確かめる。matrix.def は**縦横の大きさが違う 2×3 の表**で、転置のバグを検出する。行の不足と重複はエラーになること。
  - 読み込み: fixture の CSV から作った辞書で、`common_prefix_search` の結果が全数探索（brute force）と一致すること。grammar_id が違えば `GrammarMismatch` になること。決定性（同じ入力から同じバイト列が出ること）。
- **C**（すべて InMemory でテストする）
  - 手で決めたコストで、Viterbi の最良パスが期待どおりになる。N-best がコスト昇順で、表記の重複がない。
  - FR-06: 「へんか」から「変換」が出ない。「へんかんする」で「変換する」（変換 + する）が出る。
  - 辞書にない読みや、`←` を含む読みでも、ひらがなの候補が返る。
  - ページ: 2ページ目を取った後も1ページ目が変わらない。`has_next`。`page_size` を守る。
  - Ranker がプール単位で呼ばれる。`secure` なら文脈が空で渡る。テスト用の Ranker で score を変えると順位が変わる。
  - eval: TSV の解析（ヘッダによる列の対応付け、コメント行、必須列が欠けたらエラー）、指標の計算（top1、MRR、nearest-rank の p99）、モックの Converter でレポートが出ること。

### 10.2 fixture（`testdata/sudachi-mini/`、git 管理）
- `lex.csv`: 実際の Sudachi から評価セットの読みに関わる行を抜き出す（ひんしの一族、こうせいとかいとうの主要語、しよう、仕様、の、この、は、に、と、なった、鳴った、書き進める用の動詞、`名詞,普通名詞,一般` を含む行など、50〜150行）。表記、読み、品詞、コストは実データのまま使い、**左右IDだけを小さなID空間（0..16 程度）に付け替える**。
- `matrix.def`: 付け替えた ID 空間の 16×16 の全組み合わせ。コストは品詞クラスの大まかな値を手で決める。
- `README.md`: 出典（SudachiDict 20260723、Apache-2.0）、抜き出しと付け替えの手順。NOTICE 用の記載もここに書く。
- fixture での順位には意味がない。**候補に出るかどうかと、形式の往復だけを見る。**

### 10.3 統合テスト（CI で実辞書なしに動く）
- `cli/tests/phase0.rs`
  - `ikari_dict_builder::build` で fixture の辞書を tempdir に作る。
  - `env!("CARGO_BIN_EXE_ikari")` で CLI を起動する（assert_cmd は使わない）: `--dict <tmp> --context "ユーザー辞書に単語を登録するときは" --format tsv -n 10 ひんし`
  - stdout に「品詞」と「瀕死」が含まれることを確かめる。
- `eval/tests/report.rs`: fixture の辞書で `ikari-eval` を実行し、レポートに「正解率」と「応答時間」の節があることを確かめる。
- **実辞書のテスト**: `#[ignore]` を付け、`IKARI_DICT_DIR` が設定されているときだけ動かす（`cargo test -- --ignored`）。内容は同じく「ひんし」で品詞と瀕死が上位10件に入ることと、評価セット全体が動くこと。

### 10.4 フェーズ0の完了確認（リードが手で行う）
```
cargo test --workspace
cargo run --release -p ikari-dict-builder -- --lex data/raw/sudachi-20260723/small_lex.csv --lex data/raw/sudachi-20260723/core_lex.csv --matrix data/raw/sudachi-20260723/matrix.def --out data/build/dict
cargo run --release -p ikari-cli -- --dict data/build/dict --context "ユーザー辞書に単語を登録するときは" ひんし -n 10
cargo run --release -p ikari-eval -- --dict data/build/dict --set eval/sets/homophones.tsv
IKARI_DICT_DIR=data/build/dict cargo test --workspace -- --ignored
```

---

## 11. 性能（NFR-02: 16ms）

- **辞書を開く**: mmap、ヘッダの検証、`fst::Map::new`（O(1)）、品詞表（1,558件）の読み込みだけ。目標は 5ms 未満。連接表はコピーしない。
- **ラティス**: ノードは `Vec<Node>` のアリーナに置き、開始位置と終了位置の索引を添字で持つ。ラティスの中では String を作らない。表記を引くのは候補を作るときだけ。
- **計算量の見積もり**: 20文字の読みでノードは数百、連接表の参照は数万回程度。1回あたり数 ns なので、1ms を大きく下回る見込み。
- **1ページ目**: 前向き Viterbi と、A* で30件。2ページ目以降は差分だけを生成する（spec の「全件を一度に生成しない」）。A* は `max_astar_pops` で上限を設け、長い読み（FR-04 の長文）でも遅延が暴れないようにする。
- **cold の問題**: 71MB の連接表に飛び飛びでアクセスするため、初回はページフォルトが起きる。フェーズ0では eval で cold と warm を分けて見るだけにする。フェーズ2の常駐エンジンでは、起動時に `Mmap::advise(Advice::WillNeed)` を使うか、全ページに触れて温めておく。
- **メモリ**: 仮想アドレスで約130MB をマップするが、常駐するのは触れたページだけ。NFR-09 の予算（エンジン込みで1〜2GB）に対して十分に小さい。連接表の圧縮（左右IDの同値類で縮める）は、計測して必要になってから検討する。
- **並行性**: `Converter` は Send + Sync、`Conversion` は Send。読みの入力中に別スレッドで先行計算できる（NFR-03）。
- **計測はリリースビルドで行う**（eval が debug ビルドを警告する）。

---

## 12. 却下した案

| 案 | 却下した理由 |
| --- | --- |
| dict-builder が core に依存し、形式の定義を core に置く | 更新プロセス（通信する）が変換エンジンのコードを丸ごと抱える。並行実装で B と C が core の中で衝突する |
| crawdad | mmap でゼロコピーにできない（§3.5） |
| 連接表を辞書ファイルに同梱する | 拡張パックやユーザー辞書ごとに 71MB を複製することになる。文法のID空間は共有物 |
| 可変長エントリに読みを持たせる | 読みはキーから分かる。固定長16バイトの方が添字アクセスと検証が単純 |
| `Converter<D, G>` のジェネリクス | 型引数が CLI、eval、IPC まで広がる。性能の利点は無視できる |
| Ranker が並べ替えた結果を返す | 合成しにくい。表示済みページの固定（NFR-05）が Ranker ごとの責任になってしまう |
| 文脈を `&str` で渡す | フェーズ1で確定履歴と secure フラグが必要になり、API を壊すことになる |
| ユーザーのローマ字表を差分でマージする | ルールの優先順位が分かりにくい。Mozc と同じく丸ごと置き換える |
| serde と JSON で出力する | フェーズ0では TSV で足りる |

---

## 13. 懸念点と、確認が必要なこと

1. **補助記号の除外**（§3.7-3）: 「てん」→「、」などの競合を避けるための判断。記号の候補は Rewriter で出す前提。ユーザーの意図と合っているか確認したい。
2. **fixture に Sudachi の行を抜き出して入れる**: Apache-2.0 なので出典の記載で足りるはずだが、ライセンスの確認は人間が行う方針（spec）なので、ユーザーに一言確認してほしい。
3. **ひらがなとカタカナの候補をプールの末尾に置く**（§4.6-7）: フェーズ0での暫定。FR-05 の達成はフェーズ1の文脈（BOS の右IDと Ranker）に依存する。フェーズ0の評価で h-09（この＋の）が外れても、完了条件には反しない。
4. **未知語のコスト 10000** は仮の値。フェーズ1の評価で調整する。
5. **怒りの記録の文脈文**（§6.1 の a-01〜a-05）は私が作った。ユーザーの実体験に合わせて差し替えてよい。
6. **最後のページでの挙動と、前のページに戻る操作**は spec の未決事項のまま。core は `None` を返すだけにしているので、どちらに決めても core は変わらない。
7. 品詞の代表左右ID（`rep_*`）は、フェーズ1のユーザー辞書で動詞の活用形ごとの左右IDを決めるには足りない可能性がある（1つの品詞名に活用形まで含めているので、おおむね一意のはず）。フェーズ1の着手時に実データで確認する。
