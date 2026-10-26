# フェーズ3a 前のリファクタ計画（2026-10）

- 状態: 計画（リードとユーザーの承認待ち）。§9 の「提案」は ADR かユーザーの決定を変えるため、別に承認を取る
- 日付: 2026-10-12
- 前提: main = 6a58b73。spec の「基本原則」、ADR 0001〜0005（0003 §7.1〜7.3、0004 §15.1、0005 §15.1 の決定を含む）
- 範囲: 重複と不要なコードの整理、構造の見直し、ドキュメントの整理（インストール手順を 1 か所に）、テストとビルドの整理（辞書のビルドからインストールまでを 1 コマンドに）
- 守ること:
  - 振る舞いを変えない。eval の数値（homophones、ext、hiragana）と smoke の結果が前後で一致する（判定の方法は §7.1）。
  - 基本原則と ADR の決定を覆さない。覆すものは §9 に「提案」として分ける。
  - フェーズ3a（ADR 0005）の契約とファイル境界とぶつからない順序にする（§8.1）。

---

## 0. 現状の把握（2026-10-12 に実際に確かめたこと）

### 0.1 ビルドとテスト
| 項目 | 結果 |
| --- | --- |
| `cargo build --workspace` | 2.1 秒（増分）。**`--workspace` は `lm` と `engine-ffi` もビルドするが、feature なしなので llama.cpp はビルドされない** |
| `cargo clippy --workspace --all-targets` / `cargo fmt --check` | 警告 0 / 差分なし |
| `cargo test --workspace -- --list` | 814 件（うち `#[ignore]` 19 件。feature `llama` のときだけコンパイルされる `lm/tests/real_model*.rs` と `eval/tests/scenario_lm.rs` は 0 件と出る） |
| テスト全体の時間（debug、バイナリごと） | 合計 約 6 秒。最も遅いのは `eval/tests/flags.rs` の 2.9 秒（`ikari-eval` を 24 回起動）、次が `ikari_lm` の単体 0.5 秒。**遅いテストの問題はない** |
| `swift test --package-path macos` | 126 件（20 + 105 + 1）、約 5 秒。最後の 1 件は `IkariFFITests` のプレースホルダ |
| CI | なし（`.github/` なし。`git remote -v` も空） |
| worktree | `.claude/worktrees/` に 12 個（19 GB）。11 個は main にマージ済み。**`worktree-agent-ab805d357ee243769`（a54d87d、ADR 0005 の S0 スパイク）だけ未マージで、作業中** |
| データ | `data/raw/` 約 3 GB（Sudachi の zip と展開物、chiVe、zenz、kanakanji-edge、AJIMEE）。`data/build/` に `dict`、`dict.prev`、`dict-r1-baked`（R1 の暫定。168 MB） |

### 0.2 crate の依存（`cargo tree` より。矢印は「依存する」）
```
dict-format ← dict-builder
dict-format, input ← core ← lm(optional: llama-cpp-2)
core, lm(optional) ← stack ← engine ← engine-ffi (staticlib)
core, dict-format ← import
core, stack, import, clap (+ lm optional) ← cli, eval
input ← shell-ffi (staticlib)
prost ← proto ← engine
```
- 依存は一方向で、循環はない。
- `cli` と `eval` は `ikari-lm` に直接依存しているが、ソースで使うのは `eval/src/generate.rs:93`（`ikari-eval gen`）だけ。`cli` は feature の受け渡しのためだけに依存している（`cli/Cargo.toml` の `lm = ["dep:ikari-lm", "ikari-lm/llama", "ikari-stack/lm"]`）。
- `eval` は `ikari-import` 経由で既に `serde_json` を引いている（`eval/src/json.rs` の自前の JSON パーサは、依存を増やさないためという理由がもう成り立たない）。

### 0.3 フェーズ3a の契約とファイルの持ち主（ADR 0005 §11.2、§11.3）
| 持ち主 | ファイル |
| --- | --- |
| 契約コミット（リード） | `proto/**`、`macos/Package.swift`、`IkariShellCore` の公開の型、`IkariXPC/Identifiers.swift`、`docs/phase2-contract.md`、`macos/scripts/gen-proto.sh`、`macos/scripts/templates/IkariSettings.Info.plist` |
| E3（Rust） | `engine/src/{admin.rs,lib.rs,tests.rs,log.rs}`、`stack/src/lib.rs` の `Reuse`、`core/src/user/**` |
| S3（Swift、IME） | `macos/Sources/{IkariShellCore,IkariIME,IkariXPC}/**`、`macos/Tests/{IkariShellCoreTests,IkariXPCTests}/**`、`macos/tools/make-icons.swift`、`macos/Resources/{icons,*.lproj}/**`、`templates/ikari.Info.plist` |
| U（Swift、設定アプリと配布） | `macos/Sources/{IkariSettings,IkariSmoke}/**`、`macos/scripts/{build,install,lib}.sh`、`macos/README.md` |

S0 スパイク（捨てるコード）が `Package.swift`、`build.sh`、`IkariInputController.swift`、`make-icons.swift`、`templates/` を自分のブランチで変えている。S0 はマージしないので、このリファクタとは衝突しない。

---

## 1. 問題の定義

並行開発で生まれた負債は、次の4種類に分けられる。

1. **同じ知識が複数の場所にある**: 構成の名前（RankerSet）が 4 か所、`key=value` の読み取りが 2 か所、LM の設定キーが 3 か所、かなの変換が 5 か所、FNV-1a が 4 か所。どれも、片方だけを直すと黙って食い違う種類の重複。
2. **決着した実験のつまみが残っている**: 不採用の `eos_weight`、R1 の暫定の `bake_policy`、ADR 0003 §3.3 で削除が決まった `rare_lemma`、取り下げた `ikari-eval gen`、それに**本番の経路で環境変数を読む実験の残骸**（`IKARI_MARGIN`、`IKARI_FREE`）。
3. **ドキュメントが実装に追いついていない**: README は「フェーズ0実装中」のまま。インストールの手順が README、macos/README、`scripts/install-data.sh` の先頭、ADR 0004 §8.3、レポートに散らばり、辞書の取得からアプリの入れ替えまでを通しで書いた場所がない。ADR の索引の状態が本文と違う。
4. **テストとビルドの入口がない**: 辞書のビルド（vectors.bin を含む）→ データの配置 → アプリのビルド → インストール → smoke を、人が手で順に流している。`#[ignore]` のテストは、環境変数がないと**黙って成功する**ので、`--ignored` で流しても検証したことにならない。

いちばん危ないのは 2 の環境変数と 4 の「黙って成功する」テストで、どちらも気づかないうちに結果を変えたり、失敗を隠したりする。いちばん効率が悪いのは 1 で、つまみを 1 つ足すたびに 3〜4 か所を直すことになる。

---

## 2. 洗い出し: 重複と不要なコード

