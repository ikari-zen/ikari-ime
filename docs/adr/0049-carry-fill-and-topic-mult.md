# ADR 0049: 左文脈の持ち越しを既定で入れる（fill=40、区切りなし）と、足した前文の話題の効きを左の有無で分ける

- 状態: 採用（2026-11-04 にユーザーが値を決定: `fill = 40`、区切りなし、話題の倍率は「元の左が 0 字なら 1、それ以外は 0.5」。2026-11-04 に §10 の 1 も受け入れると決定: C5 の小さな悪化は許す）
- 日付: 2026-11-04
- 対象: `core/src/context_carry.rs`（`ComposedLeft` に足した字数、既定の値の置き場所）、`core/src/context.rs`（`Context` に 1 項目）、`core/src/analysis/mod.rs`（`AnalyzedContext` に境目、`extended` でずらす）、`core/src/converter/mod.rs`（`analyze` で境目を入れる）、`core/src/ranker/topic.rs`（ベクトルを 2 つに分ける、設定 2 項目）、`engine/src/session.rs`・`engine/src/context_limits.rs`（既定と、足した字数を渡す）、`engine/src/convert.rs`・`engine/src/cache.rs`（確定で先行計算を捨てる）、`eval/src/carry.rs`・`eval/src/run.rs`・`eval/src/scenario.rs`・`eval/src/args.rs`（既定を core から取る、足した字数を渡す）。`lm/`、shell-core、proto、Swift、学習の形式は変えない
- 前提: main = e71118c。ADR 0046（`compose_left`、§3.2 の擬似コード、§7 の派生セット C1〜C5 と carry-chat）、ADR 0047（`topic.decay = 0.8`）、ADR 0038（先行計算の当て方、`fill_left`）
- タスク: #182 の続き（ADR 0046 §7.2 の掃引の結果を既定にする）
- 根拠: `eval/reports/0046-fill-topic-sweep-2026-11-04.md`。使い捨ての実験の差分は `~/ikari-ime-archive/topic-prior-mult-experiment-2026-11-04.patch.xz`（参考。そのまま採らない。§4.4）
- 判断の置き場所: Rust の core（組み立てと話題の採点）と engine（どの材料を渡すか、先行計算の捨て方）。OS の API も UI も関わらないので Swift には何も足さない

---

## 0. 現状の把握（コードで確かめたこと）

- 左文脈は `compose_left`（`core/src/context_carry.rs`）が作る。engine は `Session::build_context`（`engine/src/session.rs`）から、eval は `eval/src/carry.rs` の `left_text` から、同じ関数を呼ぶ。shell-core は周辺テキストを送るだけで、組み立てない（A1 のセッションの ID だけ）。
- 既定は `fill = 0`（engine の `CARRY_FILL_CHARS`、eval の `NO_CARRY`）。`compose_left` の戻り値 `ComposedLeft` は `text` と `origin` だけで、**どこまでが足した前文かは呼び出し側に渡らない**。
- 足した前文は `left_text` の一部になり、話題（`TopicRanker`。解析窓のトークンから文脈ベクトルを 1 つ作る）、LM（末尾 40 字）、連接、文書内キャッシュに、元の左と区別なく入る。`topic.weight` などの設定は両者を区別できない（報告の A: 全体の weight を下げると、足さない基準も落ちる）。
- 解析のトークンの `char_end` は**左文脈の末尾からの距離**（`core/src/analysis/mod.rs`）。文節のモードでは `AnalyzedContext::extended`（`core/src/converter/segment.rs` から呼ぶ）が、前の文節の語を後ろに足し、既存のトークンの `char_end` を足した字数だけずらす。
- 実験の差分は、元の左の字数を static で渡し、`char_end >= 元の左の字数` のトークンを「足した前文」として別のベクトルにした。`extended` のずれを見ていない（文節のモードで境目が狂う）ほか、static なので並行の変換で壊れる。
- 先行計算（`engine/src/cache.rs`）: `CacheKey` は `session`・`reading`・`context_rev`。`context_rev` は shell-core が**周辺テキストが変わったときだけ**進める（`shell-core/src/machine/mod.rs` の `refresh_context`）。`find_by_content` は `PrefetchContext::same_content`（left、right、available だけ）で当てる。確定（`push_history`）で先行計算は捨てない（捨てるのは `drop_session_state` だけ）。

