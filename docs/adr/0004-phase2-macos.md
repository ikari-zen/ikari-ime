# ADR 0004: フェーズ2 macOS 版（常駐エンジン、XPC、InputMethodKit の殻）

> 注（2026-10-25）: bundle ID の頭は [ADR 0036](0036-bundle-id-migration.md) で `<旧 ID の頭>` から `io.github.ikari-zen` に移した。この文書の識別子は新しい値に直してある。チームIDと証明書のIDは文書に書かず、`macos/scripts/lib.sh` の `TEAM_ID` を参照する。

- 状態: 承認（2026-10-11。§15.1 の決定を参照）
- 日付: 2026-10-11
- 対象: フェーズ2（macOS の殻、常駐エンジン、IPC）と、ADR 0003 §6 の持ち越し課題
- 前提: main = d2219e5。ADR 0001〜0003（0003 §7.1 / §7.2 の決定を含む）。spec の UI、便利な変換、文脈の取得元、NFR-01〜20、プロセス間通信、OS別の実装メモ（macOS）、表記とアイコン

---

## 0. 現状の把握

### 0.1 手元の環境（2026-10-11 に実際に確認した）

| 項目 | 状態 |
| --- | --- |
| OS | macOS 27.0.1（26A434）、Apple M4、16GB |
| Xcode | 27.0（27A266a）。SDK は MacOSX27.0。Swift 6.4（swiftlang-6.4.0.34.1） |
| Rust | rustc / cargo 1.99、ターゲットは `aarch64-apple-darwin` だけ |
| 署名の ID | **`Apple Development` の証明書が1つある（チームIDあり。期限 2027-05-23）**。ほかに自己署名の `keymapper-local` がある |
| ツール | cmake 4.4.4 はある。**protoc、protoc-gen-swift、cbindgen、uniffi-bindgen、xcodegen はない**。Homebrew 7.0.9 で入手できる版は protobuf 36.2、swift-protobuf 1.38.1、xcodegen 2.46.0 |
| crates.io | prost 0.14.4、prost-build 0.14.4、protox 0.10.0（純 Rust の protobuf コンパイラ）、uniffi 0.32.2 |
| InputMethodKit（SDK 27） | ヘッダに変化はなく、非推奨の印もない（`IMKInputController`、`IMKServer`、`IMKCandidates`、`IMKTextInput` はすべて 10.5〜10.7 からある API）。公式ドキュメントの索引にも非推奨の項目はない |
| XPC の署名検証 | `NSXPCConnection.setCodeSigningRequirement(_:)`（macOS 13 以降）と、C の `xpc_connection_set_peer_code_signing_requirement` がある。リスナー側の `setConnectionCodeSigningRequirement` は、**匿名リスナーと Mach サービスのリスナーにしか使えない**（ヘッダの注記）。同梱の XPC サービス（`NSXPCListener.service()`）では、接続ごとに `setCodeSigningRequirement` を呼ぶ |
| llama.cpp（llama-cpp-sys-2 0.1.159） | `GGML_METAL_EMBED_LIBRARY` は既定で ON（Metal のシェーダを埋め込むので、Sandbox の中でも外部のファイルを読まない）。リンクするのは Foundation、Metal、MetalKit、Accelerate、c++ |
| Sandbox と秘密の入力欄 | `IsSecureEventInputEnabled()`（HIToolbox）が SDK にある |

### 0.2 コードの現状（設計に効く事実）
- **ユーザーデータの既定の置き場所はリードが決めている**（`~/Library/Application Support/ikari-ime`、ADR 0002 §21.1-8）。辞書は `./data/build/dict` で、モデルはパスを指定する。
- **Converter、Ranker、LM の組み立ては、`cli/src/stack.rs`（392行）と `eval/src/setup.rs`（615行）にほぼ同じものが2つある。** エンジンが3つ目の写しになる前に、共有の crate へ移す（技術的負債。利子が高く、返済は機械的）。
- `ZenzRanker` の待ちは `rank()` の呼び出しごとで、1つの変換で複数回呼ばれうる（ADR 0003 §2.3）。`RankInput` に締め切りがない。
- ローマ字とかなの入力（`core::input`）は純粋な処理で、辞書に依存しない。殻（IME のプロセス）でも使える。
- ADR 0003 §7.1 / §7.2 で決まったこと:
  - LM は zenz-v3.2 small の1つだけ。
  - 待ち時間は `max(30ms, 読み1文字あたり 6ms)`、上限 300ms。
  - 長文の最適化はしない。

### 0.3 フェーズ2の範囲（リードの案）への意見
次の5点を範囲に足し、または直した。
1. **追加: 共有の組み立てを crate に移す**（`ikari-stack`。§3.2）。エンジン、CLI、eval が同じ組み立てを使う。
2. **追加: 入力の処理を `ikari-input` crate に分ける**（§3.3）。殻はエンジンなしでもローマ字入力ができる必要がある（NFR-16）。殻に辞書や llama.cpp を持ち込まずに、Rust の入力処理だけをリンクするため。
3. **追加: 設定ファイル（TSV）**。キー割り当てのプリセット、入力方式（ローマ字／かな）、ページの件数を設定する。設定の画面はフェーズ3（spec では、ユーザー辞書の画面と取り込みの画面がフェーズ3）。
4. **完了条件から外す（フェーズ2の中で行うが、完了をこれで縛らない）**: EOS の実験、変換の質の残課題（`時が経つ`、`ココ`、数字の書式など）。どちらも評価の結果次第で、殻とエンジンの完成とは独立している。
5. **フェーズ2に入れない**: 辞書の自動更新（独立した更新プロセス）、Windows、設定の画面、注釈のデータ。注釈の入手元は spec の未決事項で、フェーズ2では2行目の枠だけを作る。

---

## 1. 決定の要約

| 項目 | 決定 |
| --- | --- |
| プロセス構成 | `ikari.app`（IMK の殻、Swift、Sandbox なし）の中に、`IkariEngine.xpc`（Swift の薄い殻 + Rust のエンジンの静的ライブラリ、**App Sandbox、ネットワークなし**）を同梱する |
| Rust と Swift の境界 | **C ABI を手で書く。メッセージは protobuf のバイト列**。エンジンの FFI は5関数（バイト列を受けてバイト列を返す）。殻の FFI は入力の処理だけ（約10関数）。UniFFI と cbindgen は使わない |
| IPC | `NSXPCConnection(serviceName:)` で同梱の XPC サービスにつなぐ。プロトコルは `handle(Data) -> Data` の1メソッドだけで、中身は protobuf。Mach サービスは登録しない |
| 署名の検証 | 双方向に `setCodeSigningRequirement`。**Apple Development の証明書でチームIDに固定することを推奨**（ad-hoc では identifier しか見られない） |
| protobuf | Rust は `prost` + `protox`（protoc 不要、build.rs）。Swift は `swift-protobuf` 1.38.x で、**生成したコードをコミットする**（再生成するときだけ protoc と protoc-gen-swift が要る） |
| ビルド | **SwiftPM + シェルスクリプト**。Rust は `cargo` で staticlib を作り、XCFramework に包んで SwiftPM の `binaryTarget` にする。バンドルの組み立て、署名、インストール、`killall`、入力ソースの再選択はスクリプトで行う。Xcode プロジェクトは作らない |
| 候補ウィンドウ | `IMKCandidates` は使わず、自前の `NSPanel` で作る（横一列、番号、区切り線、2行固定、配置の規則） |
| 合成（ローマ字→かな） | **殻のプロセスで行う**（Rust の `ikari-input` を FFI で呼ぶ）。キーを打つたびにエンジンへ往復しない。エンジンが落ちても入力できる |
| 先行計算 | 読みが変わるたびに、殻がエンジンへ `Prefetch` を投げる（返事を待たない）。エンジンは最新のものだけを計算してキャッシュする。スペースで送る `Convert` は、キャッシュに当たればすぐ返す |
| 締め切り | **変換1回（`convert` か `page` の1回の呼び出し）につき1つ**。`RankInput.deadline` で全 Ranker に渡す。長さ = `max(30ms, 6ms × 読みの文字数)`、上限 300ms |
| データの置き場所 | `~/Library/Application Support/ikari-ime/`（`dict/`、`models/`、ユーザーデータ、`settings.tsv`）。エンジンには、Sandbox の一時例外の entitlement でこのディレクトリだけを許す |

