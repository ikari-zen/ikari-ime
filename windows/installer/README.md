# Windows のインストーラ（骨組み）

Inno Setup のスクリプト（`ikari.iss`）と、インストーラが管理者で呼ぶ道具（`helper/`、Rust の `ikari-setup-helper`）。
設計は [security-design.md](../../docs/security-design.md) §8.4、[ADR 0067](../../docs/adr/0067-windows-install-layout-and-launch-chain.md) §3・§4、[ADR 0068](../../docs/adr/0068-ipc-endpoints.md) §6.2、[ADR 0070](../../docs/adr/0070-update-and-vulnerability-response.md) §4、[ADR 0063](../../docs/adr/0063-windows-tip-dll-update.md)、[ADR 0073](../../docs/adr/0073-secret-input-and-user-data.md) §4。
実機の確認は [windows-real-device-check.md](../../docs/windows-real-device-check.md) の項目 47〜55。まだ実機で流していない。

## 作り方（Windows で）

道具: Inno Setup **6.6.0 以上**（推奨 6.7.4 以上。下限はスクリプトの先頭で検査する）。

配る物は、タグからリリースの workflow（[release.yml](../../.github/workflows/release.yml) の `build-windows`）が作る。手順はそれと同じ:

1. 製品のビルド（`dev-knobs` なし）を作る。エンジン（`hide-console`）、TIP、候補窓と設定画面の核、helper（`cargo build --release -p ikari-setup-helper`）、取得の子（`-p ikari-data`。別の cargo の呼び出しで）と、LM の子（`-p ikari-lm --bin ikari-lm-host --features vulkan`）。
2. 配置の形（ADR 0067 §3.1）に並べたフォルダ（以下 `<stage>`）を作る:

   ```
   <stage>\ikari-engine-host.exe
   <stage>\ikari-lm-host.exe、llama*.dll、ggml*.dll、backends\
   <stage>\candidate\…（候補窓。dotnet publish の自己完結）
   <stage>\settings\…（設定画面。dotnet publish の自己完結）
   <stage>\tip\ikari_tip.dll、tip\resources\keymap\*.tsv
   <stage>\ikari-setup-helper.exe
   <stage>\ikari-data.exe（取得の子。送信ブロックの規則はかけない）
   <stage>\LICENSE、NOTICE、THIRD_PARTY_NOTICES.md、THIRD_PARTY_LICENSES_RUST.md（元は THIRD_PARTY_LICENSES_RUST_WINDOWS.md。cargo xtask licenses が作る）
   ```

3. `cargo xtask scan-product --stage <stage>` で検査する（あるべき物、開発用の文字列、動的な C ランタイム、開発用の exe や測定ツールが無いこと。`xtask/src/scan/stage.rs`）。
4. `ikari_tip.dll` の sha256 を取り、`ISCC` に渡す:

   ```powershell
   $sha = (Get-FileHash <stage>\tip\ikari_tip.dll -Algorithm SHA256).Hash
   ISCC /DAppVersion=0.1.0 /DStageDir=<stage> /DTipSha256=$sha windows\installer\ikari.iss
   ```

   セキュリティの修正を含む版は `/DSecurityUpdate=1` を足す（最後の案内が変わる。ADR 0070 §4 の 1。リリースの workflow には、まだ渡す口が無い）。
   出力は `windows\installer\out\ikari-ime-<版>-windows-x64-setup.exe`（workflow は `/O` で別の場所に出す）。

タグを打たずに試すときは、リリースの workflow を `workflow_dispatch` で流す（Windows のビルドだけ。証明と Release は作らない。版は 0.0.0）。

## 何をするか（順）

インストール: 管理者か確かめる → インストール先が `C:\Program Files\ikari-ime` か確かめる → 作業用のフォルダ
（`C:\Program Files\ikari-ime.setup`。毎回作り直す）に helper を置く → 既にあるインストール先の検査 →
動いているエンジンを止める（すべてのセッション）→ ファイルを置く（TIP は `tip\<版>\`）→ 最後の検査 →
`regsvr32` で登録 → ファイアウォールの送信ブロックの規則 → HKLM の Run → エラー報告の除外 → 古い版の `tip\` を消す。

削除: Run → 規則 → エラー報告の除外を戻す → 登録の解除 → エンジンを止める → ファイル（使用中は再起動で）。
LM の子と取得の子（`ikari-data transfer`）の檻（AppContainer）のプロファイルは利用者ごとにあり、helper の `cage-profile-delete` で消せるのは、削除を動かした利用者の分だけ（ほかの利用者の分は、空のプロファイルとして残る。ADR 0072 §12.8 の 3、ADR 0078 §3.2 の 1）。
利用者のデータ（各利用者の `%LOCALAPPDATA%\ikari-ime`）には触れない。

インストーラは、利用者ごとのもの（HKCU、利用者のフォルダ、入力方式の一覧）に触れない。入力方式の追加は、最後の画面で案内する。

## 守っていること（`cargo xtask check` が検査する）

検査の中身は `xtask/src/check/installer.rs`。要点:

- Inno の一時の場所（利用者の TEMP の下）に、ikari のファイルを展開しない（`ExtractTemporaryFile`、`{tmp}` を使わない）。道具は Program Files の下の作業用のフォルダに、`[Files]` で直接置く。
- 外のプログラムの起動は `RunTrusted` の 1 か所だけ。相手は system32 の道具（`regsvr32`、`netsh`）と helper だけで、絶対パス。
- `[Files]` の `regserver`、`restartreplace` の印を使わない。インストール先の項目は、検査の Check を通る。
- 使用中のファイルを持つアプリを閉じさせない（`CloseApplications=no`）。ログを出さない（利用者の TEMP に書かれるため）。

## 統合のときの作業（インストーラのレーンで足す）

- 削除の手順に、helper の `cage-profile-delete`（引数なし）の呼び出しを足す。場所は `ikari.iss` の削除の流れで、`IncludeInErrorReporting` の隣（エンジンを止めたあと、ファイルを消す前。helper はインストール先の中の `AppHelper`）。形は `RunTrusted(AppHelper, 'cage-profile-delete', Code);`。失敗しても削除は続ける（`wer-include` と同じ扱い）。終了コードは 0 が成功、4 が OS の失敗（`hr=` を 1 行出す）。プロファイルが無くても成功。

## 限界（分かっていること）

- Inno Setup 自身の部品（`setup.tmp` など）は、昇格した利用者の TEMP に置かれる。名前は推測できない（6.6.0 以上）が、場所の ACL はこちらで用意できない。実機の項目 55 で見て、道具を本決定にする（ADR 0067 Q3 は仮）。
- 既にあるインストール先が検査に落ちたとき、ファイルは置かず登録もしないが、Inno の削除用のファイル（`unins000.exe`）はそのフォルダに書かれる。
- 削除のときは、インストール先の中の helper を使う（インストールの最後に検査したもの）。
