# プロキシの下で元データを取得する

`scripts/fetch-data.sh`（中身は `cargo xtask data fetch`）は、辞書の元データなどを `curl` で取得します。このため、**curl が読む設定がそのまま効きます**。専用の設定はありません。
ここでは、認証つきのプロキシがある環境での手順をまとめます。IME 本体は通信しません（NFR-17）。通信するのは取得の段階だけです。

関連: [#162](https://github.com/ikari-zen/ikari-ime/issues/162)（オフライン導入と更新の仕組み）、[#249](https://github.com/ikari-zen/ikari-ime/issues/249)。

## 1. 分かったこと

### 取得スクリプトの動き

- 取得は `curl --fail --silent --show-error --location --retry 3 --output <一時ファイル> <URL>` です（`xtask/src/data/fetch.rs`）。`-x` や `--proxy-user` は付けません。したがって、**環境変数と `~/.curlrc` が効きます**。
- URL は、スクリプトに固定した配布元だけです（SudachiDict、chiVe、Hugging Face の zenz、GitHub の emoji-data など）。ユーザー名やパスワードは含みません。
- ダウンロードのあとに sha256 を確かめます。プロキシが中身を書き換えたり、エラーページを返したりしても、合わなければ捨てます。

### curl が読む環境変数

一次資料: curl の資料 [Proxy environment variables](https://everything.curl.dev/usingcurl/proxies/env.html)、[Proxy authentication](https://everything.curl.dev/usingcurl/proxies/auth.html)（2026-11-10 に確認）。

| 変数 | 意味 |
| --- | --- |
| `https_proxy` / `HTTPS_PROXY` | HTTPS の取得に使うプロキシ（CONNECT で中継される）。大文字でも小文字でもよい |
| `http_proxy` | HTTP の取得に使うプロキシ。**小文字だけ**効く。大文字の `HTTP_PROXY` は、CGI の `HTTP_` 変数による注入を避けるため無視される |
| `all_proxy` / `ALL_PROXY` | 上の 2 つがないときの既定。個別の変数が優先 |
| `no_proxy` / `NO_PROXY` | プロキシを通さないホスト。コンマ区切り。`*` は全部。先頭のドット（`.example.com`）は配下のドメイン。CIDR は curl 7.86.0 から |

`scripts/fetch-data.sh` の配布元はほぼ HTTPS なので、**`https_proxy` を設定すれば足ります**。

### 認証の種類

| 種類 | curl の指定 |
| --- | --- |
| Basic（既定） | `--proxy-user user:pass`（`-U`）か、環境変数の URL に `http://user:pass@host:port` |
| Digest | `--proxy-digest` |
| NTLM | `--proxy-ntlm` |
| Kerberos 等（Negotiate） | `--proxy-negotiate`（`-U :` を併用） |
| 自動選択 | `--proxy-anyauth`（往復が増える） |

- 認証が足りない・違うと、プロキシは 407 を返します。407 の `Proxy-Authenticate` ヘッダが、プロキシが受け付ける方式を示します。
- システムのプロキシ設定（macOS の「システム設定 > ネットワーク > プロキシ」、`scutil --proxy`）を curl は読まない、というのが一般に知られた動作です（この検証では確かめていない。未確認）。環境変数か curl の設定で与えます。ブラウザで通っていても、curl では通らないことがあります。

## 2. 手順

### 認証なしのプロキシ

```sh
export https_proxy=http://proxy.example.com:8080
scripts/fetch-data.sh --only sudachi --only chive --only zenz
```

### Basic 認証つきのプロキシ

```sh
export https_proxy=http://user:pass@PROXY_HOST:8080   # user と pass はダミー
scripts/fetch-data.sh --only sudachi --only chive --only zenz
```

パスワードに `@`、`:`、`/` などを含むときは、URL エンコードする（`@` は `%40`）。エンコード後の値の動作は、このページの検証では確かめていない（未確認）。

### NTLM・Kerberos（Negotiate）の会社

curl の `--proxy-ntlm`、`--proxy-negotiate` はあります。`~/.curlrc` に書けば、取得スクリプトにも効くはずです。

```
# ~/.curlrc の例（NTLM）
proxy-ntlm
proxy-user = "DOMAIN\user:pass"
```

実環境では、curl の組み込みの NTLM や Kerberos より、**会社の端末に cntlm などのローカルの中継プロキシ**を立てて、そこへ向ける方法が一般的です（推測。ikari では導入しない。説明だけ）。中継プロキシは 127.0.0.1 で認証なしに待ち受け、上流の NTLM を肩代わりします。この場合は「認証なしのプロキシ」と同じに書けます（`https_proxy=http://127.0.0.1:3128`）。

**NTLM と Kerberos は、実際のプロキシで確かめていない（未確認）**。確かめたのは、Basic だけのプロキシに対して、`--proxy-ntlm` と `--proxy-negotiate` を付けると 407 で終わる（つまり Basic の認証情報は使われない）ことまでです。

### 認証情報を設定ファイルに平文で置かない

パスワードを `~/.curlrc` やシェルの設定ファイルに平文で書くのは避けます。次の方法があります。

- **その場の環境変数**（履歴に残さない）。例: `read -rs PROXY_PASS` で入力し、`https_proxy="http://user:${PROXY_PASS}@PROXY_HOST:8080" scripts/fetch-data.sh …`。
- **キーチェーンから取る**（macOS）。保存: `security add-generic-password -a user -s ikari-proxy -w`（パスワードを聞かれる）。取得: `PROXY_PASS=$(security find-generic-password -a user -s ikari-proxy -w)`。この 2 つのコマンドの動作は、この文書の検証では流していない（未確認）。
- **`~/.curlrc` を使うなら権限を 600 に**する（`chmod 600 ~/.curlrc`）。

`curl -U user:pass` のコマンドラインは、`ps` に出ないよう curl が伏せ字にします（確かめた。下記）。ただし、シェルの履歴には残ります。

注意: **`~/.netrc` はプロキシの認証には使われませんでした**（確かめた。`machine` にプロキシのホストを書いても 407）。`.netrc` は取得先のサーバのための認証です。プロキシには環境変数、`--proxy-user`、`~/.curlrc` の `proxy-user` を使います。

### 取得スクリプトの出力に、認証情報は出ない

URL はスクリプトに固定しており、認証情報は環境変数で渡るだけなので、取得スクリプトの出力（失敗時の「ダウンロードに失敗しました」の、curl のコマンド行を含む）には出ません（確かめた。誤ったパスワードで失敗させ、出力に `wrong`、`user:` がないことを確認）。
ただし、**自分で `curl -v` を使うと、`Proxy-Authorization: Basic …`（user:pass を base64 にしただけのもの）が出ます**。ログや issue に貼る前に消してください。

### TLS を中継（復号）するプロキシの CA

会社のプロキシが TLS を中継して独自の CA で再署名する場合、curl は証明書エラーになります。

```
curl: (60) SSL certificate problem: self signed certificate
```

会社の CA 証明書（PEM）を用意して、次のどれかで渡します。

| 指定 | 備考 |
| --- | --- |
| `export CURL_CA_BUNDLE=/path/to/ca.pem` | 今回の macOS の curl（下記の版）で効いた |
| `export SSL_CERT_FILE=/path/to/ca.pem` | 同上で効いた（curl の作りによっては効かないことがある） |
| `~/.curlrc` に `cacert = "/path/to/ca.pem"` | 常に効く。環境変数を足さずに済む |

**`--insecure`（`-k`）は使わないでください**。sha256 は確かめますが、通信の相手を確かめない状態を既定にしないためです。
`--proxy-cacert` は、プロキシ自体へ HTTPS で接続するとき（`https://proxy:…` 形式）の CA です。HTTP のプロキシに TLS を中継させる上の場合には要りません（上のテストでは付けても害はなかった。`https://` のプロキシでの動作は未確認）。
配布元の証明書を信用してよいかは、環境の管理者に確かめてください。

## 3. よくある失敗

| 症状 | 原因と対処 |
| --- | --- |
| `curl: (56) CONNECT tunnel failed, response 407`、または `curl: (22) HTTP response code said error` で、プロキシのログが 407 | 認証情報がない、または違う。ユーザー名・パスワード、URL エンコードを見直す。`NO_PROXY` でなく `https_proxy` に書いたか |
| `curl: (60) SSL certificate problem` | TLS を中継するプロキシ。CA を渡す（上の表） |
| `HTTP_PROXY` を設定したのに通らない（直接つなぎに行く） | `http_proxy`（小文字）だけが効く。HTTPS なら `https_proxy` か `HTTPS_PROXY` |
| `no_proxy` が効かない | 次を参照 |
| ブラウザでは通るが curl で通らない | curl は macOS のシステムのプロキシ設定を読まないはず（未確認。システムの設定は変えていない）。`scutil --proxy` の値を `https_proxy` に写す |
| `Could not resolve proxy` | プロキシのホスト名の綴り、またはスキームの付け忘れ |

### `no_proxy` の効く書き方・効かない書き方

ホスト名が `localhost` のとき、確かめた結果です。

| 書き方 | 結果 |
| --- | --- |
| `no_proxy=localhost` | 効く |
| `no_proxy=.localhost` | 効く |
| `no_proxy="foo, localhost"`（コンマの後ろに空白） | 効く |
| `no_proxy=localhost:18443`（ポートつき） | **効かない**（プロキシに行く） |
| `no_proxy=http://localhost`（スキームつき） | **効かない** |

ホスト名（かドメイン）だけを書いてください。

## 4. 確かめたこと

環境: macOS 27.2（Darwin 27.2.0）、`curl 8.7.1`（x86_64-apple-darwin26.0、SecureTransport、LibreSSL/3.3.6）、2026-11-10。`127.0.0.1` に、Python の標準ライブラリだけで作った Basic 認証つきのプロキシ（HTTP の転送と CONNECT）を一時的に立て、user / pass はダミー。HTTPS の取得先は、openssl で作った自己署名の証明書のローカルのサーバ。確かめたあと、プロセスと一時ファイルは消した。システムのプロキシ設定や、ほかのプロセスには触れていない。

| 確かめたこと | 結果 |
| --- | --- |
| HTTP の取得: 認証なし / 誤り / 正しい（`-x` と `-U`） | 407 / 407 / 200 |
| HTTPS の取得（CONNECT）: 認証なし / 誤り / 正しい | 407（`CONNECT tunnel failed, response 407`）/ 407 / 取得できた |
| 自己署名のサーバに、CA を渡さない | `curl: (60) SSL certificate problem`。`--cacert` で通る |
| `https_proxy`、`HTTPS_PROXY`、`all_proxy`、`ALL_PROXY`（URL に user:pass） | すべてプロキシを通って取得できた |
| `http_proxy`（小文字）と `HTTP_PROXY`（大文字） | 小文字だけプロキシを通った。大文字は無視され、直接つないだ |
| `no_proxy` の書き方 | 上の表のとおり |
| `~/.netrc`（`--netrc-file`）でプロキシの認証 | 効かない（407） |
| `-K`（`~/.curlrc` 相当）の `proxy` と `proxy-user` | 通った |
| `--proxy-anyauth` と Basic | 通った |
| `--proxy-ntlm`、`--proxy-negotiate` を Basic だけのプロキシに | 407（Basic の認証情報は使われない） |
| `CURL_CA_BUNDLE`、`SSL_CERT_FILE`（この curl） | どちらも効いた |
| `ps` に `-U user:pass` | パスワードが伏せ字になる |
| `curl -v` | `Proxy-Authorization: Basic <base64>` が出る |
| **`scripts/fetch-data.sh --only emoji`**（実際の取得先 raw.githubusercontent.com へ、`https_proxy=http://user:pass@127.0.0.1:…` で） | プロキシに `CONNECT raw.githubusercontent.com:443 auth=ok` が届き、sha256 も合って取得できた。誤ったパスワードでは、`curl: (22)` で失敗し、何も置かれない |
| 失敗時の出力に認証情報が出るか | 出ない |

`fetch-data.sh` の取得先は、スクリプトに固定されていて差し替えられない。辞書の元データ（数百 MB）は取らず、小さな emoji.json で、同じ curl の呼び出しがプロキシを通ることを確かめた。ほかの配布元（Sudachi の S3、Hugging Face、ABR）も、同じ `curl` の呼び出しなので同様のはずだが、プロキシ越しには取っていない（未確認）。

## 5. 未確認

- NTLM、Kerberos（Negotiate）、Digest のプロキシ。curl の指定は資料にあるが、実環境と cntlm などの中継プロキシで通したことはない。
- URL エンコードしたパスワード（`%40` など）の通り方。
- `https://` で待ち受けるプロキシ（`--proxy-cacert`）。
- キーチェーン（`security`）から取る手順、`~/.curlrc` の 600 の権限の効き方（curl は権限を検査しない）。
- Windows の curl。macOS の curl 以外。Homebrew の curl（OpenSSL）での `CURL_CA_BUNDLE` などの扱いの違い。
- 取得スクリプト以外の通信（下記）。

## 6. 取得スクリプト以外の通信

次は curl ではない道具です。いずれも、このページでは確かめていません（未確認）。

| 道具 | 設定（一般的なもの。詳細は各道具の資料） |
| --- | --- |
| git（Swift の依存パッケージの取得など） | `git config --global http.proxy http://user:pass@host:port`（平文で `~/.gitconfig` に残るので注意）、環境変数 `https_proxy` も読む |
| Cargo | 環境変数 `https_proxy`、または `~/.cargo/config.toml` の `[http] proxy`、CA は `[http] cainfo` |
| rustup、Homebrew | 環境変数 `https_proxy` などを読む |
| gh | 環境変数 `https_proxy` |

zenz のモデルは Hugging Face から、取得スクリプトの curl で取るので、`HF_ENDPOINT` は使いません。Hugging Face の道具（`huggingface-cli` など）は使っていません。

## 7. 取得できない環境

プロキシが通らない、通信が許されていない環境では、つながる環境で取得したファイルを `data/raw/` に置く方法が使えます。`scripts/fetch-data.sh --check` が、足りないファイルと sha256 を確かめます。製品としてのオフライン導入の仕組みは #162 の A で設計します。