## 1. 問題の定義

1. `fill = 0` の既定では、送信で空になったチャットの欄（C1、carry-chat）で直前の自分の文が使われない。掃引で `fill = 40` が効くと分かった（LM あり C1 627 → 1120、chat 33 → 60）。
2. ただし、左に字がある欄で前文をそのまま足すと、無関係な前文の話題が害になる（LM あり C3 1120 → 1039、−81）。害の大部分は話題から来る（ADR 0015 §0.6 と同じ形）。
3. 足した前文の話題だけを弱めるには、話題の採点が「どこまでが足した前文か」を知る必要があるが、今の契約にはその情報がない。
4. `fill > 0` では、`available` の欄でも左文脈が確定履歴に依る。今の先行計算の当て方は、周辺テキストが同じなら同じ文脈とみなすので、確定の前の先行計算を確定のあとに使いうる（§5）。

## 2. 決定の要約

| # | 決定 |
|---|---|
| K1 | 既定を `fill = 40`、区切り `CarryJoint::None` にする（ユーザー決定）。既定の値は core の `CarryParams::DEFAULT` 1 か所に置き、engine と eval がそれを使う |
| K2 | `compose_left` は、`text` の先頭の何字が履歴から足した前文か（`carried_chars`）を返す。engine と eval は、それを `Context.left_carried_chars` に入れる |
| K3 | Converter は、解析の結果に境目（`carried_from`。末尾からの距離）を入れる。`extended` は境目も同じだけずらす |
| K4 | `TopicRanker` は、境目より手前（足した前文）と後ろ（元の左と、文節のモードで足した語）の内容語で、文脈ベクトルを別に作る。前文の加点に倍率 m をかける。m は「元の左が 0 字なら `topic.carry_mult_empty`（既定 1.0）、それ以外は `topic.carry_mult`（既定 0.5）」（ユーザー決定） |
| K5 | 倍率の入口は `--ranker-config` の `topic.*` だけ。`settings.tsv` には出さない（利用者の設定ではない） |
| K6 | engine は、確定（同じセッションの履歴に積んだとき）で、そのセッションの先行計算を捨てる。左文脈が履歴に依るようになったので、確定の前に作った計算を使わない |
| K7 | 連接、文書内キャッシュ、LM、学習は、足した前文を今どおり元の左と区別せずに見る（掃引はこの形で測った） |

## 3. 「元の左が 0 字」の定義

**元の左** = `left_text` のうち、履歴から足していない部分（`left_text` の字数 − `carried_chars`）。0 字かどうかで m を選ぶ。

| 場合 | `compose_left` の結果 | `carried_chars` | 元の左 | m |
|---|---|---|---|---|
| フィールドなし（eval の `field` 列なし。engine にはない） | `context` のまま（`compose_left` を通らない） | 0 | 全体 | 前文がないので効かない（今と同じ） |
| `Unavailable`、履歴あり | 履歴の末尾 2000 字（origin = History） | 全体 | 0 字 | `carry_mult_empty` = 1 → **今と同じ点数**（ベクトルが前文の 1 つだけになり、倍率 1） |
| `Unavailable`、履歴なし | 空 | 0 | 0 字 | 何もない |
| `Available(L)`、L が空白だけ、履歴あり（C1、C5） | 履歴 + 区切り（L は入らない） | 全体 | 0 字 | `carry_mult_empty` = 1 |
| `Available(L)`、L が空白だけ、履歴なし | L | 0 | 空白だけ | 前文がないので効かない |
| `Available(L)`、足した（origin = FieldWithHistory。C2、C3） | 前文 + 区切り + L | 前文 + 区切りの字数 | L（1 字以上） | `carry_mult` = 0.5 |
| `Available(L)`、足さない（十分に長い、一致しない C4、`fill = 0`） | L | 0 | L | 前文がないので効かない（今と同じ） |

- 判定は `carried_chars` と `left_text` の字数だけで決まり、`origin` や `field` を話題に渡さない。「空白だけ」の扱いは `compose_left` の今の判定（`trim().is_empty()`）に従う（空白だけの L は結果に入らないので、元の左は 0 字になる）。
- 実験は、`field = none` を 0 字、`available` を `context` の字数として扱った。上の表は、どの場合も実験と同じ m を選ぶ（空白だけの L は評価セットにない）。
- 区切り（「。」など）は足した前文の側に数える。今の既定は区切りなしなので、数値には効かない。