### 2.1 cli、eval、stack の組み立ての重複
| # | 重複 | 根拠 | 直し方 |
| --- | --- | --- | --- |
| D1 | 構成の名前の enum が 4 つ | `stack/src/lib.rs:47`（`RankerSet`、正）、`eval/src/setup.rs:28`（同じ 5 値の写しと `to_stack`）、`eval/src/main.rs:121`（`RankerArg`、さらに写し）、`cli/src/args.rs:39`（cost / full だけの clap 用） | eval は `ikari_stack::RankerSet` を直接使う（clap には `value_parser` で `RankerSet::parse` を渡す）。cli の clap 用の enum は、受け付ける値を cost / full に絞るという意味があるので残し、`From` で stack の値にする。eval の写しのドキュメントの食い違い（`eval/src/setup.rs:33` の「R2: R1 + DocumentCache」と `stack/src/lib.rs:52` の「左文脈、かなの表記、DocumentCache」）もこれで消える |
| D2 | `key=value` の設定ファイルの読み取りが 2 つ | `cli/src/stack.rs:72` と `eval/src/setup.rs:176`。eval は空のキーを拒むが cli は通す。エラー文も日本語と英語で違う | `ikari_stack::read_ranker_config(path)` に 1 つにする。**エラーの経路だけ挙動が変わる**（cli でも空のキーを拒む）。正常な入力の結果は同じ |
| D3 | LM の設定キーの文字列が 3 か所 | `cli/src/stack.rs:32,34`、`eval/src/setup.rs:22,23`、`engine/src/lib.rs:137,140`（`"zenz.budget_ms"`、`"zenz.idle_unload_secs"`、`"zenz.model_kind"`） | `StackOptions` に型つきの `lm: LmOverrides { budget_ms, idle_unload_secs }` を足し、`stack` の中で `ranker_config` に合成する（明示した値が設定ファイルより優先、という今の規則のまま）。キーの文字列は `stack/src/lib.rs:31` の `LM_KEY_PREFIX` の近くの 1 か所だけにする |
| D4 | CLI が `build_stack` を使わず、組み立てを手で写している | `cli/src/stack.rs:108-189`（`load_dictionaries` → `StackInputs` を手で埋める → `assemble`）は `stack/src/lib.rs:398-415` の `build_stack_with` と同じ手順 | CLI は辞書ディレクトリを決めたあと `ikari_stack::build_stack` を呼ぶ。CLI 独自の `Stack` 構造体（`cli/src/stack.rs:89`）は `ikari_stack::Stack` で置き換える。`LoadedSet` が要る `user_cmds` と `ingest_cmd` は今のまま `load_dictionaries` を使う |
| D5 | エラーの文字列を部分一致で読み替えている | `eval/src/setup.rs:313-330`（`m.contains("topic needs vectors.bin")`、`` m.contains("`lm` feature") ``、`m.starts_with("LM model not found: ")`） | `StackError` に `MissingVectors`、`LmNotBuilt`、`LmModelNotFound(PathBuf)` を足し、eval は種類で分ける。stack の文言を直すと、eval のエラー文が黙って変わる、という結合がなくなる |
| D6 | エンジンのロックのファイル名が 2 か所 | `engine/src/lock.rs:9` と `cli/src/engine_lock.rs:13`（「`ikari-engine` の `LOCK_FILE_NAME` と同じ」とコメントで約束しているだけ） | `core/src/user/dir.rs` に `ENGINE_LOCK_FILE_NAME` と `UserDir::engine_lock()` を足し、両方がそれを使う（ユーザーデータのファイル名は既に `UserDir` に集めてある） |
| D7 | ユーザーデータのファイル名の写し | `engine/src/lib.rs:76-77`（`USER_DICT_FILE`、`PROFILE_FILE`）は `core/src/user/dir.rs:7-8` と同じ値 | `UserDir::user_dict_tsv()` / `profile_tsv()` を使う |
| D8 | 辞書の置き場所の名前の写し | `cli/src/stack.rs:30`（`USER_DICT_SUBDIR = "dict"`）と `engine/src/lib.rs:48`（`DICT_DIR_NAME`）、既定の辞書ディレクトリ `cli/src/args.rs:13`（`./data/build/dict`）と `eval/src/main.rs:32`（`data/build/dict`） | 名前は `UserDir::dict_dir()` に寄せる。**辞書ディレクトリの探し方そのもの（cli はユーザーデータの `dict/` も探し、eval は探さない）は変えない**（eval が利用者の辞書を黙って使い出さないため） |
| D9 | 日付の計算の写し | `eval/src/main.rs:227`（`unix_day`）と `core/src/user/util.rs:38`（`today_unix_day`） | core の関数を使う |
| D10 | 一時ディレクトリの自前の実装 | `eval/src/setup.rs:151`（`TempDir` と `temp_user_dir`） | 振る舞いは同じなので残してよい。`tempfile` を通常の依存にする利点は小さい（nice to have） |

### 2.2 crate をまたぐ小さな重複
| # | 重複 | 根拠 | 直し方と注意 |
| --- | --- | --- | --- |
| D11 | ひらがな → カタカナ | `core/src/converter.rs:569`（`to_katakana`）、`input/src/script.rs:182`（`hiragana_to_katakana_char`）、`lm/src/prompt.rs:85`（`to_katakana`、**踊り字 ゝゞ を変えない**） | ADR 0004 §3.3 は「Converter の `to_katakana` を `ikari-input` に集める」と決めたが、写しが残っている。core は `ikari_input` の関数を使う（正規化済みの読みの上では同じ結果。**置き換える前に、ぁ〜ゖ、ゝ、ゞ、ー、ASCII の全域で一致を確かめるテストを先に書く**）。lm の版は踊り字の扱いが違うので、置き換えると LM のプロンプトが変わりうる。**lm は置き換えず、`pub` をやめて違いをコメントに書く** |
| D12 | カタカナ → ひらがな | `dict-format/src/reading.rs:21`（`normalize_key`、索引のキーの契約）、`input/src/script.rs:193`、`eval/src/ajimee.rs:27`（踊り字を変えない） | dict-format の版は辞書の形式の契約なので動かさない。eval は `ikari_core::user::normalize_reading` を使う（評価セットと AJIMEE に ゝゞヽヾ は 0 件と確かめた。数値は変わらない） |
| D13 | FNV-1a 64 が 4 つ | `dict-format/src/util.rs:7-12`（`Fnv1a64`、公開）、`core/src/user/util.rs:10-27`、`core/src/dict/memory.rs:11`、`eval/src/lib.rs:30-31` | すべて `ikari_dict_format::Fnv1a64` にする。**学習の特徴量のハッシュと eval の split の割り当てが変わらないことを、既知の入力と出力の表で固定してから置き換える** |
| D14 | 末尾の n 文字 | `lm/src/prompt.rs:74`、`engine/src/lib.rs:374`、`import/src/holdout.rs:71`（String を返す） | 小さいので nice to have。やるなら `ikari_input` か core に1つ置く |
| D15 | Ranker の名前の文字列 | `"left_context"` などが、各 Ranker の `name()`（例 `core/src/ranker/left_context.rs:112`）、Composite の分類（`core/src/ranker/composite.rs:19-23`）、設定の接頭辞（`ScopedKv` の `CONFIG_PREFIX`）、stack の説明文（`stack/src/lib.rs:571-612` の `names.push("...")`）に別々に書かれている | `core/src/ranker/names.rs` に定数を置き、`name()` と Composite はそれを参照する。stack は `ranker.name()` から説明文を作る（出力の文字列は同じ） |
| D16 | 同名の別の型 | `eval/src/holdout.rs:27`（`AlignedToken`）と `import/src/align.rs:43`（`AlignedToken`、フィールドが違う） | eval の方を `HoldoutToken` に改名する（nice to have） |

### 2.3 使われていない公開 API
crate の外から使われていないことを grep で確かめたもの（テストだけで使うものを含む）。

| crate | 項目 | 根拠 | 扱い |
| --- | --- | --- | --- |
| core | `GrammarGuard`、`GrammarGuardConfig`、`LearningGuard`、`LearningGuardConfig` の再公開 | `core/src/ranker/mod.rs:23,27`。core の外の利用 0 件。`*Config::from_kv` はテストからしか呼ばれず、`ranker_config` に `grammar_guard.enabled=0` と書いても**黙って無視される**（どの `ScopedKv` も他の接頭辞を読み飛ばす） | `pub(crate)` にする。比較実験に要るときは、そのときに `StackOptions` から配線する（今は配線しない。YAGNI） |
| core | `CompositeRanker::with_guard` | `core/src/ranker/composite.rs:45`。テストだけ | `#[cfg(test)]` にするか消す |
| core | `ConvertOptions.eos_weight` | `core/src/converter.rs:29,89,101,233-237,273`、`core/src/nbest.rs:54-66,215`。外からの利用 0 件 | §2.4 で消す |
| lm | `to_katakana`、`topic_words`、`build_prompt`、`LmLoader`、`LmError`、`ModelInfo` | `lm/src/lib.rs:19-28`。crate の外では `lm/tests/` 以外の利用 0 件（`build_prompt` は `lm/tests/real_model.rs` だけ） | テストに要るものを除いて `pub` をやめる。`LlamaScorer::info()`（`lm/src/llama.rs:190`）と `generate()`（同 :207）は `gen` を消したあとも `lm/tests/real_model_edge.rs:53,275`（edge の実装の確認）が使うので残す。`ModelInfo` は `info()` の戻り値なので公開のまま |
| stack | `assemble_with`、`LoadedSet::grammar_dyn` | `stack/src/lib.rs:457,264`。外からの利用 0 件（`assemble_with` は `assemble` と `build_stack_with` が内部で使う） | `assemble_with` は非公開にする |
| dict-builder | `BuildOptions.bake_policy` | `dict-builder/src/lib.rs:212,221,370-380,466-467` | §2.4 で消す |

`DictError`、`RewriteInput`、`KanaStyleParseError` などは外で名前を書かれていないが、公開のシグネチャに出てくる型なので残す。