---

## 2. プロセスとバンドルの構成

```
~/Library/Input Methods/ikari.app                  ← Bundle ID: io.github.ikari-zen.inputmethod.ikari（§12 で決める）
  Contents/
    Info.plist                                     ← IMK のキー（§6.1）
    MacOS/ikari                                    ← 殻（Swift + libikari_shell.a）。Sandbox なし、Hardened Runtime
    MacOS/ikari-smoke                              ← XPC 経由の E2E テスト用（§10）
    Resources/                                     ← アイコン（PDF）、既定の settings.tsv、keymap のプリセット、romaji.tsv
    XPCServices/IkariEngine.xpc
      Contents/Info.plist                          ← CFBundleIdentifier: io.github.ikari-zen.inputmethod.ikari.engine、XPCService.ServiceType=Application
      Contents/MacOS/IkariEngine                   ← Swift の薄い殻 + libikari_engine.a（core、lm、llama.cpp を含む）
                                                      App Sandbox、ネットワークの entitlement なし
~/Library/Application Support/ikari-ime/
  dict/{grammar.bin, system.dict, vectors.bin}     ← install-data.sh が data/build/dict からコピーする
  models/zenz-v3.2-small-Q5_K_M.gguf
  user_dict.tsv, learning.tsv, profile.tsv, ingest_*.tsv   ← 既存の UserDir の配置のまま
  settings.tsv                                     ← 殻とエンジンの設定（新設）
  romaji.tsv                                       ← あれば同梱の表を置き換える（ADR 0001 の方針）
  engine.lock                                      ← エンジンが動いている間 flock で持つ（§8.3）
  logs/engine.log                                  ← 文字列を含まないログ（§8.4）
```

- **XPC サービスを同梱する理由**:
  - 同梱の XPC サービスは、**それを含むアプリからしか見えない**（launchd がアプリごとの名前空間で管理する）。spec の「サービス名は launchd に管理させ、同梱したサービスだけが待ち受ける」をそのまま満たす。
  - 起動、終了、クラッシュ後の再起動は launchd が行う（次のメッセージで自動的に起動する）。NFR-16 の「自動で再起動」を自分で書かずに済む。
- **殻を Sandbox に入れない理由**:
  - キャレットの位置が取れないときのフォールバック（spec の配置の節 ①、アクセシビリティ API）は、Sandbox の中では使えない。
  - IMK の殻はもともと独立したプロセスで（Windows の TSF のように他のアプリに読み込まれるわけではない）、通信の機能も持たない。
  - 殻は Hardened Runtime だけにする。
- **エンジンを Sandbox に入れる**（NFR-17、spec の IPC の節）: `com.apple.security.app-sandbox` を有効にし、`com.apple.security.network.*` は付けない。ファイルは §8.2 の一時例外の1つだけ。

---

## 3. Rust 側の crate 構成

### 3.1 依存の向き（追加分）
```
ikari-input  ←  ikari-core（core::input は ikari-input を再公開。既存の API は変えない）
     ^                ^
     |          ikari-stack（cli/stack.rs と eval/setup.rs の共通部分。feature "lm"）
     |                ^
ikari-shell-ffi  ikari-engine（セッション、先行計算、キャッシュ、protobuf の処理。feature "lm"）
 (staticlib)          ^
                 ikari-engine-ffi（staticlib。feature "lm-metal"）
ikari-proto（prost の生成コード。build.rs で protox）← ikari-engine
```
- `ikari-cli` と `ikari-eval` は `ikari-stack` を使うように直す（写しを消す）。
- 新しい crate は workspace に加える。`ikari-engine-ffi` は llama.cpp を含むので、`default-members` の外に置く（ADR 0002 §2 と同じ扱い）。

### 3.2 `ikari-stack`（共有の組み立て）
```rust
pub struct StackPaths { pub dict_dir: PathBuf, pub user_dir: Option<PathBuf>, pub vectors: Option<PathBuf>, pub lm_model: Option<PathBuf> }
pub enum RankerSet { Cost, Analyzer, Doc, Topic, Full }          // eval の --ranker と同じ意味
pub struct StackOptions { pub ranker: RankerSet, pub ranker_config: BTreeMap<String, String>, pub profile: ProfileSel, pub convert: ConvertOptions }
pub struct Stack { pub converter: Arc<Converter>, pub learning: Option<Arc<LearningStore>>, pub lm: Option<Arc<dyn LmHandle>>, pub meta: StackMeta }
pub fn build_stack(paths: &StackPaths, opts: &StackOptions) -> Result<Stack, StackError>;
pub trait LmHandle: Send + Sync { fn warm_up(&self); fn stats(&self) -> LmStats; fn is_loaded(&self) -> bool; }
```
- 中身は `cli/src/stack.rs` を基準にし、eval の固有の部分（`--profile-ab`、RSS の計測など）だけを eval に残す。
- 移すときに挙動を変えない。確認は eval の ext test の結果がビット単位で一致すること（`--out-cases` の比較）。

### 3.3 `ikari-input`
- `core/src/input/**` と `core/assets/romaji.tsv` を、そのまま移す。`ikari-core` は `pub use ikari_input::*` で互換を保つ。
- 字種の変換（F6〜F10: ひらがな、全角カタカナ、半角カタカナ、全角英数、半角英数）を足す: `pub fn convert_script(s: &str, to: Script) -> String`。今は Converter の中に `to_katakana` があるだけなので、ここに集める。

### 3.4 core の変更（契約）
```rust
pub struct RankInput<'a> { /* 既存 */ pub deadline: Option<Instant> }   // 追加。None = 待たない（LM を使わない）
pub struct LmBudget { pub base: Duration /*30ms*/, pub per_char: Duration /*6ms*/, pub max: Duration /*300ms*/ }
pub struct ConvertOptions { /* 既存 */ pub lm_budget: LmBudget }
impl Converter {
    /// 既存の convert は「今 + lm_budget(読み)」を締め切りにする
    pub fn convert_until(&self, reading: &str, ctx: &Context, deadline: Option<Instant>) -> Conversion;
}
impl Conversion {
    /// 既存の page は「今 + lm_budget(読み)」を締め切りにする。1回の呼び出しの中の rank はすべて同じ締め切りを使う
    pub fn page_until(&mut self, index: usize, deadline: Option<Instant>) -> Option<Page<'_>>;
}
```
- `ZenzRanker` は `min(cfg.budget, deadline - now)` だけ待つ。締め切りを過ぎていれば要求を送らない（今の `budget` は上限として残す）。
- `ZenzRanker::warm_up()` は、ダミーの短い採点（「きょう」）を1回実行し、Metal を初期化して読み込みを済ませる。