## 4. 契約（シグネチャ）

### 4.1 core: 組み立て（`core/src/context_carry.rs`）

```rust
impl CarryParams {
    /// 既定（ADR 0049 K1）。engine と eval の両方がこれを使う
    pub const DEFAULT: CarryParams = CarryParams { fill_chars: 40, joint: CarryJoint::None };
}
pub struct ComposedLeft {
    pub text: String,
    pub origin: LeftOrigin,
    /// `text` の先頭のうち、履歴から足した字数（区切りを含む）。History なら text 全体の字数、Field・Empty なら 0
    pub carried_chars: usize,
}
```

- 契約のコミットでは `DEFAULT` を `fill_chars: 0` にしておき、統合で 40 にする（§7）。
- `compose_left` のシグネチャは変えない。§3.3（ADR 0046）の単体テストに `carried_chars` の期待を足す。

### 4.2 core: 文脈と解析

```rust
// core/src/context.rs（Context は #[non_exhaustive] + Default。既定 0 = 足した前文なし = 今と同じ）
pub left_carried_chars: usize,
pub fn with_left_carried_chars(self, n: usize) -> Self;

// core/src/analysis/mod.rs
pub struct AnalyzedContext {
    // …今の項目…
    /// これ以上の `char_end` のトークンは、履歴から足した前文（末尾からの距離）。None = 前文なし
    pub carried_from: Option<u32>,
}
```

- `carried_from = left_text の字数 − left_carried_chars`（元の左の字数。0 もありうる）。`left_carried_chars == 0` なら None。
- 入れるのは Converter の `analyze`（`core/src/converter/mod.rs`）。解析器（`ContextAnalyzer`）とそのキャッシュは変えない。前文があるときだけ、キャッシュの `Arc` を複製して境目を入れる（解析窓は 2000 字までで、LM の時間に比べて小さい）。secure では None。
- `extended` は `carried_from` も `shift` だけずらす（`saturating_add`）。足した語（文節のモードの前の文節）は元の左の側に入る。
- 解析窓（末尾 2000 字）は `left_text` の上限と同じなので、境目が窓の外に出ることはない。出ても、前文のトークンがないだけで壊れない。

### 4.3 core: 話題（`core/src/ranker/topic.rs`）

```rust
pub struct TopicConfig {
    // …今の 6 項目…
    /// 元の左が 0 字のときの、足した前文の加点の倍率（`topic.carry_mult_empty`）
    pub carry_mult_empty: f32,   // 既定 1.0
    /// 元の左があるときの倍率（`topic.carry_mult`）
    pub carry_mult: f32,         // 既定 0.5（契約のコミットでは 1.0。§7）
}
```

- `CONFIG_KEYS` に 2 つ足す。値の検査は 0.0 以上（負はエラー）。
- 採点（実験と同じ式。式を変えたら掃引の数字は使えないので測り直す）:
  - `carried_from` が None なら、今の `context_vector` と採点のまま（ベクトル 1 つ）。
  - Some(b) なら、`char_end < b` の内容語から own、`char_end >= b` の内容語から prior を、今と同じ作り方（新しい順、`recent_tokens`、`decay`、`min_content`）で**それぞれ別に**作る。`total = w·max(0, cos_own − τ) + m·w·max(0, cos_prior − τ)` を候補の内容語ごとに足し、`max_total` で切る。m は `b == 0` なら `carry_mult_empty`、それ以外は `carry_mult`。
- 元の左が 0 字で m = 1 のとき、own は空なので、点数は今の 1 ベクトルの採点と同じになる（`Unavailable` の今の経路が変わらない）。
- `profile.rs`（属性プロファイル）は変えない。

### 4.4 実験の差分から採らないもの

- static（`OWN_CHARS`）と環境変数（`PRIOR_MULT`、`M_EMPTY`、`M_OTHER`）。境目は `Context` → `AnalyzedContext` で渡し、倍率は `TopicConfig` に置く。
- `pub mod topic` への公開の変更（eval から static に書くためだけのもの）。

### 4.5 engine（`engine/src/session.rs`、`engine/src/context_limits.rs`、`engine/src/convert.rs`、`engine/src/cache.rs`）