### 2.4 実験のつまみと残骸
| # | 対象 | 根拠 | 判断 |
| --- | --- | --- | --- |
| E1 | **本番の経路の環境変数 `IKARI_MARGIN`** | `core/src/analysis/lattice_analyzer.rs:247-250`。左文脈の解析で、開いた終わり方の候補ごとに `std::env::var` を呼ぶ（0495001 で入った） | **最優先で消す**（`OPEN_END_MARGIN` の定数を直接使う）。打鍵ごとの解析で環境変数を読む遅さと、エンジンの環境しだいで結果が変わる危うさがある。未設定のときの挙動は同じ |
| E2 | **辞書のビルドの環境変数 `IKARI_FREE`** | `dict-builder/src/policy.rs:124-127`（0a7570c） | 消す（`LEMMA_RANK_FREE_DECADES` を使う）。辞書の再現性（同じ入力から同じ `dict_id`）を環境変数が壊しうる |
| E3 | `eos_weight` | §2.3。phase2-q §8 で不採用（dev の話題 +0.8 ポイントで、採用条件の +1 に届かない） | 消す。nbest の倍率の分岐とテスト（`nbest.rs:215`）も消す。phase2-q に「`eos_weight` は <コミット> で削除。再現はそのコミットで」と1行足す。ADR 0003 §3.4 の決定（実験して、条件を満たせば採用）を実行し終えた結果なので、ADR は覆さない |
| E4 | `--bake-policy` | `dict-builder/src/main.rs:76-78,168`、`lib.rs:212,370-380,466`、`tests/v2.rs:305`。R1 の部分評価（`eval/reports/phase1-r1-policy.md`）のためだけの暫定 | 消す。`data/build/dict-r1-baked` も不要になる（削除はユーザーの確認後。§10） |
| E5 | `rare_lemma` | `dict-builder/src/policy.rs:22,36,48,215-217,244,262,389-394,480-481,629`、`tests/v2.rs:138`、`policy/default.tsv:28`、`src/main.rs` の `--chive-vocab` の説明 | **ADR 0003 §3.3 が「フェーズ2の整理で削除する」と決めたのに残っている**。消す。今のポリシー（`chive.tsv`）は使っていないので、辞書のバイト列は変わらない（§7.1 で確かめる） |
| E6 | `ikari-eval gen` | `eval/src/generate.rs`（198 行）、`eval/src/main.rs:61-91,524-570`、（`lm/src/llama.rs:207` の `generate` は edge の実モデルのテストが使うので残す） | 消す。ADR 0003 §7.3 が「LM の生成で作った表記を候補に足す案は採らない」と決めた。結果は phase1-lm-compare §4 に残る。これで eval の `ikari-lm` への直接の依存がなくなる |
| E7 | kanakanji-edge（`model_kind`） | `lm/src/config.rs:23-46`（`ModelKind`）、`lm/src/prompt.rs:56-70`、`lm/src/ranker.rs:198`、`lm/tests/real_model_edge.rs`（279 行）、`--lm-kind`（`cli/src/args.rs:103-106`、`eval/src/main.rs:201-204`、`eval/src/setup.rs:109,245`、`cli/src/stack.rs:124`） | **残す（ADR 0003 §7.1-1「edge の実装（model_kind）は残すが、既定にはしない」のとおり）。ただし入口を `zenz.model_kind` の設定キー 1 つに絞り、CLI と eval の `--lm-kind` は消す**。理由: 本体の実装は約 40 行で、`ModelKind` の既定が `ZenzV3` なので常用の経路に影響しない。新しい edge のモデルが出たときに比較を流し直す価値がある。一方、専用のフラグは 2 つのバイナリに同じ配線（D3）を増やしているだけで、設定ファイルで足りる。実モデルのテスト（`real_model_edge.rs`）は、残した実装が腐らないための唯一の確認なので残す。完全に消す案は §9 P1 |
| E8 | 契約のスタブの時代の smoke のフラグ | `macos/Sources/IkariSmoke/main.swift:4,61,138-140`（`--allow-not-ready`）、`macos/README.md:16,42,45` | 消す。エンジンが動いている今、このフラグは失敗を SKIP に変えるだけ。README のテスト手順がこのフラグを付けているので、手順どおりに流すと失敗を見逃す |
| E9 | 中身のないテストのターゲット | `macos/Tests/IkariFFITests/FFITests.swift`（プレースホルダ 1 件）、`macos/Package.swift:52-58` | 消す。FFI の往復は smoke（XPC 経由の E2E）と `engine-ffi` / `shell-ffi` の Rust のテストで確かめている。Package.swift の条件分岐が 1 つ減る |
| E10 | `Package.swift` の XCFramework の有無による分岐 | `macos/Package.swift:8-13,35-45`、`macos/Sources/IkariIME/Composers.swift:6`、`IkariEngineService/main.swift:13,35,40,68,82`。分岐のせいで `--manifest-cache none` が要る（`macos/README.md:49`） | 分岐そのものは**残す**（`swift test` を Rust のビルドなしで流せる利点が大きい）。危ないのは、XCFramework がないまま「スタブのエンジン」の `ikari.app` ができてインストールされうること（`macos/scripts/build.sh:241` は warn だけ）。**`build.sh` では XCFramework がなければ止める**（`--allow-stub` を明示したときだけ通す）。§4.5 |
| E11 | 追跡されているキャッシュ | `testdata/sudachi-mini/__pycache__/extract.cpython-314.pyc`（git で追跡されている）、`macos/tools/.gitkeep`（ファイルがあるのに残っている） | `git rm` し、`.gitignore` に `__pycache__/` を足す |
| E12 | 工程の名前のコメント | 「レーンA」「lane X」などが 24 か所（例 `core/tests/dict_files.rs:1`、`input/src/romaji.rs:1`、`macos/Package.swift:25-47`、`macos/scripts/build.sh:181`）。フェーズごとにレーンの名前が違うので、今は意味が通らない | 担当ではなく内容を書く（「辞書形式 v2 の読み込みのテスト」など） |

### 2.5 古い Package.swift の分岐の扱い（まとめ）
- 残す: `hasFramework` による `IkariShellFFI` / `IkariEngineFFI` の有無の分岐（E10）。
- 消す: `IkariFFITests`（E9）と、その条件つきのリンク設定。
- 足す: `build.sh` の「XCFramework がなければ止める」。
- 変えない: ターゲットの名前と構成（ADR 0005 の契約コミットが `IkariBridge` などを足すので、ここでは増やさない）。

---

## 3. 洗い出し: 構造

### 3.1 大きすぎるファイル（行数の上位。テストの割合つき）
| ファイル | 行数（本体 / テスト） | 案 | いつ |
| --- | --- | --- | --- |
| `core/src/converter.rs` | 1654（596 / 1058） | `converter/mod.rs`（`Converter`、`ConverterBuilder`）、`converter/options.rs`（`ConvertOptions`、`LmBudget`）、`converter/conversion.rs`（`Conversion`、`Page`、`CommitEvent`）、`converter/tests.rs`。公開のパスは `pub use` で変えない | 今回（L1） |
| `eval/src/lib.rs` | 1348（757 / 591） | `set.rs`（`EvalCase`、`load_cases`、`Split`、`filter_cases`）、`run.rs`（`RunOptions`、`run_with`、時間の計測）、`report.rs`（`summarize`、`render_markdown`、`render_cases_tsv`）。テストはそれぞれの横に | 今回（L2） |
| `lm/src/ranker.rs` | 1307（510 / 797） | テストを `ranker/tests.rs` に出すだけ（本体は 510 行で、ワーカーと採点が1つの状態を共有しているので分けない） | 今回（L2） |
| `engine/src/tests.rs` | 1270 | フェーズ3a で E3 が管理のテストを足すファイル。今分けると E3 と衝突する | **3a の後** |
| `core/src/user/dictionary.rs` | 1195（740 / 455） | Mozc 形式の読み書きと、登録の検証と展開に分けられる。ただし E3 が `core/src/user/**` に手を入れる | **3a の後** |
| `dict-format/src/dict.rs` | 1092（574 / 518） | テストを出すだけで十分 | nice to have |
| `stack/src/lib.rs` | 1027（771 / 256） | `lib.rs`（公開の型、`Stack`、**`Reuse`**、`build_stack`、`build_stack_with`）、`load.rs`（辞書、vectors、プロファイル）、`assemble.rs`（`assemble`。§3.3 の整理の後）、`lm.rs`（feature の有無の2つの実装）。**`Reuse` と `build_stack_with` は ADR 0005 §11.3 が `stack/src/lib.rs` として指しているので lib.rs に残す** | 今回（L2） |
| `engine/src/lib.rs` | 992（ほぼ全部が本体） | セッション、変換、先行計算、データの監視、補助関数に分けられる。ただし ADR 0005 §8.3 で E3 が `dispatch`、上限、`refresh_data` の切り出しを行う | **3a の後**。今回は D7 などの定数の置き換えだけ |
| `core/src/ranker/composite.rs` | 859（310 / 549） | §3.4 | 今回（L1） |
| `core/src/analysis/lattice_analyzer.rs` | 859（419 / 440） | E1 だけ。分けない | 今回（L1） |
| `cli/src/args.rs` | 831（508 / 323） | テストを `args/tests.rs` に出す | nice to have |
| `eval/src/main.rs` | 608（ほぼ全部が本体） | E6 と D1 で約 150 行減る。残りを `args.rs`（clap の定義）と `main.rs`（実行）に分ける | 今回（L2） |
| `macos/Sources/IkariSmoke/main.swift` | 395 | `main.swift`（引数と実行の順）、`Report.swift`、`Checks/{Hello,Conversion,Recovery,Version,Latency}.swift`。**フェーズ3a で U が管理の E2E を足すので、その前に分けておくと U の差分が小さくなる** | 今回（L3。U より前にマージ） |
| `macos/Sources/IkariShellCore/InputStateMachine.swift` | 603 | `// MARK:` で状態ごとに整理されていて、読みにくくはない。S3 が `reconfigure` を足す | 分けない（3a の後に必要なら extension で分ける） |

`macos/Sources/IkariProto/engine.pb.swift`（1552 行）は生成物なので対象外。