---

## 4. Rust と Swift の境界（FFI）

### 4.1 比較

| 案 | 良い点 | 悪い点 |
| --- | --- | --- |
| **C ABI を手書きし、メッセージは protobuf のバイト列**（採用） | 境界が小さい（エンジンは5関数）。IPC と同じスキーマなので、XPC サービスは**中身を解釈せずに中継するだけ**になる。Rust 側で単体テストやファジングができる。生成ツールが要らない | ヘッダを手で書く（関数が少ないので、Rust 側のテストでシンボルと型を確かめる） |
| UniFFI 0.32 | 型付きの Swift API が自動で生成される | 生成の手順と uniffi のランタイムが増える。protobuf と二重のスキーマになる。XPC サービスに型の知識が入り、「薄い殻」でなくなる |
| cbindgen | ヘッダを自動で生成できる | 関数が10個程度なら、手書きの方がツールを増やさずに済む（YAGNI） |

### 4.2 エンジンの FFI（`engine-ffi/include/ikari_engine.h`、契約）
```c
typedef struct IkariEngine IkariEngine;
typedef struct { uint8_t *ptr; size_t len; } IkariBuf;          // Rust が確保したバイト列。ikari_buf_free で解放する

// config: EngineConfig（protobuf）。失敗なら NULL を返し、*err に ErrorReply（protobuf）を入れる
IkariEngine *ikari_engine_open(const uint8_t *config, size_t len, IkariBuf *err);
// req: Envelope（protobuf）。返り値も Envelope。決して NULL にせず、エラーも Envelope で返す。複数スレッドから同時に呼んでよい
IkariBuf ikari_engine_handle(IkariEngine *e, const uint8_t *req, size_t len);
void ikari_buf_free(IkariBuf buf);
void ikari_engine_close(IkariEngine *e);                         // 学習を保存してから解放する
uint32_t ikari_engine_schema_version(void);
```
- Rust 側はすべての入口で `catch_unwind` を使う。panic を FFI の外へ伝えず、`ErrorReply { code: INTERNAL }` にする。
- `Convert` は、締め切りまで呼び出しのスレッドを止めて待つ。XPC の受信は並行のキューで受けるので、止めてよい。

### 4.3 殻の FFI（`shell-ffi/include/ikari_shell.h`、契約）
```c
typedef struct IkariComposer IkariComposer;
typedef struct { const uint8_t *ptr; size_t len; } IkariStr;    // 借用。次にその composer を呼ぶまで有効

IkariComposer *ikari_composer_new_romaji(const uint8_t *table_tsv, size_t len /* 0 なら同梱の表 */, IkariBuf *err);
IkariComposer *ikari_composer_new_jis(void);
void   ikari_composer_push_char(IkariComposer *c, uint32_t codepoint);          // ローマ字
void   ikari_composer_push_jis(IkariComposer *c, uint8_t jis_key, bool shift);  // JisKey の番号（ヘッダに列挙する）
bool   ikari_composer_backspace(IkariComposer *c);
void   ikari_composer_flush(IkariComposer *c);
void   ikari_composer_clear(IkariComposer *c);
IkariStr ikari_composer_kana(const IkariComposer *c);
IkariStr ikari_composer_pending(const IkariComposer *c);
void   ikari_composer_free(IkariComposer *c);
IkariBuf ikari_convert_script(const uint8_t *s, size_t len, uint32_t script);  // F6〜F10
void   ikari_buf_free(IkariBuf buf);
```

---

## 5. IPC（XPC と protobuf）

### 5.1 XPC
- Swift の `@objc protocol IkariEngineXPC { func handle(_ request: Data, reply: @escaping (Data) -> Void) }`。メソッドは1つだけで、スキーマは protobuf に任せる。
- **殻**: `NSXPCConnection(serviceName: "io.github.ikari-zen.inputmethod.ikari.engine")`。`resume()` の前に `setCodeSigningRequirement` を呼び、`interruptionHandler` と `invalidationHandler` を設定する（§7.5）。
- **XPC サービス**（`IkariEngineService/main.swift`）:
  - `NSXPCListener.service()` のデリゲートの `shouldAcceptNewConnection` で、新しい接続に `setCodeSigningRequirement` を設定してから `resume` する。
  - 起動時に `ikari_engine_open` を1回だけ呼び、すべての接続で共有する。
  - `handle` は `ikari_engine_handle` を呼んで、結果を返すだけにする（**中身を解釈しない**）。
- **署名要件**（ビルド時に `SIGNING` で選び、Swift の定数として生成する）:

  | SIGNING | 殻 → エンジン | エンジン → 殻 |
  | --- | --- | --- |
  | `team`（推奨） | `identifier "io.github.ikari-zen.inputmethod.ikari.engine" and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"` | `(identifier "io.github.ikari-zen.inputmethod.ikari" or identifier "io.github.ikari-zen.inputmethod.ikari.smoke") and anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"` |
  | `adhoc`（NFR-42 どおり） | `identifier "…engine"` | `identifier "…ikari" or identifier "…smoke"` |

  - **ad-hoc でできること**: identifier の一致しか見られない。identifier は誰でも名乗れるので、署名の検証は形だけになる。cdhash に固定するとビルドのたびに変わり、運用できない。
  - **同梱の XPC サービスなので、ほかのプロセスはそもそも名前を引けない**。どちらの方式でも、「エンジンになりすましてキー入力を受け取る」「外から接続して学習データを引き出す」は、launchd の名前空間で防げる。署名の検証は多層防御として足す。
  - Apple Development の証明書を使えば、`anchor apple generic` とチームID で固定でき、spec の「互いのコード署名を検証する」を実質的に満たせる（§12-1 の判断事項）。

