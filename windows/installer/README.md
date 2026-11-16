# Windows のインストーラ（骨組み）

Inno Setup のスクリプト（`ikari.iss`）と、インストーラが管理者で呼ぶ道具（`helper/`、Rust の `ikari-setup-helper`）。
設計は [security-design.md](../../docs/security-design.md) §8.4、[ADR 0067](../../docs/adr/0067-windows-install-layout-and-launch-chain.md) §3・§4、[ADR 0068](../../docs/adr/0068-ipc-endpoints.md) §6.2、[ADR 0070](../../docs/adr/0070-update-and-vulnerability-response.md) §4、[ADR 0063](../../docs/adr/0063-windows-tip-dll-update.md)、[ADR 0073](../../docs/adr/0073-secret-input-and-user-data.md) §4。
実機の確認は [windows-real-device-check.md](../../docs/windows-real-device-check.md) の項目 47〜55。まだ実機で流していない。

## 作り方（Windows で）

道具: Inno Setup **6.6.0 以上**（推奨 6.7.4 以上。下限はスクリプトの先頭で検査する）。

1. 製品のビルド（`dev-knobs` なし）を作る。エンジン、LM の子、TIP、helper（`cargo build --release -p ikari-setup-helper`）。
2. 配置の形（ADR 0067 §3.1）に並べたフォルダ（以下 `<stage>`）を作る:

   ```
   <stage>\ikari-engine-host.exe
   <stage>\ikari-lm-host.exe、llama*.dll、ggml*.dll、backends\（あれば）
   <stage>\candidate\…（候補窓。できるまで無くてよい）
   <stage>\tip\ikari_tip.dll、tip\resources\…
   <stage>\ikari-setup-helper.exe
   <stage>\LICENSE、THIRD_PARTY_LICENSES_RUST.md
   ```

3. `ikari_tip.dll` の sha256 を取り、`ISCC` に渡す:

   ```powershell
   $sha = (Get-FileHash <stage>\tip\ikari_tip.dll -Algorithm SHA256).Hash
   ISCC /DAppVersion=0.1.0 /DStageDir=<stage> /DTipSha256=$sha windows\installer\ikari.iss
   ```

   セキュリティの修正を含む版は `/DSecurityUpdate=1` を足す（最後の案内が変わる。ADR 0070 §4 の 1）。
   出力は `windows\installer\out\ikari-ime-<版>-windows-x64-setup.exe`。

リリースの workflow への組み込みは、まだしていない（`.github/workflows/release.yml` の持ち主のレーンで足す）。

## 何をするか（順）

インストール: 管理者か確かめる → インストール先が `C:\Program Files\ikari-ime` か確かめる → 作業用のフォルダ
（`C:\Program Files\ikari-ime.setup`。毎回作り直す）に helper を置く → 既にあるインストール先の検査 →
動いているエンジンを止める（すべてのセッション）→ ファイルを置く（TIP は `tip\<版>\`）→ 最後の検査 →
`regsvr32` で登録 → ファイアウォールの送信ブロックの規則 → HKLM の Run → エラー報告の除外 → 古い版の `tip\` を消す。

削除: Run → 規則 → エラー報告の除外を戻す → 登録の解除 → エンジンを止める → ファイル（使用中は再起動で）。
利用者のデータ（各利用者の `%LOCALAPPDATA%\ikari-ime`）には触れない。

インストーラは、利用者ごとのもの（HKCU、利用者のフォルダ、入力方式の一覧）に触れない。入力方式の追加は、最後の画面で案内する。

## 守っていること（`cargo xtask check` が検査する）

検査の中身は `xtask/src/check/installer.rs`。要点:

- Inno の一時の場所（利用者の TEMP の下）に、ikari のファイルを展開しない（`ExtractTemporaryFile`、`{tmp}` を使わない）。道具は Program Files の下の作業用のフォルダに、`[Files]` で直接置く。
- 外のプログラムの起動は `RunTrusted` の 1 か所だけ。相手は system32 の道具（`regsvr32`、`netsh`）と helper だけで、絶対パス。
- `[Files]` の `regserver`、`restartreplace` の印を使わない。インストール先の項目は、検査の Check を通る。
- 使用中のファイルを持つアプリを閉じさせない（`CloseApplications=no`）。ログを出さない（利用者の TEMP に書かれるため）。

## 限界（分かっていること）

- Inno Setup 自身の部品（`setup.tmp` など）は、昇格した利用者の TEMP に置かれる。名前は推測できない（6.6.0 以上）が、場所の ACL はこちらで用意できない。実機の項目 55 で見て、道具を本決定にする（ADR 0067 Q3 は仮）。
- 既にあるインストール先が検査に落ちたとき、ファイルは置かず登録もしないが、Inno の削除用のファイル（`unins000.exe`）はそのフォルダに書かれる。
- 削除のときは、インストール先の中の helper を使う（インストールの最後に検査したもの）。