### 3.2 モジュールの境界
- **今のままでよいもの**: 依存の向き（§0.2）は一方向で、ADR 0004 §3.1 のとおり。`ikari-stack` を共有の組み立てにした判断も正しい。重複は「stack を使い切っていない」ことによる（§2.1）。
- **cli と eval の `ikari-lm` への直接の依存をやめる**: E6 の後、どちらもソースでは使わない。feature を `lm = ["ikari-stack/lm"]`、`lm-metal = ["lm", "ikari-stack/lm-metal"]` にする（`cli/Cargo.toml`、`eval/Cargo.toml`）。LM を知っている crate が `stack` と `lm` だけになる。
- **`AssemblyMode`（Runtime と Eval の違い）は残す**: Eval は「常に Composite で包む」「Full なら学習とプロファイルを常に入れる」など、比べるための固定の組み立てで、ここを揃えると eval の数値が変わる。違いが `stack/src/lib.rs:104-120` の1か所に書かれているので、重複ではなく意図した分岐として扱う。
- **core の `user/`（ファイルの読み書き）を別 crate にする案は採らない**（§5）。

### 3.3 公開 API の整理（方針）
1. 外から使われていない型と関数は `pub(crate)` にする（§2.3）。
2. 「ある crate の写し」は、正の crate の型を使う（D1〜D8）。
3. 実験のための設定は、`ranker_config`（`key=value`）の 1 つの入口に集め、CLI のフラグを増やさない（E7）。
4. core の `pub use ikari_input as input` とその再公開（`core/src/lib.rs:32-38`）は、ADR 0004 §3.1 / §3.3 が「既存の API は変えない」として残したもの。使っているのは cli だけ（`cli/src/reading.rs:3`、`convert.rs:8`、`session.rs:16`）。消すなら ADR の変更になるので §9 P2。

### 3.4 core の `ranker/` の構成（Guard と Composite の置き場所）
今の問題:
- `composite.rs` が 4 つの役割を持つ: Ranker の合成、2 つの Guard を差し込む位置の計算、「文法候補」の分類（`is_continuing_form`、`is_particle_plus_auxiliary`、`composite.rs:166-207`）、全 Ranker が共有する部品（`ScopedKv`、`apply_bonus`、`content_segments`、`segment_lemma`、`composite.rs:209-309`）と、テストの共有の環境（`testutil`、`composite.rs:311-477`）。
- Guard は Ranker ではない（phase2-q §2.1: 各 Ranker の前後の差分が要るので Composite の中で働く）のに、Ranker と同じ階層に並んでいる。
- Composite は Ranker を名前の文字列で見分ける（D15）。

案:
```
core/src/ranker/
  mod.rs            Ranker、RankInput、RankerConfigError、再公開（今と同じ公開のパス）
  names.rs          Ranker の名前の定数（D15）
  config.rs         ScopedKv（composite.rs から移す）
  common.rs         apply_bonus、content_segments、segment_lemma（composite.rs から移す）
  composite.rs      CompositeRanker（並びと、Guard を差し込む位置の計算だけ）
  guard/
    mod.rs          pub(crate)。Guard が受け取る差分の型
    grammar.rs      GrammarGuard と「文法候補」の分類（is_continuing_form、is_particle_plus_auxiliary）
    learning.rs     LearningGuard
  testutil.rs       #[cfg(test)]。composite.rs の testutil を移す
  cost.rs left_context.rs doc_cache.rs kana_style.rs learning.rs profile.rs topic.rs   （変えない）
```
- 「文法候補」の分類は GrammarGuard の規則の一部（phase2-q §2.1 の追加 2 と 3）なので、Guard の横に置く。
- 移すだけで、計算の順序と式は変えない。確かめ方は eval の結果の一致（§7.1）と、既存の Composite と Guard の単体テストがそのまま通ること。

### 3.5 Swift 側のターゲットの分け方
- **今のままでよい**: `IkariShellCore`（AppKit と IMK に依存しない純 Swift）、`IkariProto`（生成物）、`IkariXPC`、`IkariIME`、`IkariEngineService`、`IkariSmoke` の分け方は、ADR 0004 §6.2 のとおりで、依存も一方向。フェーズ3a で `IkariBridge`、`IkariSettings` が足されるが、それは契約コミットの仕事で、ここでは先取りしない。
- 直すもの:
  - `IkariFFITests` を消す（E9）。
  - `macos/Sources/IkariShellCore/LmBudget.swift` の中身は `EngineTimeouts`（テストも `EngineTimeoutsTests.swift`）。ファイル名を `EngineTimeouts.swift` にする（S3 の持ち物になる前に）。
  - `IkariSmoke` を複数のファイルに分ける（§3.1）。
  - 識別子は `macos/scripts/lib.sh:9-15`、`IkariXPC/Identifiers.swift`、`docs/phase2-contract.md` §1 の 3 か所にある。シェルと Swift の間で 1 つにする仕組み（生成など）は、変更の頻度に比べて重い。**今は作らない**。3a の契約コミットで識別子が増えるので、そのときに `lib.sh` と `Identifiers.swift` の一致を確かめる検査を `build.sh` の検証に足すかをリードが決める。

---

## 4. 洗い出し: ドキュメント

### 4.1 README と各 README の食い違い、古い記述
| 箇所 | 問題 |
| --- | --- |
| `README.md:15` | 「フェーズ0実装中」。実際はフェーズ2まで完了し、3a に入る |
| `README.md:17-33` | リポジトリ構成に crate が 5 つしかない（実際は 13 と `macos/`、`scripts/`、`proto/`） |
| `README.md:37-55` | SudachiDict の取得が `curl -O .../...` のまま（URL がない）。手元には zip（`data/raw/sudachi-20260723/*.zip`）がある |
| `README.md:57-79` | 辞書のビルドが 2 通り書かれていて、**既定（`policy/default.tsv`、vectors なし）の方は製品の辞書と違う**。実際に使う辞書は `policy/chive.tsv` + vectors.bin |
| `README.md:81-83` | 生成物に `vectors.bin` がない |
| `README.md:113-116` | kanakanji-edge の手順（`--lm-kind`。E7 で消すフラグ） |
| `README.md:168-196` | crate の依存の図が 5 crate のまま |
| `README.md:201` | ADR の説明が「フェーズ0の設計詳細」 |
| `README.md` 全体 | macOS 版のビルドとインストールへの言及も、`macos/README.md` へのリンクもない |
| `macos/README.md:17` | `scripts/install-data.sh` がビルドの手順の中に混ざっている（データの手順の全体は README 側にもない） |
| `macos/README.md:16,42,45` | `--allow-not-ready`（E8） |
| `macos/README.md:49` | manifest のキャッシュの注意（E10 の分岐を残すので、注意は残す） |
| `lm/src/lib.rs:4` | 「`budget`（既定 30ms）」。今の既定は 300ms（`lm/src/config.rs:10`、phase2-q §3.1） |
| `eval/src/setup.rs:33` | R2 の説明が stack と違う（D1 で消える） |
| `eval/src/json.rs:3` | 「eval は依存を増やさない方針」。eval は既に `serde_json` を引いている（§0.2） |
| `cli/tests/phase1.rs:3-4` | 「`#[ignore = "requires phase1 lane X"]` を付けている」。今は 1 つも付いていない |
| 環境変数の名前 | 同じユーザーデータのディレクトリが、Rust では `IKARI_USER_DIR`（`core/src/user/dir.rs:5`）、`scripts/install-data.sh:16` では `IKARI_DATA_DIR`。モデルは `IKARI_ZENZ_MODEL`（`lm/tests/real_model.rs`、`eval/tests/scenario_lm.rs`）と `IKARI_MODEL`（`engine/src/tests.rs:1182`） |
| `THIRD_PARTY_NOTICES.md` | chiVe（vectors.bin の元。ユーザーのデータの場所に入る）と、prost、serde / serde_json、jiff、llama-cpp-2 の記載がない（NFR-43） |
| `dict-builder/reports/phase1-policy.md` | 除外 21224 件のまま（ポリシーを変えた phase2-q §7 より前の結果）。製品の状態と違う |
| `docs/ikari-ime_spec.md:289` | 「画面からの取り込みは…フェーズ3」。ADR 0005 §15.1-7 で「spec のロードマップと『文章の取り込み』のルールを直す」と決まったが、まだ直していない |
| `docs/ikari-ime_spec.md:544-554、577-592` | 「リポジトリ構成（案）」と「未決事項」に、決着したもの（辞書ソース、zenz の採用、話題スコアの方式、ユーザー辞書の形式、最後のページの挙動）が残る。spec はユーザーの文書なので、直すかは §9 P5 |
| `docs/phase2-contract.md:39` | 「設定の画面はフェーズ3」。**3a の契約コミット（リード）が §1、§3、§5 を直すので、このリファクタでは触らない** |

### 4.2 インストール手順を 1 か所にまとめる案
- **置き場所: `README.md` の「インストール」節を唯一の手順にする。**
  - 1 コマンド（`scripts/setup.sh`、§6.3）で済むようになるので、README に置いても短く保てる。
  - 失敗したときのために、同じ節に「段ごとの手作業」（取得、辞書、データの配置、アプリ、入力ソースの追加、smoke）を、各スクリプトを 1 行ずつ呼ぶ形で書く。コマンドの細かい引数は各スクリプトの `--help` を正にし、README には写さない（写しが古くなるのを防ぐ）。