### 5.2 protobuf のスキーマ（`proto/ikari/engine/v1/engine.proto`、契約）
```proto
syntax = "proto3";
package ikari.engine.v1;

message Envelope {
  uint32 schema_version = 1;     // = 1。不一致なら ErrorReply{code: SCHEMA_MISMATCH}
  uint64 request_id = 2;         // 殻が付ける。返事にそのまま入れる
  oneof body {
    Hello hello = 10;               HelloReply hello_reply = 11;
    BeginSession begin_session = 12; Ack ack = 13;
    EndSession end_session = 14;
    Prefetch prefetch = 15;          // 返事は Ack（すぐ返す）
    Convert convert = 16;            ConvertReply convert_reply = 17;
    PageRequest page = 18;           ConvertReply page_reply = 19;
    Commit commit = 20;              // 返事は Ack
    CommitText commit_text = 21;     // 変換せずに確定した文字列（確定履歴のため）。返事は Ack
    Cancel cancel = 22;
    Shutdown shutdown = 23;          // 版の不一致のときに殻が送る。エンジンは保存して exit
    ErrorReply error = 30;
  }
}
message EngineConfig { string data_dir = 1; bool enable_lm = 2; uint32 idle_unload_secs = 3; }  // FFI の open に渡す
message Hello { string shell_version = 1; }
message HelloReply { string engine_version = 1; string dict_label = 2; LmState lm = 3; }
enum LmState { LM_STATE_UNSPECIFIED = 0; LM_DISABLED = 1; LM_LOADING = 2; LM_READY = 3; LM_FAILED = 4; }
message BeginSession { uint64 session_id = 1; string client_bundle_id = 2; bool secure = 3; }
message EndSession { uint64 session_id = 1; }
message SurroundingText { string left = 1; string right = 2; bool available = 3; }   // 末尾 2000 文字に切る（殻）
message Prefetch { uint64 session_id = 1; string reading = 2; SurroundingText context = 3; uint64 context_rev = 4; }
message Convert  { uint64 session_id = 1; string reading = 2; SurroundingText context = 3; uint64 context_rev = 4; uint32 page_size = 5; }
message PageRequest { uint64 conversion_id = 1; uint32 index = 2; }
message Candidate { string surface = 1; string annotation = 2; CandidateKind kind = 3; }
enum CandidateKind { CANDIDATE_KIND_UNSPECIFIED = 0; DICTIONARY = 1; HIRAGANA = 2; KATAKANA = 3; REWRITER = 4; }
message ConvertReply { uint64 conversion_id = 1; uint32 page_index = 2; repeated Candidate candidates = 3; bool has_next = 4;
                       bool lm_applied = 5; uint32 engine_micros = 6; }
message Commit { uint64 conversion_id = 1; uint32 candidate_index = 2; }   // 生成済みの列の中の位置（ページをまたいだ通し番号）
message CommitText { uint64 session_id = 1; string reading = 2; string surface = 3; }
message Cancel { uint64 conversion_id = 1; }
message Ack {}
message ErrorReply { ErrorCode code = 1; string message = 2; }   // message にユーザーの文字列を入れない
enum ErrorCode { ERROR_CODE_UNSPECIFIED = 0; SCHEMA_MISMATCH = 1; BAD_REQUEST = 2; UNKNOWN_SESSION = 3; UNKNOWN_CONVERSION = 4;
                 ENGINE_NOT_READY = 5; INTERNAL = 6; }
```
- **信用しない入力として扱う**（spec）: Rust 側では、未知のフィールドを無視し、文字列の長さに上限を設ける（読みは256文字、文脈は2000文字、page_size は1..9）。Swift 側も、返ってきた候補の数と長さを検証する。Rust の decode は、ADR 0001 以来のバイト反転と切り詰めのテストで網羅する。
- **版の扱い**: 殻とエンジンは同じバンドルで出すが、入れ替えた直後に古いエンジンのプロセスが残ることがある。殻は `Hello` で版を照合し、合わなければ `Shutdown` を送って再接続する。
- **生成**: Rust は `ikari-proto/build.rs` で `protox` を使い、生成したコードはコミットしない。Swift は `macos/scripts/gen-proto.sh` が protoc と protoc-gen-swift で `macos/Sources/IkariProto/engine.pb.swift` を生成し、**これをコミットする**（通常のビルドに protoc を要らなくするため）。スキーマを変えたときは、このスクリプトを流して差分をコミットする。

---

## 6. 殻（InputMethodKit、Swift）

### 6.1 Info.plist の要点
- `InputMethodConnectionName` = `io.github.ikari-zen.inputmethod.ikari_Connection`。`InputMethodServerControllerClass` = `IkariInputController`（Swift のクラスに `@objc(IkariInputController)` を付けて名前を固定する）。
- `LSBackgroundOnly` = YES。`LSMinimumSystemVersion` = 26.0（NFR-40: 直近2メジャー）。`TISIntendedLanguage` = ja。
- `tsInputMethodCharacterRepertoireKey` = Hira、Kana、Latn。
- `ComponentInputModeDict` に入力モードを2つ置く。`.Japanese`（「あ」）と `.Roman`（「A」）で、それぞれにアイコンを付ける。
- **Bundle ID に `.inputmethod.` を含める**。Sandbox に入れた IME はこれが必須で、殻は Sandbox に入れないが、規約に合わせておく。

### 6.2 モジュール（SwiftPM、`macos/`）

| ターゲット | 種類 | 中身 |
| --- | --- | --- |
| `IkariShellCore` | library（**AppKit と IMK に依存しない純 Swift**） | 入力の状態機械（§6.3）、キー割り当て（プリセットと上書き）、候補ウィンドウの配置の計算（§6.4）、`EngineClient` プロトコル、エンジンなしのときのフォールバック、設定（TSV）の読み込み |
| `IkariShellFFI` | binaryTarget（`Frameworks/IkariShellFFI.xcframework`） | `libikari_shell.a` と `ikari_shell.h` |
| `IkariEngineFFI` | binaryTarget（`Frameworks/IkariEngineFFI.xcframework`） | `libikari_engine.a` と `ikari_engine.h` |
| `IkariProto` | library | 生成した protobuf（SwiftProtobuf 1.38.x に依存） |
| `IkariXPC` | library | `EngineClient` の XPC の実装（接続、署名要件、再接続、タイムアウト）と、署名要件の定数（ビルド時に生成） |
| `IkariIME` | executable | `IMKServer` の起動、`IkariInputController`（IMK のイベントを状態機械に渡すだけ）、`CandidatePanel`（NSPanel）、周辺テキストとキャレットの矩形の取得 |
| `IkariEngineService` | executable | XPC サービス（§5.1。FFI を呼ぶだけ） |
| `IkariSmoke` | executable | XPC 経由の E2E（§10.2） |
| テスト | `IkariShellCoreTests`、`IkariXPCTests`（モック）、`IkariFFITests`（FFI の往復） | `swift test` |

### 6.3 入力の状態機械（`IkariShellCore`）
- **状態**: `idle` / `composing(kana, pending)` / `converting(conversion, page, selected)` / `asciiRun`（Shift＋英字の一時英数）。
- **キーと操作**（spec の UI と便利な変換の節）:

  | キー | composing | converting |
  | --- | --- | --- |
  | 文字 | 合成器へ（ローマ字、または JIS の keyCode）。読みが変わったら `Prefetch` を投げる | 選択中の候補を確定し、その文字から新しい合成を始める |
  | Space | `Convert`（読み = かなの部分。pending は含めない。FR-07） | 次の候補へ。ページの最後の候補なら次のページの先頭へ |
  | Shift+Space | （何もしない） | 前の候補へ。ページの先頭なら前のページの最後へ |
  | 1〜9 | （文字として扱う） | その番号の候補を確定する |
  | Enter | かなをそのまま確定し、`CommitText` を送る | 選択中の候補を確定し、`Commit` を送る |
  | Esc | 合成を取り消す | **読みに戻る**（composing へ） |
  | BackSpace | 1文字消す（pending から先に消す） | **読みに戻る** |
  | F6〜F10、プリセットのキー | 読みを字種変換して converting に入る（候補は1つ） | 同じ |
  | Shift＋英字 | `asciiRun` に入り、英字を ASCII のまま読みに積む（「API」「Kubernetes」）。Space、Enter、Esc で抜ける | 確定してから asciiRun を始める |
  | z＋キー | ローマ字表の行として合成器が処理する（変更なし） | — |