- `build_context`: `compose_left` の `carried_chars` を `ctx.left_carried_chars` に入れる。Prefetch（先行計算）も Convert のその場の変換も `context_for` → `build_context` を通るので、どちらも同じ境目になる。
- `CARRY_FILL_CHARS`・`CARRY_JOINT` の定数は消し、`ContextLimits::DEFAULT.carry` を `CarryParams::DEFAULT` にする（既定の置き場所を 1 つにする）。テスト `defaults_follow_adr_0015` の期待を改める。
- 確定で先行計算を捨てる（K6。§5）: 同じセッションの履歴に積む所（`push_history` を呼ぶ確定の経路）で、`queue.forget_session` と `cache.remove_session`（`drop_session_state` の先行計算の部分と同じ）を行う。文節の先読み（`lookahead`）はこの ADR では触らない。

### 4.6 eval（`eval/src/carry.rs`、`eval/src/run.rs`、`eval/src/scenario.rs`、`eval/src/args.rs`）

- `left_text` は `ComposedLeft`（または `(String, usize)`）を返し、`case_context` と scenario は `with_left_carried_chars` を入れる。`field` 列がないときは 0。
- `--carry-fill` と `--carry-joint` の既定は `CarryParams::DEFAULT`（engine と同じ）。`NO_CARRY` は `--carry-fill 0` の意味として残す。`eval/scripts/snapshot.sh` は引数を足さない（既定で engine と同じものを測る）。
- `--ranker-config` で `topic.carry_mult_empty`・`topic.carry_mult` を振れる（追加の配線は要らない。`TopicConfig::from_kv` が読む）。

## 5. 先行計算と実際の変換の食い違い

**分かったこと**（コード）:
- `available` の欄で `fill = 0` なら、左文脈は周辺テキストだけで決まる。今の当て方（同じ `context_rev`、または同じ left・right・available）はこれを前提にしている。
- `fill > 0` では、左が 40 字に満たないとき、左文脈が確定履歴に依る。確定しても周辺テキストが変わらなければ（送信で空に戻るチャットの欄など）、`context_rev` は進まず、left・right も同じ。同じ読みを前に打っていれば、確定の前の履歴で作った先行計算に、キーでも中身でも当たりうる。Prefetch はキーが当たると計算し直さない（`engine/src/convert.rs`）。
- `Unavailable` の欄（左文脈 = 履歴）では、今もキーでは当たりうる（`same_content` は false を返すが、`find` はキーだけを見る）。

**推測**（実機・テストで未確認）: 同じ欄で同じ読みを、確定をはさんで二度打つと、二度目に古い前文の変換が出る。`fill = 40` で、空になったチャットの欄では起こりやすくなる。

**対処**（K6）: 確定で、そのセッションの先行計算を捨てる。

| 案 | 内容 | 判断 |
|---|---|---|
| **P1（採用）: 確定で捨てる** | 履歴に積むときに、そのセッションの先行計算とキューを捨てる | 採用。「履歴が変わったら先行計算は無効」を 1 か所で表せる。確定の前の先行計算は、確定で合成が終わるので使い道がない（次の合成の Prefetch は確定のあとに来る）。`Unavailable` の欄の今の食い違いも同時に消える |
| P2: 履歴の番号をキーと `same_content` に足す | セッションに確定の回数を持ち、`CacheKey` と `PrefetchContext` に入れて比べる | 不採用。比べる所が 2 つに増え、ログの `miss` の理由も足す必要がある。得るものは P1 と同じ |
| P3: 合成した左文脈そのものを `PrefetchContext` に持って比べる | Convert のたびに `compose_left` を走らせて比べる | 不採用。Convert の当て方が組み立てに依存する。ADR 0038 の `filled`（Prefetch だけ left を補う）と比べ方が合わなくなる |

- 前提: 同じセッションの確定と次の Prefetch は、殻から順に届く（殻は確定のあとに次の合成を始める）。テストで「確定 → 同じ読みの Prefetch → Convert」が新しい履歴で作られることを確かめる（engine の `tests/convert.rs`）。
- 残る食い違い（対処しない）: ADR 0038 の `fill_left` は Prefetch だけに効き、先行計算がないときのその場の変換は補わない左で組み立てる。アプリの反映が遅いとき、補わない左は履歴の接尾辞にならず前文を足さない（安全側。今と同じく文脈が少ないだけ）。