- `macos/README.md` は「macOS 版の内部」（バンドルの構成、署名の検証の中身、知っておくこと＝落とし穴）だけにし、ビルドとインストールの手順は README へのリンクにする。「入力ソースの追加（初回だけ、手動）」は README に移す。
- `scripts/install-data.sh` の先頭の説明、ADR 0004 §8.3 は、それぞれの場所の説明として残す（手順の正は README）。
- 開発者向けの手順（テスト、eval、LM の実モデルのテスト、smoke の詳細）は README の「開発」節にまとめる。
- **却下**: `docs/install.md` を新設する案。README の「セットアップ」が既にあり、入口を 2 つにすると今と同じ食い違いが起きる。

### 4.3 現在の構成の地図
- `docs/architecture.md`（新設、150 行以内）: crate の一覧と責務、依存の図（§0.2）、プロセスの構成（殻、XPC、エンジン）、データの置き場所、どの ADR が何を決めたかの索引。
- 理由: ADR は日付つきの判断の記録で、0001〜0005 の「追加分」を積み重ねないと今の構成が分からない（合計 約 2,900 行）。フェーズごとに新しいエージェントが読み直すコストが毎回かかる。README の crate の図（`README.md:168-196`）はこの文書に移し、README からはリンクする。
- 守ること: 判断の理由は書かない（ADR へのリンクにする）。構成を変えたコミットで一緒に直す。

### 4.4 ADR の索引と状態の欄
| ADR | 本文の状態 | 索引（`docs/adr/README.md:9-13`） | 直し方 |
| --- | --- | --- | --- |
| 0001 | 承認（2026-10-11） | Accepted | 表記を揃える |
| 0002 | 承認（2026-10-11。§21） | Accepted | 同上 |
| 0003 | **「提案（承認待ち）」のまま**（`0003:3`）。しかし §7.1〜§7.3 にユーザーの決定がある | Proposed | 本文を「承認（2026-10-11。§7.1〜§7.3）」にし、索引も直す |
| 0004 | 承認（2026-10-11。§15.1） | Accepted | 表記を揃える |
| 0005 | 承認（2026-10-12。§15.1） | **Proposed** | 索引を直す |

- 状態の語を、本文に合わせて日本語にする: `提案` / `承認` / `一部置き換え（ADR XXXX §n）` / `置き換え（ADR XXXX）`。索引のテンプレート（`docs/adr/README.md:24`、英語の Accepted）も直す。
- 索引に「決定の節」の列を足す（例: 0003 → §7.1〜7.3）。決定がどこにあるかを探す手間をなくす。
- 「一部置き換え」の例: 0003 §2.2（xsmall / small の使い分け）は §7.1-1 で取り下げ。0002 §13.1 の Topic の既定値は 0003 §3.2 で置き換え。本文は書き換えず、索引の備考に書く。
- ADR の本文は、状態の行と、ファイルの移動に伴うリンクのパス以外は変えない。

### 4.5 eval/reports の整理
方針: ADR が根拠として参照しているレポートは、リンクを壊さないように**場所を変えない**。参照がなく、もう再現もできない（つまみを消した）ものだけを `eval/reports/archive/` に移す。代わりに `eval/reports/README.md`（索引）を作り、各レポートの位置づけを書く。

| ファイル | 参照元 | 扱い | 理由 |
| --- | --- | --- | --- |
| `phase2-q.md` | なし | **残す（現行の基準）** | 今の回帰の数値（phase2-q §9 の表）の出どころ。§8 に `eos_weight` の削除のコミットを1行足す |
| `phase1-hiragana.md` | ADR 0002 | **残す（現行の基準）** | hiragana の基準 135 / 140 |
| `phase0-baseline-cases.tsv` | phase1-gates | **残す（現行の基準）** | `--baseline` に使う TSV（h-01 / a-01 の回帰の確認） |
| `phase0-baseline.md` | ADR 0002 §0 | 残す（判断の根拠、凍結） | ADR の前提 |
| `phase1-topic.md`、`phase1-quality.md`、`phase1-gates.md`、`phase1-ajimee.md` | ADR 0002 / 0003 | 残す（判断の根拠、凍結） | ADR 0003 の根拠。`phase1-quality.md` には `rare_lemma` の削除のコミットを1行足す |
| `phase1-lm-compare.md` | ADR 0003、THIRD_PARTY_NOTICES、`eval/src/generate.rs` | 残す（判断の根拠、凍結） | ADR 0003 §7.1-1 の根拠。先頭に「`ikari-eval gen` と `--lm-kind` は <コミット> で削除。再現はそのコミットで。edge は `zenz.model_kind` で選べる」と足す |
| `phase1-r1-policy.md` | なし | **archive** | `--bake-policy` と `dict-r1-baked` の暫定の結果。つまみを消すので再現できない |
| `dict-builder/reports/phase1-policy.md` | なし | **archive**（`eval/reports/archive/` に移す） | 除外の件数が今と違う。今のポリシーのレポートは辞書のビルドが `data/build/dict-policy-report.md` に毎回出す |

- `eval/reports/README.md` の欄: ファイル、日付、区分（現行の基準 / 判断の根拠（凍結） / archive）、何の数値か、今のコードで再現できるか。
- `eval/sets/REVIEW.md` は評価セットの確認の記録なので、`eval/sets/` に置いたままにする。

---

## 5. 洗い出し: テストとビルド

### 5.1 遅いテストと重複したテスト
- **遅いテスト**: なし（§0.1）。`eval/tests/flags.rs` の 2.9 秒も、バイナリの起動の統合テストとして妥当。性能の確認用の `#[ignore]`（`core/src/converter.rs:1120`、`core/src/analysis/lattice_analyzer.rs:790`）は、release で流すものとして `scripts/check.sh --perf` から呼ぶ（§6.2）。
- **重複**:
  - 辞書の fixture（`testdata/sudachi-mini`）を作る `BuildConfig { ... }` が 7 か所にある（`core/tests/dict_files.rs:48`、`core/tests/dict_v2.rs:31`、`cli/tests/phase0.rs:18`、`cli/tests/phase1.rs:23`、`eval/tests/flags.rs:32`、`eval/tests/report.rs:26`、`dict-builder/tests/build.rs:428`）。未知語の品詞とコストの定数も各ファイルに写されている。→ `ikari-dict-builder` に `#[doc(hidden)] pub mod fixture { pub fn build_sudachi_mini(out: &Path) }` を置き、各テストから呼ぶ（nice to have。値の食い違いを防ぐ程度の効果）。
  - `core/src/lattice.rs:216-` の `TestDict` は、`InMemoryDictionary` に表記の検索がなかったころの代わり（コメントの「レーンAの InMemoryDictionary に頼らないため」）。今は置き換えられる可能性があるが、28 か所で使っていて、得るものは小さい。**触らない**。
- **名前**: テストのファイルがフェーズの名前（`cli/tests/phase0.rs`、`phase1.rs`）とレーンの説明（`core/tests/dict_files.rs:1`「レーンB」）になっている。→ 内容の名前にする（`cli/tests/convert.rs`、`cli/tests/subcommands.rs`、`core/tests/dict_mmap.rs`、`core/tests/dict_v2.rs`）。
- **消えるテスト**: `nbest.rs:215`（E3）、`dict-builder/tests/v2.rs:138,305`、`dict-builder/src/policy.rs:629`（E4、E5）、`FFITests`（E9）、`lm/src/config.rs` の `model_kind` のテストは残す（E7）。

### 5.2 `#[ignore]` の棚卸し（19 件）
| ファイル | 件数 | 必要なもの | 問題 |
| --- | --- | --- | --- |
| `lm/tests/real_model.rs:100-433` | 7 | `IKARI_ZENZ_MODEL`、`--features metal` | **環境変数がないと黙って成功する**（`:29` で `?` で抜ける） |
| `lm/tests/real_model_edge.rs:47-258` | 5 | `IKARI_EDGE_MODEL`、`--features metal` | 同上（`:30`） |
| `eval/tests/scenario_lm.rs:67-79` | 3 | `IKARI_DICT_DIR`、`IKARI_ZENZ_MODEL`、`--features lm-metal` | 同上（`:30`） |
| `engine/src/tests.rs:1198,1251` | 2 | `IKARI_DICT_DIR`（と `IKARI_MODEL`） | 同上（`:1095`）。モデルの環境変数の名前が他と違う |
| `core/tests/dict_real.rs:16` | 1 | `IKARI_DICT_DIR` | 同上（`:12`） |
| `core/src/converter.rs:1120`、`core/src/analysis/lattice_analyzer.rs:790` | 2 | release | 性能の確認。問題なし |
| （参考）`eval/tests/flags.rs:434` | — | `IKARI_DICT_DIR` | `#[ignore]` が付いておらず、普段のテストで**黙って何もしない** |