- **最後のページで Space を押したとき**（spec の未決事項）は、**先頭のページに戻る**案を既定にする（§12-3）。`settings.tsv` の `paging.wrap` で切り替えられるようにする。
- **キー割り当て**: プリセットは `macos/Resources/keymap/{ms-ime,macos}.tsv`（spec の表のとおり）。`settings.tsv` の `keymap.preset` で選び、`keymap.<操作>=<キー>` で個別に上書きする。
- **入力方式**: `settings.tsv` の `input.method=romaji|kana`。かな入力は、物理キーの keyCode を `JisKey` の番号に対応付ける（キーボードの配列には依存しない）。

### 6.4 候補ウィンドウ（`CandidatePanel`、spec の配置と候補リストの節）
- 枠のない、アクティブにならない `NSPanel` にする。ウィンドウレベルは `client.windowLevel() + 1`。
- 2行固定。1行目は候補で、`1 品詞 │ 2 瀕死 │ …`（番号と区切り線、選択中は背景色）。2行目は選択中の候補の注釈で、空でも高さを保つ。
- **アンカー**: 変換文字列の先頭の矩形（`client.firstRect(forCharacterRange: NSRange(location: markedStart, length: 0), actualRange:)`）。打鍵やページ送りでは横に動かさない。
- **配置の計算**（純関数 `placePanel(anchor:panelSize:visibleFrame:) -> NSRect` を `IkariShellCore` に置いてテストする）:
  - 下に置く。下端に入らないときだけ上に置く。右端からはみ出すなら左にずらす。
  - 基準は、キャレットを含むスクリーンの `visibleFrame`（Dock とメニューバーを除いた領域）。
- **キャレットが取れないとき**（矩形がゼロ、または NaN）:
  1. アクセシビリティ API で、フォーカスのある要素の左下を使う（**権限があるときだけ**。自動で権限を求めない。NFR-20）。
  2. 同じアプリで最後に取れた正常な値を使う。
  3. どちらもなければ、マウスのあるスクリーンの中央下に出す。

### 6.5 文脈と secure
- **周辺テキスト**: `Convert` の直前と、セッションの開始時に、`client.selectedRange()` の手前を最大2000文字、`attributedSubstring(from:)` で取る。取れなければ `available=false`（ターミナル、一部の Electron）。
  - `Prefetch` には直近に取った値を使い、打鍵のたびには取りに行かない（クライアントへの問い合わせを打鍵ごとにしないため）。
- **確定履歴**は、エンジンがセッションごとに持つ（`Commit` と `CommitText` で積む）。周辺テキストが取れないときは、エンジンが履歴を左文脈にする。
- **secure**: 入力モードの有効化と打鍵のたびに `IsSecureEventInputEnabled()` を見る。真なら、`BeginSession{secure: true}` を送り、文脈を送らず、`Commit` も送らない。
  - エンジン側も、secure のセッションでは解析、学習、LM を使わない（既存の Converter の処理）。
  - macOS は、パスワード欄では普通は日本語の IME を無効にするが、それに頼らずに二重で守る（NFR-18）。
- **セッション**: `IkariInputController` のインスタンスごとに `session_id` を振る。`client.bundleIdentifier()` を `client_bundle_id` として送る（アプリごとのプロファイルは将来の候補で、フェーズ2ではログにも残さない）。

### 6.6 アイコン
- spec の「錨の軸を I ビームに見立てる」案で作る。入力モードは、リングの中の「あ」「A」で示す。
- 16pt の単色テンプレート画像を PDF で用意する。`macos/tools/make-icons.swift`（CoreGraphics で描くスクリプト）で生成し、PDF をコミットする。ライトモードとダークモードの両方で、メニューバーで見分けられることを手動で確かめる。

---

## 7. エンジン（`ikari-engine`）

### 7.1 構成
```rust
pub struct Engine { stack: Stack, sessions: Mutex<HashMap<u64, Session>>, cache: ConversionCache, prefetcher: Prefetcher, data: DataWatcher }
impl Engine {
    pub fn open(cfg: &EngineConfig) -> Result<Engine, EngineError>;   // 辞書、vectors、学習、LM を読み込み、LM の warm_up を別スレッドで始める
    pub fn handle(&self, req: &[u8]) -> Vec<u8>;                     // Envelope → Envelope
}
```

### 7.2 先行計算とキャッシュ（NFR-03、NFR-05、ADR 0003 §6-2〜4）
1. `Prefetch` は、キー `(session_id, 読み, context_rev)` を作り、セッションごとに**最新の1件だけ**を待ち行列に置く（古いものは捨てる）。すぐ `Ack` を返す。
2. 先行計算のスレッドは、`convert_until(読み, 文脈, now + lm_budget.max)` と `page_until(0, …)` を実行し、`Arc<Mutex<Conversion>>` をキャッシュに入れる。状態は `Ready`。
3. `Convert` は同じキーを探す。
   - `Ready` なら、そのまま1ページ目を返す（エンジン内で 1ms 未満の見込み）。
   - 計算中なら、`開始時刻 + lm_budget(読み)` まで待つ。
   - 間に合わない、またはキャッシュにないときは、**LM を使わない締め切り**（`deadline = Some(now)`）でその場で変換して返す。`lm_applied=false`。
4. 返した `Conversion` には `conversion_id` を付けて固定する。後から LM の結果が届いても、**表示済みのページは差し替えない**（NFR-05。既存の Conversion の性質のまま）。次のページの生成には、そのときまでに届いた LM の結果を使ってよい。
5. キャッシュは、セッションごとに最新の8件まで、全体で64件まで。`EndSession` と、60秒使われなかったものを捨てる。
6. LM のワーカーは1つで、最新の要求だけを処理する（既存の仕組み）。打鍵が速いと途中の Prefetch は捨てられ、最後の読みだけが計算される。

### 7.3 読み込み直後と、アイドル時の解放（ADR 0003 §6-4、NFR-11）
- エンジンの起動時に、LM を別スレッドで読み込み、`warm_up()` を1回実行する。それまでの `Convert` は LM なしで返す（`HelloReply.lm = LM_LOADING`）。
- アイドル時の解放（既定は300秒）のあとは、**最初の `Prefetch` で読み直しと warm_up を始める**。打鍵を始めてからスペースを押すまでの間に、たいてい読み込みが終わる。
- 殻は入力モードが有効になったとき（`activateServer`）に `Hello` を送り、エンジンの起動を前倒しする。

### 7.4 ユーザーデータ
- **学習**: エンジンが書き手になる。`Commit` のたびにメモリに記録し、最後の保存から5秒たつか、`EndSession` のときに保存する（tmp に書いて rename）。
- **ユーザー辞書とプロファイル**: CLI で編集できる。エンジンは `Convert` のときに mtime を見て（1秒に1回まで）、変わっていれば読み直す。
- **`engine.lock`**: エンジンが flock で持つ。CLI の `learning.tsv` を書き換えるコマンド（`--commit`、`learn reset`、`data import`）は、ロックが取られていればエラーにし、「エンジンを止めてから実行する」ように案内する（`--force` で強制）。ユーザー辞書とプロファイルの編集はロックの対象外（エンジンは読むだけなので）。

