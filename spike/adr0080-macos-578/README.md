# スパイク #578（macOS の分）: ADR 0080 の C1（束の中のデータを Sandbox の中から読む）と C2（750MB を含む束の署名）

spike。マージしない。捨てるコードと結果の記録（2026-11-22、macOS 27.2、arm64、内蔵 SSD。Windows の C4 は別に行う）。

## データ
- 辞書: `data/build/dict/`（2026-11-10 のビルド。`system.dict` 242.3MB、`grammar.bin` 71.7MB、`vectors.bin` 336.7MB、`postal.tsv`、`emoji.tsv`、`BUILD_INFO`）
- モデル: `zenz-v3.2-small-Q5_K_M.gguf` 73.9MB（利用者のデータの場所の `models/` から clone で写しただけ。元は書き換えていない）
- 束の中の `data/` は 714MB（du）、束の全体は 717MB（試験の束）・765MB（ikari.app に足した束）。ADR §1.1 の 748.8MB は 10 進の MB の合計で、同じもの。

## 中身
| ファイル | 役目 |
| --- | --- |
| `xpc-probe.c` | XPC サービス（launchd が起こす。App Sandbox、network なし、一時例外は存在しない試験用のパス）。自分の束の `Contents/Resources/data/` の 4 ファイルを mmap し、`mincore` で常駐の割合を見てから全ページに触る。束の外のファイルが読めないこと（対照）も見る |
| `xpc-host.c` | 束の中の殻の代わり（Sandbox なし）。サービスに 1 回要求を送り、往復の時間を出す |
| `engine-probe.rs` | 本物の `Engine::open`（LM なし）を Sandbox の中で呼ぶ。`engine/Cargo.toml` の `[[example]]` で作る（dev-dependencies の `dev-knobs` が付く。開く時間には効かないと推測）。辞書は、コンテナの中の試験用のデータの場所の `dict/` を、束の `dict/` へのリンクにして見せた（Sandbox はリンクの先で判定する） |
| `resident.c` | ファイルの常駐の割合だけを出す（ページを読み込まない） |
| `c1.sh <作業dir> <adhoc\|team> [印]` | C1 の全部。束を作り、署名し、データを clone で置き直して（新しい vnode なので、ファイルのキャッシュに無い状態になる。`resident` で 0% を確かめた）冷えた状態と温まった状態を測る |
| `diag*.sh` | 最初の起動で誰がデータを読むかを分けた試験（下の「分かったこと 3」） |
| `c2.sh <作業dir> <adhoc\|team> <data\|nodata>` | C2。`target/macos/release-team/ikari.app` を写し、`IkariEngine.xpc/Contents/Resources/data/` に置いて、`build.sh` の `sign_bundle` と同じ順・同じ引数で署名し直す。束は起動しない |
| `logs/` | 実際の出力（パスは `W`・`~` に伏せた） |

使った場所: worktree の `target/spike578/`（ホームの下。Sandbox の判定を本物に近づけるため）と、スパイクの識別子（`io.github.ikari-zen.spike578.*`）のコンテナだけ。install、smoke、本物の ikari の停止、入力ソースの変更、`~/Library/Application Support/ikari-ime/` への書き込みはしていない。

## 結果

### C1: Sandbox の中のエンジン（XPC の束）が、自分の束の中のデータを読めるか → **読める**
| 項目 | ad-hoc | Apple Development |
| --- | --- | --- |
| `sandbox_check` | 1（Sandbox の中） | 1 |
| 4 ファイルの open と mmap | すべて成功 | すべて成功 |
| 束の外（ホームの下の別のファイル）の open（対照） | EPERM | EPERM |
| `CodeResources` に data のファイルが載る | 14 行（7 ファイル × files・files2） | 同じ |
| XPC の起動 + 4 ファイルの全ページ（データが冷えた状態） | 1095 / 1019 / 1117ms（3 回） | 1091ms |
| 同（温まった状態） | 117 / 76ms | 60ms |
| うち全ページに触る時間（冷えた → 温まった） | 約 1.1s → 約 50〜60ms（約 725MB） | 1.08s → 51ms |
| `Engine::open`（LM なし。冷えた → 2 回目） | 436 / 265ms → 299 / 126ms | 274ms → 128ms |
| 開いたあとの常駐（`system.dict`） | 40%（`grammar.bin`・`vectors.bin` はほぼ 0%。開くときには全部を読まない） | 同じ |
| モデルの sha256（エンジンの照合と同じ読み方、73.9MB） | 49 / 45ms（冷えた）、36〜44ms（温まった） | 39ms → 36ms |

### C2: 750MB を含む束の署名と検証（`build.sh` と同じ順・引数。`--timestamp=none`、`--options runtime`）
| 項目 | ad-hoc（データなし） | ad-hoc（データあり） | Apple Development（データなし） | Apple Development（データあり） |
| --- | --- | --- | --- | --- |
| 束の大きさ（du） | 51MB | 765MB（data 714MB） | 51MB | 765MB |
| 署名の合計 | 277ms | 2914ms | 488ms | 3223ms |
| うち `IkariEngine.xpc` | 39ms | 2634ms | 66ms | 2792ms |
| `verify --deep --strict`（温まった） | 26 / 30ms | 297 / 163ms | 29 / 33ms | 354 / 303ms |
| 同（冷えた） | — | 256 / 311ms | — | 346 / 475ms |
| 検証の結果 | 成功 | 成功 | 成功 | 成功 |
| `vectors.bin` の 1 バイトを変えたあと | — | 失敗（`a sealed resource is missing or invalid`、`IkariEngine.xpc`） | — | 同じ |