直し方:
1. `#[ignore]` のテストは、明示して流したときに前提がなければ**失敗させる**（「`IKARI_ZENZ_MODEL` が要る」という panic）。`#[ignore]` と「黙って抜ける」の二重の門をやめる。
2. `eval/tests/flags.rs:434` に `#[ignore]` を付ける。
3. モデルの環境変数を `IKARI_ZENZ_MODEL` に揃える（`engine/src/tests.rs:1182`）。ユーザーデータは Rust と同じ `IKARI_USER_DIR` に揃え、`scripts/install-data.sh` の `IKARI_DATA_DIR` は互換のために当面は読む（警告を出す）。
4. 実データのテストをまとめて流す入口を `scripts/check.sh --real`（§6.2）にする。前提（辞書とモデルの場所）は setup と同じ既定値を使う。

### 5.3 スクリプトの整理（scripts/ と macos/scripts/ の二重構造）
今:
| 場所 | 中身 |
| --- | --- |
| `scripts/` | `install-data.sh` だけ |
| `macos/scripts/` | `build.sh`、`install.sh`、`smoke.sh`、`gen-proto.sh`、`lib.sh`、`templates/` |
| `eval/scripts/` | `fetch_ajimee.sh`、`make_long_context_set.py` |
| `testdata/sudachi-mini/` | `extract.py`、`extend.py`（fixture を作る道具） |
| `macos/tools/` | `make-icons.swift`、`select-input-source.swift` |

判断: **二重構造は「OS に依らないもの」と「macOS のバンドル」の責務の分け方として正しいので、残す。** 足りないのは、全体を通す入口と、データの取得と辞書のビルドのスクリプト。
- `scripts/`（OS に依らない、リポジトリ全体の入口）: `setup.sh`（新設、§6.3）、`fetch-data.sh`（新設）、`build-dict.sh`（新設）、`install-data.sh`（今のもの）、`check.sh`（新設、§6.2）。
- `macos/scripts/`（macOS のバンドル）: 今のまま。**名前と場所を変えない**（ADR 0005 §11.3 で U の持ち物として名指しされている）。`build.sh` は E10 の変更だけ。
- `eval/scripts/`（評価のデータの道具）: 今のまま。`fetch-data.sh` が `fetch_ajimee.sh` を呼ぶ。
- `testdata/`、`macos/tools/`: 今のまま。
- 共通の関数（`log`、`die`）: `scripts/` 側は `scripts/lib.sh` に置き、`macos/scripts/lib.sh` とは**共有しない**。3 行の重複より、`macos/scripts` が上の階層のファイルに依存する結合の方が高くつく（macos/ を単独で読めなくなる）。

### 5.4 1 コマンドの案（§6.3 に詳細）
`scripts/setup.sh` を採用する。`make install` と `just` は却下（§5.6）。

### 5.5 CI の雛形を置くか
**置かない。** 理由:
- リモートがない（`git remote -v` が空）。動かない CI の定義は、置いたとたんに古くなる。
- 意味のある検査（辞書、モデル、署名、macOS 27 / Xcode 27）は、ホスト型のランナーでは揃わない。揃うのは `cargo test` と `swift test` だけで、それは手元で 10 秒で終わる。
代わりに:
- `scripts/check.sh` を「CI と同じ門」として作る（§6.2）。将来 CI を置くときは、ワークフローはこれを呼ぶ 10 行で済む。
- コミットの前に流す運用は、各レーンの完了条件（§8.3）に入れる。
- 見直す条件: GitHub にリモートを作ったとき、またはエージェントの並行レーンが 4 本を超えたとき。

### 5.6 却下した入口の案
| 案 | 却下の理由 |
| --- | --- |
| `Makefile`（`make install`） | パスに空白がある（`~/Library/Application Support`、`~/Library/Input Methods`）。make の依存の追跡が効くのは辞書のビルドだけで、それはスタンプのファイル（§6.3）で足りる。既存のスクリプトはすべて bash |
| `just` | 手元にはあるが、この repo の誰も前提にしていない新しい道具を増やす |
| `cargo xtask` | Rust で書くと、codesign、xcodebuild、launchctl を呼ぶ処理が冗長になる。既存の `macos/scripts` を書き直すことになる |
| `macos/scripts/install.sh` に辞書のビルドまで入れる | macOS のバンドルのスクリプトが、OS に依らないデータの処理を持つことになる。Windows 版の段で分け直す |

---

## 6. 新しい入口の設計

### 6.1 等価性の基準を取る（`eval/scripts/snapshot.sh`、新設）
リファクタの前に基準を取り、各レーンのマージのあとに同じものを取り直して比べる。
```
eval/scripts/snapshot.sh <出力ディレクトリ> [--lm <gguf>] [--ajimee]
```
- 実行する構成（`--out-cases` を書く）:
  | セット | 構成 |
  | --- | --- |
  | `homophones.tsv` | `--ranker cost`、`--ranker topic`、`--ranker full`、`--ranker full --lm` |
  | `homophones-ext.tsv`（全件） | `--ranker topic`、`--ranker topic --lm`、`--ranker full` |
  | `hiragana.tsv` | `--ranker topic`、`--ranker topic --lm` |
  | AJIMEE（`--ajimee`） | `--ranker topic`、`--ranker topic --lm` |
  | シナリオ a-02 / a-03 / a-04 | `--scenario`（LM なしとあり） |
- **LM ありの構成は `--lm-budget-ms 2000 --lm-deadline-ms 2000` で流す**。時間切れがあると順位が実行ごとに揺れるので、時間切れ 0 を確かめた結果だけを比べる（レポートの「時間切れ」の欄が 0 でなければ、その回は無効にして流し直す）。既定の締め切りでの数値は、参考としてレポートを保存する。
- あわせて記録するもの: 辞書の 3 ファイルの sha256、`ikari --format tsv` の出力（読み 20 件 × 文脈あり／なし。セットは `eval/scripts/cli-golden.tsv` に置く）、`smoke.sh --installed` の出力、各レポートの warm p99。
- 出力は `target/refactor-baseline/<コミット>/`（gitignore 済み。コミットしない。辞書が手元にしかないため）。

### 6.2 `scripts/check.sh`（新設。CI と同じ門）
```
scripts/check.sh            # fmt --check、clippy -D warnings、cargo test --workspace、swift test
scripts/check.sh --lm       # + cargo test/clippy -p ikari-lm -p ikari-engine-ffi --features lm-metal（cmake が要る）
scripts/check.sh --real     # + 実データの #[ignore] テスト（辞書とモデルの既定の場所を使う）
scripts/check.sh --perf     # + 性能の #[ignore] テスト（release）
scripts/check.sh --eval <基準のディレクトリ>   # snapshot を取り、基準と §7.1 の規則で比べる
```

### 6.3 `scripts/setup.sh`（新設。辞書のビルドからインストールまで）
```
scripts/setup.sh [--signing team|adhoc] [--no-lm] [--fetch] [--rebuild-dict] [--skip-app] [--skip-smoke]
```
| 段 | 処理 | 呼ぶもの | 飛ばす条件 |
| --- | --- | --- | --- |
| 0. 前提の確認 | cargo、swift、xcodebuild、codesign。`--no-lm` でなければ cmake。`team` なら証明書（`build.sh` の `resolve_identity` と同じ判定）。ディスクの空き（目安 1 GB） | — | — |
| 1. 取得 | SudachiDict Core 20260723（small_lex / core_lex / matrix.def の zip）、chiVe 1.3 mc90、AJIMEE、zenz-v3.2-small Q5_K_M。**URL とバージョンと sha256 をスクリプトに固定**し、展開してから sha256 を確かめる | `scripts/fetch-data.sh`（AJIMEE は `eval/scripts/fetch_ajimee.sh`） | `--fetch` を付けたときだけ。ないファイルがあれば、取得の方法を出して止まる（黙って通信しない） |
| 2. 辞書 | `ikari-dict-builder --policy dict-builder/policy/chive.tsv --chive-vocab <chiVe> --lex small_lex.csv --lex core_lex.csv --matrix matrix.def --out data/build/dict.new --policy-report data/build/dict-policy-report.md` → `ikari-dict-builder vectors --chive … --dict data/build/dict.new --out data/build/dict.new/vectors.bin` → `--verify data/build/dict.new` → `dict` を `dict.prev` に、`dict.new` を `dict` に入れ替える。`data/build/dict/BUILD_INFO` に入力（ファイルの大きさと sha256、ポリシーの sha256、`git rev-parse HEAD:dict-builder` と `HEAD:dict-format`）と出力の sha256 を書く | `scripts/build-dict.sh` | `BUILD_INFO` の入力が同じなら飛ばす（`--rebuild-dict` で強制）。ビルドは決定的（`builds_are_deterministic`）なので、飛ばしても結果は同じ |
| 3. データの配置 | `~/Library/Application Support/ikari-ime/{dict,models}/` に tmp を経て rename で入れる（今の実装のまま） | `scripts/install-data.sh` | 同じ内容のファイルは今も飛ばしている |
| 4. アプリ | `build.sh --signing S [--no-lm]` → `install.sh --signing S` | `macos/scripts/build.sh`、`install.sh` | `--skip-app` |
| 5. smoke | `smoke.sh --installed --signing S` | `macos/scripts/smoke.sh` | `--skip-smoke` |