### 7.5 落ちたときと再起動（NFR-16）
- **殻**:
  - `interruptionHandler`（エンジンが落ちた）では、状態を「エンジンなし」にする。次の要求で launchd がエンジンを起動し直すので、そのときに `Hello` と `BeginSession` を送り直す。
  - `invalidationHandler` では、接続を作り直す。
  - **エンジンなしのときのフォールバック**: Space を押すと、殻だけで「ひらがな、カタカナ、半角カタカナ」の候補を出す。入力は止めない。
- **タイムアウト**: 殻は `Convert` と `Page` を、`lm_budget(読み) + 40ms` まで同期で待つ（IMK の主スレッドを止める）。超えたらフォールバックの候補を出し、その返事は捨てる。`Prefetch` と `Commit` は返事を待たない。
- **連続して落ちるとき**: 60秒に3回落ちたら、5分間はエンジンを呼ばない（壊れた辞書で再起動を繰り返さないため）。

### 7.6 ログ
- `logs/engine.log` に、時刻、要求の種類、所要時間、LM の状態、エラーの種類だけを書く。**読み、候補、文脈の文字列は書かない**（NFR-19）。1MB で回し、3世代まで残す。
- （追記 2026-10-25）回し方と保持は [ADR 0034](0034-diagnostic-log.md) §3.2 で改めた（4MB か日付が変わったときに回し、gzip にして 180 日・合計 100MB まで残す。書き込みは別スレッド）。

---

## 8. データの置き場所と Sandbox

### 8.1 配置
§2 のとおり。`UserDir` の既定（ADR 0002 §21.1-8）に、`dict/`、`models/`、`settings.tsv`、`engine.lock`、`logs/` を足す。CLI と eval の `--dict` の既定値も、`$IKARI_DICT_DIR` がなければ `<UserDir>/dict` を探すようにする（今の `./data/build/dict` はその次）。

### 8.2 Sandbox の例外
エンジンの entitlement:
```
com.apple.security.app-sandbox = true
com.apple.security.temporary-exception.files.home-relative-path.read-write = ["/Library/Application Support/ikari-ime/"]
```
- **判断: Sandbox のコンテナ（`~/Library/Containers/…`）ではなく、一時例外で共有のディレクトリを許す。** CLI、エンジン、殻が同じファイルを見る必要があり（ADR 0002 の決定）、移行も不要になる。一時例外は App Store の審査では通らないが、配布は自分用のローカルビルドなので問題にならない（NFR-41）。
- App Group（`<TEAM>.io.github.ikari-zen.ikari`）も検討したが、macOS の新しい版でグループコンテナにプロビジョニングが要るかどうかが不確かで、ad-hoc 署名では使えない。却下した。

### 8.3 辞書とモデルの入れ方
- `scripts/install-data.sh` で、`data/build/dict/*` と `data/raw/zenz/zenz-v3.2-small-Q5_K_M.gguf` を `~/Library/Application Support/ikari-ime/{dict,models}/` へコピーする。コピーは tmp を経て rename し、エンジンが mmap している最中に書き換えない（ADR 0001 の方針）。
- バンドルの中には入れない。約340MB あり、辞書は将来の自動更新で入れ替えるため。

---

## 9. ビルド、署名、インストール

### 9.1 判断: SwiftPM + スクリプト（Xcode プロジェクトは作らない）

| 案 | 判断 |
| --- | --- |
| **SwiftPM + シェルスクリプト**（採用） | Swift のコードとテストは SwiftPM（`swift build`、`swift test`）で扱う。.app と .xpc のバンドルの組み立て、署名、インストールはスクリプトで行う。設定がテキストなので差分を読め、エージェントでも扱える |
| Xcode プロジェクト（.xcodeproj） | IME のバンドルは作りやすいが、pbxproj は差分が読みにくく、並行作業で衝突する |
| XcodeGen | project.yml から生成できるが、ツールが1つ増える。インストール、killall、入力ソースの再選択はどのみちスクリプトになる |

### 9.2 `macos/scripts/build.sh`（`--signing team|adhoc`、`--release|--debug`）
1. `cargo build --release --target aarch64-apple-darwin -p ikari-shell-ffi` と、`CARGO_TARGET_DIR=target/lm cargo build --release -p ikari-engine-ffi --features lm-metal`。
2. `xcodebuild -create-xcframework -library <lib>.a -headers <include>` で、`macos/Frameworks/{IkariShellFFI,IkariEngineFFI}.xcframework` を作る（gitignore する）。
3. `swift build -c release --package-path macos`。リンクの設定（`linkerSettings`）に Foundation、Metal、MetalKit、Accelerate、c++ を足す（`cargo rustc -- --print native-static-libs` の出力と一致させ、ずれたらスクリプトで検出する）。
4. バンドルを組み立てる。Info.plist はテンプレートから版を埋めて作る。
5. 署名は内側から行う。`codesign --force --options runtime --entitlements engine.entitlements -s "$ID" IkariEngine.xpc`、次に ikari-smoke、最後に ikari.app。最後に `codesign --verify --deep --strict` で確かめる。
6. `macos/scripts/install.sh`: `~/Library/Input Methods/ikari.app` へコピーし、`killall ikari IkariEngine`（存在しなくても失敗しない）を実行し、`macos/tools/select-input-source.swift`（TIS の API で入力ソースを有効にし、再選択する）を実行する。

### 9.3 必要なツール

| ツール | 用途 | 状態 |
| --- | --- | --- |
| Xcode 27.0 / Swift 6.4 | SwiftPM、codesign、xcodebuild（XCFramework） | あり |
| cmake 4.4.4 | llama.cpp | あり |
| protobuf 36.x と swift-protobuf 1.38.x（Homebrew） | Swift の生成コードの再生成（スキーマを変えるときだけ） | **ない（`brew install protobuf swift-protobuf`）** |
| SwiftProtobuf（SwiftPM の依存、1.38.x に固定） | Swift の実行時ライブラリ | 初回の `swift build` で GitHub から取得する（ビルドのときだけネットワークを使う。IME 自体は通信しない） |

---

## 10. テスト方針

### 10.1 自動テスト

| 層 | 方法 | 主な内容 |
| --- | --- | --- |
| core と lm | `cargo test`（既存） | GrammarGuard、`RankInput.deadline`（締め切りを過ぎたら LM を呼ばない。1回の page の中の rank は同じ締め切りを使う）、`lm_budget` の計算、warm_up、時間切れ率のテストの不足分（ADR 0003 §6-8） |
| ikari-stack | `cargo test` + eval | 移す前後で ext test の `--out-cases` が一致する |
| ikari-engine | `cargo test`（fixture の辞書と、モックの LM で） | protobuf の往復、壊れたバイト列（バイト反転、切り詰め、巨大な長さ）でパニックしない、Prefetch → Convert でキャッシュに当たる、計算中なら締め切りまで待つ、間に合わなければ LM なしで返す、表示したページを差し替えない、secure、セッションの破棄、ロック、mtime による読み直し |
| ikari-engine（性能） | `cargo test --release -- --ignored`（実辞書と実モデル） | 打鍵の間隔 120ms で読みを打ち、最後にスペースを押す、というシミュレーションで、`Convert` のエンジン内の時間が p99 1ms 以下（キャッシュに当たった場合）。読み込み直後でも 30ms を超えない（締め切りを守る） |
| FFI | Rust のテストから `extern "C"` の関数を呼ぶ。Swift の `IkariFFITests` から両方の FFI を呼ぶ | NULL、空、不正なバイト列、二重の free をしない使い方、panic が FFI の外に出ない |
| 殻のロジック | `swift test`（`IkariShellCoreTests`） | 状態機械（§6.3 の表を全部）、キー割り当てとプリセット、配置の計算（下、上への反転、左へのずれ、複数のディスプレイ、矩形が取れないときのフォールバック）、エンジンなしのフォールバック、タイムアウトのときの扱い（モックの EngineClient で） |
| XPC の E2E | `ikari.app/Contents/MacOS/ikari-smoke`（バンドルの中に置き、同梱の XPC サービスにつなぐ） | Hello → BeginSession → Prefetch → Convert（「ユーザー辞書に単語を登録するときは」+ ひんし → 1位が品詞）→ Commit。`kill -9 IkariEngine` のあと1秒以内に再接続して変換できる。版の不一致で Shutdown が働く。XPC 経由の往復を含めた Convert の時間を出す |
| 署名と entitlement | build.sh の最後 | `codesign --verify --deep --strict`。`codesign -d --entitlements -` で、エンジンに app-sandbox があり network がないこと |
| 評価 | `ikari-eval`（ADR 0003 の下限） | ext test の話題 kind が LM ありで 72% 以上、LM なしで 54% 以上。否定と文法は全構成で 100%。hiragana.tsv と homophones.tsv で回帰がない |

