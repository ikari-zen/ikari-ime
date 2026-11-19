# スパイク #489: macOS の ikari-data の Sandbox（ADR 0079 §5 の (a)〜(d)）

spike。マージしない。捨てるコードと結果の記録（2026-11-20、macOS 27.2.1、arm64）。

## 中身
- `data-spike.c`: ikari-data 相当（`pw_dir` と `HOME` を出す。`fetch/` 作成、`.lock` と flock、`inbox/x.partial` の作成と rename、`statvfs`、例外の外への書き込み）。`net` モードは `getaddrinfo`+`connect(:443)` と、子の `/usr/bin/curl -I https://huggingface.co/`（HEAD 1 回、本文なし）。C で書いた（見たいのは OS の振る舞いで、Rust の std も同じ syscall のため）。
- `spawner.c`: 束の中の設定アプリ代わり（Sandbox なし）。`posix_spawn` で起こす。
- `Info.plist` / `data.entitlements`: ADR 0079 §3.3 の 3 キー。一時例外は `…/ikari-ime-spike/fetch/` と `…/inbox/`。
- `run.sh <作業dir> <adhoc|team> [識別子の印]`: 全項目を流す。`flip.sh`: 署名の方式の切り替えの確認。`log-adhoc.txt`、`log-team.txt`: 実際の出力（個人情報は伏せた）。
- 使った場所は `~/Library/Application Support/ikari-ime-spike/` と、スパイクの識別子のコンテナだけ。本物の ikari、入力ソース、本物のデータの場所には触れていない。smoke なし。kill はスパイクの自分の子だけ。

## 結果
| 項目 | ad-hoc | Apple Development（team。`L9Z8SK775X`） |
| --- | --- | --- |
| (a) `ikari-ime-spike/` だけある状態から、`fetch/` 作成、`.lock` 作成と flock、`download.bin` 書き、`inbox/` 作成、`inbox/x.partial/` 作成と書き、rename、`statvfs` | すべて通った | 同じ（すべて通った） |
| (a) 例外の外（`root/other` の mkdir、`root/other-file` の書き、`root` の open、HOME 直下の mkdir） | すべて EPERM | 同じ |
| (a) 親が先に `fetch/`・`inbox/` を作った形 | 通る（`mkdir` は EEXIST） | 同じ |
| (a) 根の扱い: `lstat(root)` | 通る | 通る |
| (a) 根の `mkdir`（`create_dir_all` の最初の段） | 根がある: EEXIST（EPERM ではない）。根が無い: EPERM、続く `fetch/` は ENOENT | 同じ |
| (b) Sandbox 内の `HOME` | コンテナ `~/Library/Containers/<識別子>/Data` | 同じ |
| (b) `pw_dir` | 本当のホーム | 同じ |
| (b) 環境変数を空にして起こす（`env -i`） | `HOME` はそれでもコンテナ（Sandbox の初期化が足す） | 同じ |
| (b) 作ったファイルの xattr | `com.apple.quarantine`（ファイル、ディレクトリとも）と `com.apple.provenance` が付く | 同じ |
| (c) 例外のパスが署名で変わるか | （基準） | 変わらない。a〜b と同じ結果 |
| (d) Info.plist なし | exit 133（SIGTRAP。0076 §16 と同じ） | 同じ |
| (d) Helpers 以外の場所（スクラッチ）から Terminal | a〜b と同じ | 同じ |
| (d) `Spike.app/Contents/Helpers/` から Terminal | 同じ。`codesign --verify --deep --strict` が通り、`CodeResources` に名前がある | 同じ |
| (d) 束の中の Sandbox なしの exe から `posix_spawn` | 同じ（子の exit 0） | 同じ |
| 通信（`network.client` と一時例外を一緒に） | `connect :443` 成功。子の curl の HEAD は 200 | 同じ |