- `install.sh --build` は `--no-lm` を `build.sh` に渡せない（`parse_common_flag` が拒む）ので、setup は `build.sh` と `install.sh` を別々に呼ぶ。`install.sh` の引数は変えない（U の持ち物）。
- 入力ソースの追加（初回だけ）は自動化しない（`install.sh` の方針「入力ソースの設定は変えない」のまま）。setup の最後に、未登録なら README の該当の節を出す。
- 定数（辞書のバージョン、chiVe の名前、モデルのファイル名）は `scripts/lib.sh` に 1 か所で持つ。`engine/src/lib.rs:50` の `MODEL_FILE_NAME` との一致は、`check.sh` で grep して確かめる（言語をまたぐので生成まではしない）。
- `fetch-data.sh` の URL は、リードが一次配布元で確かめて固定する。sha256 は、手元の `data/raw/` のファイルから取る（基準の取得のときに記録する）。ライセンスの確認は spec のとおり人が行い、スクリプトの先頭に配布元とライセンスを書く。
- **製品（殻とエンジン）は通信しない、という NFR-17 には影響しない**。取得は開発の道具で、明示のフラグがあるときだけ動く。

---

## 7. 振る舞いを変えないことの確かめ方

### 7.1 一致の判定
| 対象 | 判定 |
| --- | --- |
| eval（LM なし） | `--out-cases` の列 1〜7（id、kind、context、reading、expected、top1、rank）がバイト単位で一致。`ikari-eval compare` の差分が 0 件 |
| eval（LM あり） | 時間切れ 0 の条件（§6.1）で同じ判定 |
| シナリオ | 全 probe が PASS（前後で同じ） |
| 速さ | 各レポートの warm p99 が基準の 1.1 倍以内（`IKARI_MARGIN` を消すと速くなる方向） |
| 辞書 | L1 の後に `build-dict.sh --rebuild-dict` で作り直した 3 ファイルの sha256 が基準と一致（E2、E4、E5 で出力が変わらないことの確認） |
| CLI | golden の出力がバイト単位で一致 |
| smoke | PASS の項目の集合が同じで、FAIL が 0。Convert の p99 が 16ms 以下 |
| Swift | `swift test` が全件通る（件数は E9 の 1 件だけ減る） |

### 7.2 基準の数値（phase2-q §9。step 0 で 6a58b73 を測り直し、それを正にする）
| セット | LM なし（R3） | LM あり（R3 + small） |
| --- | --- | --- |
| homophones | 12 / 14（cost 6 / 14） | full + LM 13 / 14 |
| hiragana | 135 / 140 | 135 / 140 |
| ext dev（話題） | 416 / 659（63.1%） | 77.7% |
| ext test（話題） | 101 / 173（58.4%） | 75.7% |
| 否定 / 文法（ext 全件） | 179 / 180、254 / 254 | 180 / 180、254 / 254 |
| AJIMEE | 21.0% | 51.0% |

---

## 8. 実装の進め方

### 8.1 3a との順序
```
[S0 スパイク（リード、進行中、捨てるブランチ）] ───────────────┐
step 0（リード）基準の取得 ─→ L1 ┐                             │
                            L2 ├─ マージ: L1 → L2 → L3 ─→ step 4（リードの最終確認）─→ 3a 契約コミット ─→ E3 / S3 / U
                            L3 ┘
```
- **このリファクタは、3a の契約コミットの前にすべてマージする。** 契約コミットとレーン E3 / S3 / U が触るファイル（§0.3）のうち、リファクタが触るのは次だけで、すべて契約コミットより前に入る: `engine/src/lib.rs`（定数の置き換えだけ）、`engine/src/tests.rs`（環境変数の名前だけ）、`stack/src/lib.rs`（分割。`Reuse` と `build_stack_with` は lib.rs に残す）、`core/src/user/dir.rs`（定数を足すだけ）、`core/src/user/util.rs`（FNV の置き換え）、`macos/Package.swift`（E9）、`macos/Sources/IkariSmoke/**`（分割と E8）、`macos/Sources/IkariShellCore/LmBudget.swift`（改名）、`macos/scripts/build.sh`（E10）、`macos/README.md`。
- **ADR 0005 が名指しするパスは変えない**: `engine/src/admin.rs`（新設予定）、`engine/src/lib.rs` の `dispatch` と `refresh_data`、`stack/src/lib.rs` の `Reuse`、`core/src/user/**`、`macos/scripts/{build,install,lib}.sh`、`macos/Sources/{IkariShellCore,IkariIME,IkariXPC,IkariSettings,IkariSmoke}`、`IkariXPC/Identifiers.swift`、`docs/phase2-contract.md`、`proto/**`、`gen-proto.sh`。
- リファクタの間、3a の契約コミットは入れない。S0 は自分のブランチで続けてよい（マージしないので衝突しない）。S0 の結果を ADR 0005 §0.1 に書くのはリードで、L3 は ADR 0005 の本文に触れない（索引の状態だけ）。
- 3a の後に回すもの（§3.1）: `engine/src/lib.rs` と `engine/src/tests.rs` の分割、`core/src/user/dictionary.rs` の分割、`InputStateMachine.swift`。

### 8.2 step 0（リード、半日）
1. `eval/scripts/snapshot.sh` と `eval/scripts/cli-golden.tsv` を入れる（道具だけのコミット。振る舞いは変えない）。
2. 6a58b73 + 1 で snapshot を取り、`target/refactor-baseline/<コミット>/` に置く。辞書の sha256、`data/raw/` の sha256、smoke の出力も記録する。
3. ユーザーの確認を取って、マージ済みの worktree 11 個とブランチを消す（§10-1）。
4. 各レーンの worktree を作る。

### 8.3 レーン（ファイルの境界で分ける）
| レーン | 持つファイル | 中身（§の番号） | 完了条件 |
| --- | --- | --- | --- |
| **L1: core と辞書** | `core/**`（`core/src/user/` は `dir.rs` と `util.rs` だけ）、`input/**`、`dict-format/**`、`dict-builder/**`（`policy/*.tsv` を含む）、`testdata/**` | E1、E2、E3、E4、E5、D6（`dir.rs` に定数を足す）、D11（core の分）、D13（core と dict-format の分）、D15、§2.3 の core と dict-builder の分、§3.1 の `converter.rs` の分割、§3.4 の `ranker/` の構成、E11（pyc）、E12（L1 のファイルの分）、§5.1 の core のテストの名前、§5.2 の core の分 | `scripts/check.sh` が通る（step 0 で `check.sh` がまだなければ同じコマンドを手で）。§7.1 の eval、辞書の sha256、CLI の golden が一致。`--ranker-config` に未知のキー（`grammar_guard.*` を含む）を書いたときの挙動が前と同じ |
| **L2: 組み立てと道具** | `stack/**`、`cli/**`、`eval/src/**`、`eval/tests/**`、`eval/Cargo.toml`、`engine/**`（§8.1 の範囲だけ）、`engine-ffi/**`、`import/**`、`lm/**` | D1〜D5、D7〜D10、D11（lm の `pub` をやめる）、D12、D13（eval の分）、D14、D16、E6、E7（`--lm-kind` の削除）、§2.3 の lm と stack の分、§3.1 の `eval/src/lib.rs`、`eval/src/main.rs`、`lm/src/ranker.rs`、`stack/src/lib.rs`、§3.2 の feature の整理、E12（L2 のファイルの分）、§5.2（engine、eval、lm の分）、`eval/src/json.rs` を `serde_json` に置き換える（AJIMEE の 200 件の読み込みの結果が同じことを確かめる） | `check.sh` と `check.sh --lm` が通る。§7.1 の eval（LM あり・なし、AJIMEE を含む）、CLI の golden、smoke が一致（smoke は L3 のマージ前なので、`build.sh` と `install.sh` を手で）。`zenz.model_kind=kanakanji-edge` を設定ファイルで渡して `real_model_edge` のテストが通る |
| **L3: ドキュメント、スクリプト、macOS** | `README.md`、`macos/README.md`、`docs/**`（ADR の本文は状態の行とリンクのパスだけ。`docs/phase2-contract.md` は触らない）、`eval/reports/**`、`dict-builder/reports/**`（archive への移動）、`scripts/**`、`eval/scripts/fetch_ajimee.sh`（呼び出し方だけ）、`macos/Package.swift`、`macos/Tests/IkariFFITests/**`、`macos/Sources/IkariSmoke/**`、`macos/Sources/IkariShellCore/LmBudget.swift`（改名だけ）、`macos/scripts/build.sh`（E10 だけ）、`macos/tools/.gitkeep`、`.gitignore`、`THIRD_PARTY_NOTICES.md` | §4 のすべて、§5.3、§6.2、§6.3、E8、E9、E10、E11（`.gitignore`）、E12（L3 のファイルの分）、spec の ADR 0005 §15.1-7 で決まった箇所（`docs/ikari-ime_spec.md:289` とロードマップ） | **きれいな worktree**（`data/` だけを本体からつなぐ）で `scripts/setup.sh --signing team` が最後まで通り、smoke が §7.1 で一致。`--signing adhoc` でも `build.sh` と `install.sh` が通る。`swift test` が通る。README の手順を上から順に手で流して通る（リードが確かめる）。文書の相対リンクが切れていない（`check.sh` に grep の検査を足す） |