- **未確認の前提**: バンドルの `Contents/MacOS` に置いた補助の実行ファイル（ikari-smoke）から、同梱の XPC サービスにつながること。最初に確かめ、だめなら、smoke を殻の起動引数のモード（`ikari --smoke`）にする。

### 10.2 手動のチェックリスト（実機。フェーズ2の完了条件に含める）
1. インストールして入力ソースに追加でき、メニューバーにアイコンが出る（ライトとダーク）。モードの「あ」と「A」を切り替えられる。
2. **アプリごとの確認**: テキストエディット、メモ、Safari のテキスト欄、VS Code（Electron）、ターミナル（文脈が取れない）、パスワード欄（日本語の入力にならない、または secure として扱われる）。
3. **入力**: ローマ字（ん、っ、z＋キー）、かな入力（JIS キーボードがあれば）、Shift＋英字（「API」）。
4. **変換と確定**: Space で変換する。番号で選ぶ。ページを送る（最後のページの挙動、Shift+Space）。Esc と BackSpace で読みに戻る。Enter で確定する。F6〜F10 と、2つのプリセット。
5. **候補ウィンドウの配置**: 下に出る。画面の下端で上に出る。右端で左にずれる。ページを送ってもアンカーが動かない。2行目の高さが一定。外部ディスプレイでも正しい。
6. **文脈**: テキストエディットで「ユーザー辞書に単語を登録するときは」と書いてから ひんし → 品詞。ターミナルで、確定履歴だけで同じ結果になる。
7. **学習**: 同じ文脈で別の候補を確定したあと、次から上位に来る。パスワード欄の後で learning.tsv が変わっていない。
8. **体感の速さ**: ふつうに打ってスペースを押したとき、候補が遅れて見えない（ログの Convert の所要時間で裏を取る）。
9. **落ちたとき**: `killall -9 IkariEngine` のあとも入力が止まらず、次の変換で復帰する。
10. **アイドル**: 5分置いてから打っても、最初の変換が遅れない（ログで LM の状態を確かめる）。
11. **メモリ**: アクティビティモニタで、殻が 50MiB 以下、エンジンが 500MiB 以下。
12. **再インストール**: build.sh → install.sh のあと、入力ソースを選び直さなくても新しい版になる（版の照合が働く）。

---

## 11. 並行実装のレーン分割

### 11.1 契約コミット（リードが入れる）
1. workspace に crate を足す: `input`（ikari-input。core::input を**この時点で移す**。中身の機械的な移動）、`stack`（中身は todo!）、`proto`（ikari-proto。スキーマと build.rs まで完成させる）、`engine`、`engine-ffi`、`shell-ffi`。依存を足し、Cargo.lock を確定させる（prost 0.14、prost-build 0.14、protox 0.10）。
2. `proto/ikari/engine/v1/engine.proto`（§5.2 の全文）。`engine-ffi/include/ikari_engine.h` と `shell-ffi/include/ikari_shell.h`（§4）。
3. core の契約の変更（§3.4 の型とシグネチャ。`RankInput.deadline` を足し、既存の呼び出し元を `None` などで直す）。
4. `macos/Package.swift`（ターゲットの宣言。SwiftProtobuf 1.38.x に固定）と、`IkariShellCore` の公開の型（`EngineClient` プロトコル、`InputState`、`KeyAction`、`PanelPlacement` の入出力）。生成済みの `engine.pb.swift`。
5. Bundle ID、署名要件の文字列の型、`settings.tsv` のキー一覧を `docs/` に置く（§12 の決定を反映する）。

### 11.2 レーン

| レーン | 持つファイル | 最初に置くもの | 完了条件 |
| --- | --- | --- | --- |
| **Q: 変換の質と締め切り（Rust）** | `core/src/{converter,lattice,nbest}.rs`、`core/src/ranker/**`（GrammarGuard を含む）、`lm/**`、`eval/src/{lib,gates,…}.rs`（setup.rs 以外）、`eval/sets/**` | GrammarGuard の受け入れテスト（ADR 0003 §3.2） | GrammarGuard（ext-neg-10 が R3 で1位、否定・文法が 100%、話題の低下が 0.5 ポイント以内）。締め切りが変換1回あたりになる。lm_budget。warm_up。時間切れ率のテストの不足分と、eval の欄の名前。LM ありの a-02 シナリオ。否定・文法の test を各30件以上。EOS の実験レポート（採否は問わない） |
| **E: エンジン（Rust）** | `stack/**`、`engine/**`、`engine-ffi/**`、`shell-ffi/**`、`proto/**`、`cli/src/stack.rs` と `eval/src/setup.rs`（ikari-stack への置き換え）、`cli` の `--dict` の既定値とロック | `Engine::handle` を、fixture とモックの LM で通す | §10.1 の ikari-engine、FFI、性能の各行。ikari-stack への移行で eval の結果が一致する |
| **X: XPC、ビルド、配布** | `macos/Sources/{IkariXPC,IkariEngineService,IkariSmoke}/**`、`macos/scripts/**`、`macos/tools/**`、entitlements、Info.plist のテンプレート、`scripts/install-data.sh`、THIRD_PARTY_NOTICES（llama.cpp を追加）、`macos/README.md` | 契約の FFI のスタブ（`ikari_engine_handle` が ENGINE_NOT_READY を返す）でバンドルを組み、署名して起動する | build.sh と install.sh が、team と adhoc の両方で通る。`codesign --verify` が通り、entitlement を確かめられる。ikari-smoke の E2E（E のマージ後）。kill -9 からの復帰 |
| **S: 殻（Swift）** | `macos/Sources/{IkariShellCore,IkariIME}/**`、`macos/Tests/**`、`macos/Resources/{keymap,icons}/**`、`macos/tools/make-icons.swift` | モックの `EngineClient` と、FFI の composer（X の xcframework ができるまでは Swift のテスト用の替え玉） | §10.1 の殻のロジックのテストがすべて通る。§10.2 の手動チェックリストの 1〜5 を、モックではなく実エンジンで通す（X と E のマージ後） |