## 6. 既定を `fill = 40` にする影響

- **評価の 1 位**: `field` 列のないセット（carry-* 以外の全セット、AJIMEE）は `compose_left` を通らず、`carried_chars = 0` なので、**作りの上で変わらない**。変わるのは carry-* の 6 セットと `c182-carryover.tsv` などの scenario。統合で `eval/scripts/snapshot.sh` を LM あり・なしで流し、carry-* 以外が 1 件も変わらないことと、carry-* の数が報告（場合分け (1, 0.5)）と一致することを確かめる。
  - 期待（報告 C 節、LM あり）: C1 1120、C2 1171、C3 1062（基準 1120 から −58）、C4 1120、C5 623（基準 627 から −4）、chat 59。LM なし: 994、1059、966、994、614、46。
- **LM の窓**: zenz は末尾 40 字だけを見る（`context_chars = 40`）。`fill = 40` で左がそれを超えることはない。ただし、左が短い欄では、今より長い左（最大 40 字）がプロンプトに入る。
- **時間**: 掃引は締め切り 3000ms で時間切れ 0 だった。本番の締め切りでは測っていない。プロンプトが長くなるぶん、LM の時間が伸び、時間切れ（#159）が増えうる。統合で `cargo xtask perf profile` を 1 回と、本番の締め切りで carry-chat・C1 を流し、時間切れの件数を `fill = 0` と並べて PR に書く。
- **話題の採点の手間**: 前文があるときだけ、ベクトルを 2 つ作る（内容語は最大 `recent_tokens` × 2）。測れるほどの差はない見込み（推測。perf で見る）。
- **実機**: A1（ADR 0046 E4）と合わせて、同じアプリの 30 分以内の確定が、欄の左が 40 字に満たないときに前に入る。学習の文脈にも入る（ADR 0046 E6 のまま）。

## 7. 実装の分け方（契約コミット → 並行レーン 3 つ → 統合）

1 つの統合ブランチ（例 `feat/carry-fill-default`。Refs #182）にまとめて 1 つの PR にする。

**契約コミット**（統合ブランチに先に入れる。レーンは変えない。既定は今と同じ動きになる値）
- `core/src/context_carry.rs`: `CarryParams::DEFAULT`（`fill_chars: 0`）、`ComposedLeft.carried_chars` とその計算、§3.3（ADR 0046）のテストに `carried_chars` の期待を足す。純関数で小さいので実装まで入れる。
- `core/src/context.rs`: `left_carried_chars` と `with_left_carried_chars`。
- `core/src/analysis/mod.rs`: `carried_from`（既定 None）と、`extended` でずらす 1 行とそのテスト。
- `core/src/ranker/topic.rs`: `TopicConfig` の 2 項目と `CONFIG_KEYS`（既定は両方 1.0。採点はまだ変えない）。
- 木がビルドできる最小の修正（`ComposedLeft` を作る所、`AnalyzedContext` を作る所の `..Default::default()` など）。

**レーン A（core の採点）**: 持つファイル `core/src/converter/mod.rs`（`analyze` で境目を入れる）、`core/src/ranker/topic.rs`（§4.3 の採点。単体テスト: 境目なしは今と同じ点、元の左 0 字で m = 1 なら今と同じ点、元の左ありで前文の加点が m 倍、文節のモードで境目がずれる）。触らない: engine、eval、`context_carry.rs`。

**レーン B（engine）**: 持つファイル `engine/src/session.rs`、`engine/src/context_limits.rs`（定数を消し `CarryParams::DEFAULT` へ）、`engine/src/convert.rs`・`engine/src/cache.rs`（§4.5 の確定で捨てる）、`engine/src/tests/`（`context.rs` に `left_carried_chars` の期待、`convert.rs` に §5 のテスト）。shell-core は変えない（組み立てない。確かめるだけ）。触らない: core、eval。

**レーン C（eval）**: 持つファイル `eval/src/carry.rs`、`eval/src/run.rs`、`eval/src/scenario.rs`、`eval/src/args.rs`、`eval/tests/`（`field` 列のないケースで `left_carried_chars = 0`、既定が `CarryParams::DEFAULT` に従う）。触らない: core、engine、`eval/sets/`。