- レーンの間の依存:
  - L2 の D6（`UserDir::engine_lock()` を使う）は L1 の定数に依存する。L2 はこれを最後のコミットにし、L1 のマージの後に rebase して入れる。
  - L3 のドキュメントは L2 の最終形（`--lm-kind` の削除、環境変数の名前）を書く。書くのは並行でよいが、マージは最後にする。
  - それ以外にファイルの重なりはない（`engine/**` は L2 だけ、`macos/**` は L3 だけ）。
- コミットの粒度: §2〜§5 の項目ごとに 1 コミット。移動だけのコミット（ファイルの分割）と中身を変えるコミットを混ぜない（レビューで差分を読めるように）。各コミットで `check.sh` が通ること。

### 8.4 マージの順と、マージごとの確認
1. **L1**（いちばん下の層。他のレーンが rebase で吸収する）。マージ後: `check.sh`、snapshot の比較、辞書の作り直しと sha256。
2. **L2**（L1 の上で rebase し、D6 を足す）。マージ後: `check.sh --lm`、snapshot の比較（LM あり）、CLI の golden、smoke。
3. **L3**（最後。ドキュメントが最終の状態を書く）。マージ後: きれいな worktree で `setup.sh`、smoke。
4. **step 4（リード）**: `check.sh --lm --real --perf --eval <基準>` をすべて流し、結果をこの文書の末尾（§11）に記録する。ADR の索引を最終の状態にする。そのあと 3a の契約コミットに進む。

### 8.5 優先度（負債の評価）
優先度 = (利子 + リスク) / 返済コスト。高・中・低を 3・2・1 として数えた。

| 項目 | 返済コスト | 利子 | リスク | 優先度 |
| --- | --- | --- | --- | --- |
| E1、E2（本番の環境変数） | 1 | 1 | 3 | 4.0 |
| §5.2（黙って成功する `#[ignore]`） | 1 | 1 | 3 | 4.0 |
| D1〜D3（名前、設定の読み取り、LM のキー） | 1 | 3 | 2 | 5.0 |
| D4、D5（CLI の組み立て、エラーの部分一致） | 2 | 2 | 2 | 2.0 |
| E3〜E7（決着したつまみ） | 1 | 2 | 1 | 3.0 |
| §6.3（setup.sh） | 2 | 3 | 2 | 2.5 |
| §4.2（インストール手順の一本化） | 2 | 3 | 1 | 2.0 |
| §3.4（ranker/ の構成） | 2 | 2 | 1 | 1.5 |
| D11〜D13（かな、FNV） | 1 | 1 | 2 | 3.0（ただし同値性のテストを先に書く） |
| §3.1 のテストの移動 | 1 | 1 | 1 | 2.0 |
| §5.1 の fixture の共通化 | 1 | 1 | 1 | 2.0（nice to have） |

レーンの中では、この順に手を付ける。時間が足りなければ、優先度 2.0 未満と nice to have を 3a の後に回す。

---

## 9. 提案（ADR かユーザーの決定を変えるもの。別に承認を取る）

| # | 提案 | 変える決定 | 推奨 | 理由 |
| --- | --- | --- | --- | --- |
| P1 | kanakanji-edge の実装（`ModelKind`、`real_model_edge.rs`、THIRD_PARTY_NOTICES の項）を完全に消す | ADR 0003 §7.1-1「edge の実装は残す」 | **今回はしない**（E7 の「入口を絞って残す」を採る） | 残すコストは約 40 行と、手で流す実モデルのテストだけ。比較を流し直す価値の方が大きい。消すのは、edge を 2 回続けて比べなかった、または zenz v4 などで前提が変わったとき |
| P2 | core の `ikari_input` の再公開（`core/src/lib.rs:32-38`）をやめ、cli が `ikari-input` に直接依存する | ADR 0004 §3.1、§3.3「core::input は ikari-input を再公開。既存の API は変えない」 | 承認されれば L2 で行う（cli 側 3 ファイル）。core 側の削除は L1 | 再公開を使うのは cli だけ。同じ型に 2 つのパスがあると、新しいコードがどちらを使うかで揺れる |
| P3 | 辞書のポリシーの `exclude`（除外）の仕組みを消す（形式の `IME_EXCLUDED` の読み取りは残す） | phase2-q §7.1「`exclude` の仕組みは残した」。ADR 0003 §7.3「除外は大きなペナルティに改め、候補には残す」 | 推奨（L1） | 今のポリシーは `exclude` を使っていない。仕組みが残っていると、基本原則（選択肢を不当に制限しない）に反する設定を書けてしまう。辞書の出力は変わらない |
| P4 | `[workspace] default-members` をやめる | ADR 0002 §2、ADR 0004 §3.1 | 推奨（L3。`Cargo.toml`） | 根拠の「llama.cpp のネイティブビルドを伴う」は、feature を付けなければ成り立たない（§0.1 で確かめた）。今は `cargo test`（`--workspace` なし）が `ikari-lm` の単体テスト 39 件と `engine-ffi` の 5 件を黙って飛ばす |
| P5 | spec の「リポジトリ構成（案）」と「未決事項」を、決着した項目への ADR のリンクに置き換える | spec（ユーザーの文書） | ユーザーが決める | 未決のまま見えるものが、決着済みになっている。ADR 0005 §15.1-7 で決まった箇所（L3）以外は、ユーザーの判断なしには直さない |
| P6 | eval の辞書ディレクトリの探し方を CLI と揃える（ユーザーデータの `dict/` も探す） | — | **しない** | 揃えると、eval が `data/build/dict` の代わりに、入れてある辞書を黙って使い出す。評価の再現性が落ちる |

---

## 10. 懸念点とユーザーの判断が必要なこと

1. **ローカルのデータの削除**（ユーザーの確認が要る）:
   - マージ済みの worktree 11 個とブランチ（合計 約 19 GB の大部分）。**S0 の `worktree-agent-ab805d357ee243769` は消さない。**
   - `data/build/dict-r1-baked`（168 MB。E4 で不要）。
   - `data/build/dict.prev` は残す（spec の「旧版は残し、いつでも戻せる」。`build-dict.sh` が以後 1 世代を管理する）。
2. **D2 はエラーの経路だけ挙動が変わる**（cli の設定ファイルで空のキーを拒む）。正常な入力の結果は同じ。
3. **D11 / D13 の置き換えは、同値性のテストを先に書いてから行う**。lm のかなの変換（踊り字）は、プロンプトが変わりうるので置き換えない。
4. **LM ありの等価性は、時間切れ 0 の条件でしか判定できない**。既定の締め切りでの数値は、時間切れの件数の差で揺れうる。差が出たら、時間切れの件数で説明できることを確かめる。
5. **`fetch-data.sh` の URL**: SudachiDict の raw の配布元、chiVe、zenz の配布元の URL は、この計画では固定していない。リードが一次配布元で確かめる。ライセンスの確認は spec のとおり人が行う。
6. **辞書の作り直しにかかる時間**: setup の 2 段目は数分〜十数分かかる見込み（chiVe の読み込みと vectors.bin の k-means）。スタンプで飛ばすので、2 回目以降はかからない。最初の測定値を step 0 で記録する。
7. **3a の開始が遅れる**: 3 レーンを並行して 2 日前後、step 0 と step 4 で 1 日の見込み。S0 と並行に進めれば、3a の契約コミットまでの遅れは 1〜2 日。時間を優先するなら、§8.5 の優先度 2.0 未満を 3a の後に回す。

---

## 11. 結果の記録（step 4 でリードが書く）
| 項目 | 基準（step 0） | 最終 | 一致 |
| --- | --- | --- | --- |
| eval（LM なし）の列 1〜7 | | | |
| eval（LM あり、時間切れ 0） | | | |
| シナリオ a-02〜a-04 | | | |
| 辞書の sha256 | | | |
| CLI の golden | | | |
| smoke | | | |
| `cargo test` / `swift test` の件数 | 814 / 126 | | |

## 12. 決定（2026-10-12）
| 項目 | 決定 | 決めた人 |
| --- | --- | --- |
| P1 | edge は完全には消さない（E7 のとおり、入口を `zenz.model_kind` だけに絞って残す） | リード（案どおり） |
| P2 | core による `ikari_input` の再公開をやめる。cli は ikari-input に直接依存する | リード（案どおり） |
| P3 | ポリシーの `exclude` の仕組みを消す（基本原則に合わせる） | リード（案どおり） |
| P4 | `default-members` をやめる | リード（案どおり） |
| P5 | spec の「リポジトリ構成（案）」と「未決事項」で決着した項目を、ADR へのリンクに置き換える（L3）。要件の本文は変えない | ユーザー |
| P6 | しない | リード（案どおり） |
| §10-1 | マージ済みの worktree 11個と `data/build/{dict-r1-baked,dict.prev}` は、ユーザーの承認を得て削除した（S0 の worktree は残す） | ユーザー |
