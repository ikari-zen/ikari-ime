# スパイク: 匿名の口につないだ側から、相手の署名要件を確かめられるか（#448 の FC-1。ADR 0077 §10）

捨てるコード。製品の束にもテストにも入れない。結果を [ADR 0077](../../../docs/adr/0077-macos-lm-cage.md) §10 に書いたら、このフォルダは消してよい。

## 問い

1. エンジンの役（つなぐ側）が、殻の役からもらった匿名の口（`xpc_endpoint`）に `xpc_connection_set_peer_code_signing_requirement` を付けたとき、相手が要件に合わなければ、返事は落とされるか（どの誤りになるか）。
2. 要件に合わない相手にも、**こちらが送った最初の伝言（fd つき）は届くか**。届くなら、最初の伝言は fd を付けない `hello` にする必要がある（契約はその形にした）。
3. 要件を付けない形で、返事の伝言から相手のコードを取り（`SecCodeCreateWithXPCMessage`）、自分で要件を確かめられるか（1 が使えないときの代わり）。

## 作り

- `CageSpike.app`（`host`。Sandbox なし）が、中の XPC サービス `io.github.ikari-zen.spike.cage`（`cage`）に名前でつなぎ、匿名の口をもらう。そのまま匿名の口につなぎ直して、`hello`（確かめのために fd を付ける）と、2 つ目の伝言を送る。
- 殻とエンジンを 1 つのプロセスにまとめている（エンジンの App Sandbox は付けていない）。つなぐ側の確かめの効き方を見るのが目的で、Sandbox の有無はこの問いに関わらない見込み（推測）。違いが疑わしければ、ADR 0072 §13.5 の試験用のアプリの形で試し直す。
- `cage` は、匿名の口に届いた伝言を `os_log`（subsystem `io.github.ikari-zen.spike`）に出し、`ok` を返す。

## 手順（リードとユーザーが実機で行う。利用者の ikari には触れない）

1. 作る（ad-hoc の署名）:
   ```sh
   macos/tools/cage-peer-check-spike/build.sh
   ```
   置き場所は `$TMPDIR/ikari-cage-spike/CageSpike.app`。
2. 別の端末でログを見る:
   ```sh
   log stream --style compact --predicate 'subsystem == "io.github.ikari-zen.spike"'
   ```
3. 3 つの形を順に流す（それぞれ数秒で終わる）:
   ```sh
   app="$TMPDIR/ikari-cage-spike/CageSpike.app/Contents/MacOS/host"
   "$app" match
   "$app" mismatch
   "$app" seccode
   ```
   `host: endpoint reply error:connection-invalid` で止まるときは、`open -W "$TMPDIR/ikari-cage-spike/CageSpike.app" --args match` で起動し直し、出力はログ（手順 2）で見る（直接起動では XPC サービスを探せない場合がある。推測）。
4. （できれば）team の署名で同じことをする:
   ```sh
   macos/tools/cage-peer-check-spike/build.sh --identity "Apple Development"
   SPIKE_TEAM=<チーム ID> "$app" match    # mismatch、seccode も
   ```
   チーム ID は `macos/scripts/lib.sh` の `TEAM_ID`。

## 見るもの（記録する）

| 形 | host の出力 | cage のログ | 何が分かるか |
| --- | --- | --- | --- |
| match | `set_peer_code_signing_requirement ... rc=0`、`hello reply dictionary`、`reply word=ok` | `received op=hello fd=present` | 合う相手なら通る（対照） |
| mismatch | `hello reply` が `error:peer-code-signing-requirement` か、ほかの誤りか。`second reply` も | **`received op=hello` が出るか**。出れば、確かめの前に伝言（と fd）が相手に届く（問い 2） | 問い 1、2 |
| seccode | `SecCodeCreateWithXPCMessage status=0`、合う要件で `status=0`、合わない要件で `status=-67050`（errSecCSReqFailed）になるか | `received op=hello` | 問い 3 |

- 記録には、macOS の版（`sw_vers`）、署名の形（ad-hoc / team）、各行の出力をそのまま残す。
- 問い 1 が「落とされる」で、問い 2 が「届く」なら、契約（`hello` に fd を付けず、確かめるまで `launch` を送らない）のまま実装に進める。問い 1 が効かなければ、問い 3 の形（返事の伝言から自分で確かめる）を実装の方式にする。