**統合（リード）**: 3 レーンをまとめたあと、既定を決定の値にする 1 コミット（`CarryParams::DEFAULT` を `fill_chars: 40`、`TopicConfig` の `carry_mult` を 0.5。`carry_mult_empty` は 1.0 のまま）。フルの `scripts/check.sh`、§6 の snapshot（LM あり・なし）と時間、`c182-carryover.tsv`。ADR 0046 の冒頭の状態の行に「値は ADR 0049 で決定」を足す。

レーンの間に共有のファイルはない。A と B はどちらも契約の `left_carried_chars`・`carried_from` にだけ依る。

## 8. リスク

- **C3 の害が残る**: 同じアプリで話題が変わる場（チャット）で、LM ありで −58（1120 → 1062）。倍率 0.5 で −81 から減らしたが、0 にはならない。左の長さ別では 1〜10 字の左で崩れが大きい（報告の末尾）。実機での頻度は測れない。
- **C5 が基準より少し悪い**: 空の欄で、同じアプリの無関係な前文だけが左にあるとき、LM あり −4、LM なし −9。ADR 0046 §7.2 の決め方の 2（C5 が文脈なしより悪くならない。必須）を満たさない（§10）。
- **時間切れ**: §6。本番の締め切りでは測っていない。増えたら PR でユーザーに数字を出す。
- **境目の扱いの誤り**: 文節のモードで `extended` のずれを落とすと、元の左のトークンが前文に数えられる。レーン A のテストで押さえる。
- **確定で捨てる前提**: 殻が確定より前に次の合成の Prefetch を送ることはない、という前提（§5）。崩れると、新しい Prefetch まで捨てる（害は「先行計算に当たらない」だけで、結果は正しい）。
- **掃引の数字との差**: 実験は static で境目を渡し、文節のモードのずれを見ていない。評価は読み全体の 1 位なので、数字への影響はない見込み（推測）。統合の snapshot で報告と一致することを確かめ、違えば理由を書く。
- 倍率は話題にだけ効く。連接、文書内キャッシュ、LM は前文を元の左と同じに見る（K7）。害がそちらから来ていれば、この ADR の倍率では減らない（報告の C 節で m = 0 でも C3 は 1071 で、基準より −49。残りの害は話題の外から来ている）。

## 9. 検討した案

| 案 | 内容 | 判断 |
|---|---|---|
| 設定だけ（`topic.weight` を全体で下げる） | コードを変えない | 不採用。足さない基準も落ちる（報告 A: LM なし C4 994 → 737） |
| 単一の倍率（左の有無で分けない） | m = 1 または 0.5 | 不採用。m = 1 は C3 −81、m = 0.5 は C1 が 1120 → 1080 に落ちる。場合分けで両方を取る（ユーザー決定） |
| F3: LM にだけ足す（ADR 0046） | 話題・連接には足さない | 不採用。LM なしで得がなく、空の欄（C1）では話題も効いている（LM なし C1 623 → 994） |
| 境目を `RankInput` に持たせる | 採点のたびに渡す | 不採用。文節のモードのずれ（`extended`）を呼び出し側がそれぞれ計算することになる。解析の結果に持たせ、ずらすのを 1 か所にする |
| 境目をトークンごとの印にする | `ContextToken` に「前文」の印 | 不採用。解析器とそのキャッシュに、左文脈の出どころを教える必要がある |

## 10. ユーザーに聞くこと

1. **C5（空の欄で、同じアプリの無関係な前文だけがある）の小さな悪化を受け入れてよいか**。LM あり 627 → 623（−4）、LM なし 623 → 614（−9）。ADR 0046 §7.2 の決め方の 2 では「必須」としていた条件に反する。原因は `fill = 40` と元の左 0 字での m = 1 の組み合わせで、m = 1 を下げると C1 の得（627 → 1120）も減る。推奨: 受け入れる（C1 と chat の得のほうがずっと大きい）。**決定（2026-11-04、ユーザー）: 受け入れる。ADR 0046 §7.2 の決め方の 2 は、この ADR で緩める（C5 は −4〜−9 を許す）。**

## 11. 参考

- ADR 0046（左文脈の持ち越し）、ADR 0047（話題の減衰）、ADR 0038（先行計算の当て方）、ADR 0015 §0.6（無関係な前文の害は話題から）
- `eval/reports/0046-fill-topic-sweep-2026-11-04.md`