### 分かったこと
1. **C1 は通る。** App Sandbox の XPC サービスは、ホームの下に置いた自分の束の `Contents/Resources/data/` を open・mmap できる。束の外は EPERM なので、Sandbox は効いていて、束の読みは束だから許されている。署名の方式（ad-hoc、team）で差はない。親の `ikari.app/Contents/Resources` を読めるかは試していない（ADR §3.4 のとおり XPC の束に置くので要らない）。
2. **起動の時間。** `Engine::open`（LM なし）は、データが冷えた状態で約 0.27〜0.44s、温まった状態で約 0.13s。開くときに読むのは `system.dict` の 40% ほどで、`vectors.bin` と `grammar.bin` は開くときにはほとんど読まない（触ったところだけ読まれる）。全部のページに触っても冷えた状態で約 1.1s。モデルの sha256 は約 40ms。
3. **新しく置いた束の、最初の exec で、束の全体が読まれる（想定外）。** 新しい場所に置いた（作り直した・clone で写した）束の主の実行ファイル（殻）を最初に exec すると、約 2.1〜2.6s かかり、その間にデータの 4 ファイルが 100% 常駐になる（`diag4.sh`: データありの束 2210ms、データなしの束 95ms、どちらも 2 回目は 10ms）。同じ場所の束で殻の cdhash だけ変えても（`diag3.sh`）、XPC サービスの cdhash だけ変えても（`diag2.sh`）起きない。束を置いた最初の 1 回だけ。誰が読んでいるか（syspolicyd の評価など）は確かめていない（`log show` に出なかった）。隔離の印のない束での結果で、ブラウザで取った dmg から写した束（隔離の印あり）では、別の評価が足されるかもしれない（C3、#241）。
4. このため、実際の使い方（置いた直後に IME が起動する）では、エンジンが最初に開くときにはデータが既に温まっている見込みが高い（推測）。再起動のあとは冷えた状態（約 0.3〜0.4s の `Engine::open`）。
5. **C2: 署名は約 3s、検証は約 0.3〜0.5s。** 署名の時間はほとんど `IkariEngine.xpc`（データの封印）で、ad-hoc と Apple Development の差は小さい（+0.2〜0.3s）。封印は効いていて、データの 1 バイトの変更で検証が失敗する。

### 推測
- 署名が検証より 1 けた遅いのは、署名が資源ごとに SHA-1 と SHA-256 の 2 つを数えるため（files と files2）と、並列で読まないため。確かめていない。
- 「最初の exec で束が読まれる」のは、新しい束を最初に動かすときのシステムの評価（封印の検証など）。`verify --deep --strict` は 0.3s なので、それより重い何か（マルウェアの検査など）が足されている可能性がある。

### ADR 0080 に反映すべきこと（ADR 自体は変えていない）
1. §3.4: 「Sandbox の中のプロセスは自分の束を読める」は、この構成で確認済み（ad-hoc・team とも。束の外は EPERM）。未確認の印を外す。
2. §3.4 の推測「750MB を封印すると、署名と最初の起動の検証に時間がかかるかもしれない」を数値に替える: 署名は約 3s（データなし 0.3〜0.5s）、`verify --deep --strict` は約 0.3〜0.5s、**新しく置いた束の最初の起動は約 2.2〜2.6s（データなし 0.1s）。束を置いた最初の 1 回だけ**。CI のランナーでは測っていない（§10 の C2 の「ランナー」の分は残る）。
3. §10 の C2 に「最初の起動」の項を足すなら、隔離の印のある束（dmg から写したもの）でも測ること（C3、#241 と一緒に）。
4. §3.3 の反映: 更新で束が置き換わると、最初の起動で約 2s かかる（3 から）。初回の変換が遅れる理由として、手順書か §14 の結果に書く候補。
5. 起動の時間の判断（§3.2 の「1 番目が検査に落ちたら 2 番目」）は、`Engine::open` が約 0.1〜0.4s なので、2 つの場所を順に試しても、起動の時間は大きく増えない（推測。落ちる側の検査の時間は測っていない）。

## 後始末
- 試験の束（約 2.1GB。clone なので実際の使用量はわずか）は消した。`target/spike578/` には小さいログだけ。
- スパイクの識別子のコンテナ 6 つ（`~/Library/Containers/io.github.ikari-zen.spike578.{adhoc,adhoc.r2,team}.{probe,engineprobe}`）が残る。中は空に近い（試験用のデータの場所はプログラムの終わりに消した）。#489 と同じく `rm` では消せない見込み（containermanagerd の保護）。ユーザーが Finder などで消すか、そのままでよい。

## 再現
```sh
macos/scripts/build.sh --signing team --release     # C2 の元の束
spike/adr0080-macos-578/c1.sh "$PWD/target/spike578/w" adhoc .r2   # 印を毎回替える（新しいコンテナで最初の起動を見る）
spike/adr0080-macos-578/c1.sh "$PWD/target/spike578/w" team
spike/adr0080-macos-578/diag4.sh "$PWD/target/spike578/w" Spike578-adhoc.r2.app a
spike/adr0080-macos-578/c2.sh "$PWD/target/spike578/w" adhoc data   # adhoc|team × data|nodata
rm -rf target/spike578/w
```