### 分かったこと
1. **(a) は通る。§3.5 の既定（`ikari-data` が `fetch/` と `inbox/` を自分で作る）で足りる。** 根（データの場所そのもの）の下に、例外の 2 つのパスそのものを新しく作れる。根が無いときは、根の mkdir が EPERM で作れない（ADR のとおり `internal` で終わる）。
2. **根に対する操作の幅**: `lstat(root)` は通る（根があり、ディレクトリで、リンクでないことは確かめられる）。`open(root, O_RDONLY)`（根の一覧）は EPERM。例外の外の `lstat(root/other)` は、EPERM ではなく ENOENT（`other` は元から無いため。無い名前は見られる）。ADR §3.5 の「根は確かめるだけ」の形でよい。根に `create_dir_all` をかけた場合は、根があれば mkdir が EEXIST で返るので（Rust の `create_dir_all` は EEXIST のとき続ける。このスパイクでは C の mkdir の errno だけ確認）失敗しない。根が無ければ EPERM で失敗する（意図どおり）。
3. **(b) `HOME` はコンテナを指す。§3.4（`getpwuid_r` の `pw_dir`）の決定が必要。** `HOME` を `UserDir::resolve` のように使うと、例外とずれて根の `lstat` すら ENOENT になる。環境変数を消しても `HOME` はコンテナに戻るので、親が `HOME` を渡さない（§3.6）形は問題ない。
4. **(c) team 署名でも同じ。** ad-hoc の実測と差は無い。
5. **(d) 置き場所は関係ない。** Info.plist を `-sectcreate` で埋めてあれば、Helpers でも外でも、Terminal からでも束の中の exe の `posix_spawn` からでも起動する。埋めなければどこでも SIGTRAP（133）。ADR 0077 §8 の未確認（Helpers の中の起動）は、`ikari-data` の型については確認できた。`/Library/Input Methods` は範囲外（#241）。
6. **隔離の印**: Sandbox 内の exe が作ったファイル、`inbox/`、`inbox/x/` に `com.apple.quarantine`（値 `0086;…;<exe 名>;`）が付く。エンジンの取り込みが印に左右されないことは、このスパイクでは見ていない（ADR §6 の推測のまま。§7 の 5 の実機の確認で見る）。
7. **署名の方式を替えると、同じ識別子のコンテナで起動が止まる（想定外。要確認）。** ad-hoc で作ったコンテナ（識別子 `…data.spike`）を、同じ識別子で team 署名の exe が使おうとすると、起動の途中（`_libsecinit_appsandbox` の `xpc_pipe_routine`、containermanagerd の返事待ち）で止まり、返らない（`sample` で確認。少なくとも 2 分）。識別子を替えれば（`.team`、`.t3`）team でも問題なく動く。逆向き（team で作ったコンテナを ad-hoc で、そのあと元の team で）も止まり、止まったあとは元の署名でも戻らなかった（`flip.sh`。数分あけて再実行しても 15 秒で打ち切り）。止まるのはその識別子だけで、別の識別子（`.t3`）は同じ時間帯に動いた。ユーザーの ikari には影響は見ていない（見に行っていない）。原因は推測（コンテナの持ち主の署名が変わったことの確認を待っている可能性。画面の確認が出ていたかは見ていない）。
8. **後始末の制限**: スパイクのコンテナ 3 つ（`…ikari.data.spike`、`….spike.team`、`….spike.t3`）は、`rm -rf` が `Operation not permitted`（containermanagerd の保護。以前の `cagetest` のコンテナも同じ理由で残っている様子）で消せなかった。`~/Library/Application Support/ikari-ime-spike/` と HOME 直下は消した。コンテナはユーザーがあとで Finder などで消すか、そのままでよい（中は空に近い。`Data` だけ）。

## ADR への提案（ADR 自体は変えていない）
1. §8 に上の結果を書き、§5 の「(a) が通らなければ」の分岐を閉じる。§3.5 の既定を確定し、代わりの案（親が作る）は残すだけにする。
2. §3.5 に「根があるときの `create_dir_all` も通る。根が無いときは mkdir が EPERM。この EPERM を `internal` に写す」を足す（実装の注意）。`lstat`、`open(root)` の差（読み取りの一覧は不可）も書く。
3. §3.4 は確定。「`HOME` は Sandbox が必ずコンテナに設定する（環境を空にしても）」を理由に足す（R5 のためだけではない）。
4. §3.6 の「環境は proxy の変数だけに絞る」は、`HOME` を渡さなくても動くことを確認済みと書ける。ただし実際の ureq（`NativeTls`、証明書の確かめ）の通信は見ていない（このスパイクは TCP の connect と子の curl だけ）。実機の確認（§7 の 5）で見るか、段階 1 で 0076 §16 と同じ ureq の exe を足して 1 回確かめる。
5. 新しい項目（§3.2 か §9 の限界）: **同じ識別子で署名の方式（ad-hoc、team）を切り替えると、既存のコンテナで Sandbox の初期化が止まる。** ADR 0069 §6 の配布（ad-hoc）と、利用者が team 署名で自分でビルドして入れ替える場合に当たる。対策の候補: (i) 配布の署名の方式を固定し、切り替えない、(ii) 識別子に方式を入れる（コンテナが分かれる。エンジンの `engine` と `lm-cage` も同じ問題があるはず。要確認）、(iii) 手順書にコンテナの削除を書く（`ikari-data` はコンテナに大事なものを持たない。取得の作業は `fetch/` で、コンテナは使わない）。リードと architect が決めるもの。
6. §7 の 5 に、確認する項目を足す: 取り込みのときの `com.apple.quarantine`（#6）。
7. §3.7 の検査に変更は不要。スパイクの束の `CodeResources` に Helpers の exe の名前があることは確認できた。
8. §5 の手順 7（片づけ）に、「コンテナは `rm` で消せない」と書く。次は識別子に毎回違う印を付けるか、消し方（Finder、コンテナを持つ ID のゴミ箱）を決める。

## 再現
```sh
spike/macos-ikari-data-489/run.sh <作業dir> adhoc            # ad-hoc
spike/macos-ikari-data-489/run.sh <作業dir> team .team        # team（別の識別子にする。理由は分かったこと 7）
```