### 11.3 マージの順
1. **Q と E を先に**（Rust 側。Q は core の中身、E は新しい crate と cli/eval の setup の置き換えで、ファイルが重ならない）。E の先行計算は、Q の `convert_until` と `page_until` に依存するが、契約のシグネチャで先に書ける。
2. **X**（E の FFI を実物にして E2E を通す）。
3. **S**（実エンジンで手動チェックリストを通す）。
4. リードが §10.2 のチェックリストをすべて流し、結果を `docs/phase2-checklist.md` に記録する。

---

## 12. フェーズ2の完了条件
- `cargo test --workspace` と、`cargo test -p ikari-engine-ffi -p ikari-lm --features lm-metal` が通る。`swift test --package-path macos` が通る。
- `macos/scripts/build.sh --signing <決定した方式>` と `install.sh` が通り、`codesign --verify --deep --strict` が通る。エンジンの entitlement に app-sandbox があり、network がない。
- ikari-smoke の E2E が通る（文脈つきの変換、確定、kill -9 からの1秒以内の復帰、版の不一致の処理）。
- **性能**: 打鍵のシミュレーション（120ms 間隔）で、Space から1ページ目が返るまでの時間が、XPC の往復を含めて**短い読み（10文字以下）で p99 16ms 以下**（NFR-02）。読み込み直後でも、締め切り（30ms + IPC）を超えない。
- ADR 0003 の評価の下限を満たす（LM ありの話題 72% 以上、LM なし 54% 以上、否定と文法は全構成で 100%）。
- §10.2 の手動チェックリストの 1〜12 がすべて OK（記録を残す）。
- フェーズ2に含めるが完了の条件にしないもの: EOS の実験の採否、`時が経つ`・`ココ`・数字の書式・助詞で始まる分割の改善（レーン Q で取り組み、結果を報告する）。

---

## 13. 却下した案

| 案 | 却下した理由 |
| --- | --- |
| エンジンを Mach サービス（LaunchAgent）として登録する | 名前を自分で守る必要があり、他のプロセスからも引ける。同梱の XPC サービスなら launchd が名前空間を分けてくれる |
| UniFFI で型付きの FFI を作る | protobuf と二重のスキーマになる。XPC サービスが薄い殻でなくなる（§4.1） |
| ローマ字の合成をエンジンで行う | 打鍵ごとに IPC が走る。エンジンが落ちると入力も止まり、NFR-16 に反する |
| ローマ字の合成を Swift で書き直す | Rust の実装（テスト済み、ユーザーのローマ字表に対応）と二重になり、ずれる |
| `IMKCandidates` を使う | 横一列、番号、2行固定、spec の配置の規則を満たせない |
| 殻も Sandbox に入れる | アクセシビリティ API のフォールバックが使えない。殻は通信しないので、得るものが小さい |
| 辞書とモデルをバンドルに入れる | 340MB あり、辞書の自動更新で入れ替えられない |
| Xcode プロジェクト / XcodeGen | §9.1 |
| Swift の protobuf のコードを毎回生成する | 通常のビルドに protoc を要求することになる。スキーマはめったに変わらない |

---

## 14. ユーザーの操作が必要なこと
1. `brew install protobuf swift-protobuf`（スキーマを変えて Swift のコードを再生成するときだけ必要。契約コミットで1回）。
2. **署名の方式を選ぶ**（§15-1）。`team` を選ぶなら、既存の `Apple Development` の証明書を使う（期限 2027-05-23。切れたら Xcode で更新する）。
3. `scripts/install-data.sh` で、辞書（約260MB）とモデル（74MB）を `~/Library/Application Support/ikari-ime/` にコピーする。
4. 初回だけ: システム設定 → キーボード → 入力ソース → ＋ → 日本語 → ikari を追加する。表示されないときは、一度ログアウトしてログインし直す。
5. （任意）キャレットの位置が取れないアプリ向けに、システム設定 → プライバシーとセキュリティ → アクセシビリティで ikari を許可する。
6. §10.2 の手動チェックリストを実機で流す（リードと一緒に）。

---

## 15. 判断が必要な点
1. **署名の方式**: spec の NFR-42 は ad-hoc。ad-hoc では、署名の検証が identifier の一致だけになり、形だけになる（同梱の XPC サービスなので、実害は launchd の名前空間で防げる）。**推奨は、既存の Apple Development 証明書でチームIDに固定すること**（無料で、年に1回の更新が要る）。NFR-42 を「Apple Development 証明書で署名する」に改めてよいか。
2. **Bundle ID と表示名**（spec の未決事項「Ikari／錨／ikari-ime」）。案は、Bundle ID `io.github.ikari-zen.inputmethod.ikari`、入力ソースの表示名「ikari」。
3. **最後のページで Space を押したとき**（spec の未決事項）。案は、先頭のページに戻り、前のページへは Shift+Space。設定で「止まる」にもできる。
4. **既定のキー割り当てのプリセット**（MS-IME 風か macOS 風か）。案は macOS 風。
5. **注釈**: フェーズ2では2行目の枠だけを作り、データ（spec の未決事項）はフェーズ3で決める案でよいか。
6. **アクセシビリティ API のフォールバック**: 権限があるときだけ使い、自分から権限を求めない案（NFR-20）でよいか。
7. **データの置き場所と Sandbox の一時例外**（§8.2）。エンジンに `~/Library/Application Support/ikari-ime/` の読み書きを許すこと。
8. **CLI とエンジンが同時に学習ファイルを書くことの禁止**（§7.4 のロック）。エンジンが動いている間は、CLI の学習の書き込みをエラーにする。
9. **対応する macOS**: NFR-40 は直近2メジャー（26 と 27）だが、手元の実機は 27 だけ。26 は `LSMinimumSystemVersion` で許すが、実機での確認はしない、でよいか。

### 15.1 決定（2026-10-11）
| # | 決定 | 決めた人 |
| --- | --- | --- |
| 1 | Apple Development 証明書で署名し、チームID（`lib.sh` の `TEAM_ID`）（証明書の OU。値は `macos/scripts/lib.sh` の `TEAM_ID`）に固定して検証する。NFR-42 を「Apple Development 証明書で署名する（無料、年1回更新）」に読み替える。`brew install protobuf swift-protobuf` も実行する | ユーザー |
| 2 | Bundle ID は `io.github.ikari-zen.inputmethod.ikari`、表示名は「ikari」 | ユーザー |
| 3 | 最後のページで Space を押すと先頭のページに戻る。前のページへは Shift+Space。設定で「止まる」にもできる | ユーザー |
| 4 | 既定のキー割り当ては macOS 風のプリセット | ユーザー |
| 5 | 注釈は、フェーズ2では2行目の枠だけを作る。データはフェーズ3 | リード（案どおり） |
| 6 | アクセシビリティ API は、権限があるときだけ使う。自分から権限は求めない（NFR-20） | リード（案どおり） |
| 7 | エンジンの Sandbox に、`~/Library/Application Support/ikari-ime/` の読み書きだけを一時例外で許す | リード（案どおり） |
| 8 | エンジンが動いている間は、CLI からの学習ファイルの書き込みをエラーにする（flock） | リード（案どおり） |
| 9 | 対応 OS は macOS 26 と 27。実機での確認は 27 だけ | リード（案どおり） |
