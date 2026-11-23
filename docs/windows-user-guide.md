# Windows 版の導入・更新・アンインストールの手順（下書き）

> [!NOTE]
> **この文書は下書きです（2026-11-22 時点）。**
> ビルド済みのインストーラは、まだ配布していません（[README](../README.md)）。また、インストーラ（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）および一部の実機挙動は、Windows の実機（VM）での通し確認（[windows-real-device-check.md](windows-real-device-check.md) の項目 31、47〜55、71 など）が済んでいません。
> 実機で確かめていない画面・文言・挙動には、本文中で明示的に **（未確認）** と記しています。実機での確認が済み次第、事実に合わせて更新します。

この文書は、Windows 版の ikari-ime をインストーラで導入・更新・アンインストールする手順と、困ったときのログの場所・確認方法をまとめた利用者向けの手順書（下書き）です。
設計の根拠は [ADR 0063](adr/0063-windows-tip-dll-update.md)、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md)、[ADR 0069](adr/0069-distribution-authenticity.md)、[ADR 0070](adr/0070-update-and-vulnerability-response.md)、[ADR 0073](adr/0073-secret-input-and-user-data.md)、および [security-design.md](security-design.md) にあります。

---

## 1. 対象環境と前提

| 項目 | 内容 |
| --- | --- |
| 対象 OS | Windows 11（64 ビット / x64。インストーラの設定上は `MinVersion=10.0`、`x64compatible`） |
| 必要な権限 | **管理者権限**（インストーラの実行・更新・アンインストールに必要です） |
| インストール先 | `C:\Program Files\ikari-ime`（**固定**。変更できません） |
| 利用者のデータの場所 | `%LOCALAPPDATA%\ikari-ime`（利用者ごと。辞書、モデル、学習、ユーザー辞書、設定、ログ） |
| 対象アプリ（1.0.0） | ふつうの権限で動くデスクトップアプリ（後述の制限を参照） |

### プロセスの種類による制限（素通し）

ikari-ime（1.0.0）は、権限の境界を守るため（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §5）、入力先のプロセスの種類によって次のように動きます（実機での通し確認は [windows-real-device-check.md](windows-real-device-check.md) の項目 42 で **（未確認）**）。

- **ふつうのデスクトップアプリ**（整合性レベルが「中」で、AppContainer でないもの）: 通常どおり日本語変換が使えます。
- **高い権限のアプリ**（「管理者として実行」したアプリ、サービス、サインイン画面や UAC の画面）: **素通し**になります（**（未確認）**）。エンジンにつながず、設定やログも読み書きせず、打鍵をそのままアプリに渡します（変換はできません）。
- **ストアのアプリ（AppContainer）・低い整合性のプロセス**: 1.0.0 では**素通し**になります（**（未確認）**）。

---

## 2. 導入（インストール）

### 2.1 インストーラの入手と本物の確認

1. **公式の配布元は GitHub Releases だけです**（[ADR 0069](adr/0069-distribution-authenticity.md) §3.1）。
   - 配布ページ: `https://github.com/ikari-zen/ikari-ime/releases`
   - インストーラのファイル名: `ikari-ime-<版>-windows-x64-setup.exe`（例: `ikari-ime-1.0.0-windows-x64-setup.exe`）
   - Windows 版の配布はインストーラ（Inno Setup による exe）のみで、zip 配布はありません。プレリリース（`-rc` 等）の配布はなく、数字のみの正式版（`MAJOR.MINOR.PATCH`）で配布されます（`.github/workflows/release.yml`。CI でのインストーラ作成は `workflow_dispatch` で確認中）。
   - ミラーサイトや第三者のパッケージ集積所にあるものは、作者が [README](../README.md) に明記するまで公式ではありません。
2. **本物かどうかの確認（任意）**（[ADR 0069](adr/0069-distribution-authenticity.md) §3.2・§3.4。リリース手順と実機での確認は **（未確認）**）:
   - Release には、成果物のハッシュ一覧 `SHA256SUMS`、GitHub の成果物の証明（attestation）、作者の鍵による `SHA256SUMS` の署名（minisign 形式）を添付する設計です（`.github/workflows/release.yml` の `provenance` / `draft-release`）。
   - Windows 標準の `certutil` で SHA-256 を確かめるには、コマンドプロンプトまたは PowerShell で次を実行し、`SHA256SUMS` の値と比べます:

     ```powershell
     certutil -hashfile ikari-ime-<版>-windows-x64-setup.exe SHA256
     ```

   - GitHub CLI（`gh`）がある環境では、次でビルド経路の証明を確かめられます:

     ```powershell
     gh attestation verify ikari-ime-<版>-windows-x64-setup.exe -R ikari-zen/ikari-ime
     ```

### 2.2 未署名による OS の警告（SmartScreen・Smart App Control）

ikari-ime の最初の配布版には、Authenticode のコード署名が付いていません（[ADR 0040](adr/0040-windows-feasibility.md) §2.7、[ADR 0069](adr/0069-distribution-authenticity.md)）。そのため、Windows の保護機能によって次の警告や制限が出ます。

> [!IMPORTANT]
> **Windows Defender、SmartScreen、Smart App Control などの OS やウイルス対策ソフトの守りを切らないでください。**
> ikari-ime の公式文書では、OS の保護機能の無効化や、ウイルス対策ソフトの除外設定を求めることはありません（[ADR 0069](adr/0069-distribution-authenticity.md) §3.5）。守りを切るよう求める案内があった場合は、偽物の案内を疑ってください。

- **Microsoft Defender SmartScreen の警告（未確認）**:
  - インターネットから取得した未署名のインストーラ（`ikari-ime-<版>-windows-x64-setup.exe`）を起動すると、SmartScreen の警告画面が出る見込みです（**（未確認）**。実際の画面の文言と進める操作は、[windows-real-device-check.md](windows-real-device-check.md) の項目 31 で実機確認後にここへ追記します）。
  - 実行を進める場合は、公式の GitHub Releases から取得したファイルであること（および必要に応じて `SHA256SUMS`）を確かめた上で進めてください。
- **Smart App Control（スマート アプリ コントロール）が有効な PC（未確認）**:
  - Windows 11 の Smart App Control が「オン（有効）」になっている PC では、未署名の ikari-ime は使えません（[ADR 0069](adr/0069-distribution-authenticity.md) §3.5・§9。Smart App Control がインストーラ自身の起動を止めるか、インストーラ起動時の案内が出るかは [windows-real-device-check.md](windows-real-device-check.md) の項目 31・52 で **（未確認）**）。
  - インストーラが起動できた場合、レジストリ（`HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy` の `VerifiedAndReputablePolicyState = 1`）を検出すると次の案内を出します（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）（**（未確認）**: 項目 52）:
    > この PC では Smart App Control が有効です。ikari には署名がまだ無いので、署名が付くまで、この PC では ikari を使えません。
  - Smart App Control を切る手順は案内しません（一度切ると戻せない場合があり、PC 全体の守りが下がるためです）。署名付きの配布に対応するまでお待ちください。

### 2.3 インストーラの実行手順

1. `ikari-ime-<版>-windows-x64-setup.exe` を起動し、ユーザーアカウント制御（UAC）の確認が出たら管理者として許可します（管理者権限がない場合は「管理者として実行してください。」と表示されて終了します）（**（未確認）**: [windows-real-device-check.md](windows-real-device-check.md) の項目 47〜55）。
2. ライセンス（Apache License 2.0）を確認して進めます。インストール先は `C:\Program Files\ikari-ime` に固定されており、フォルダ選択画面は出ません（コマンドラインで `/DIR=` を指定しても拒否されます）（**（未確認）**: 項目 47）。
3. インストーラは機械全体（`C:\Program Files\ikari-ime` と `HKLM`）に対して次を行います（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §3.2）:
   - **インストール先の権限（ACL）の検査**: `C:\Program Files\ikari-ime` とその配下に、管理者（Administrators）・SYSTEM・TrustedInstaller 以外が書き込めるファイルやフォルダがないかを検査します（`ikari-setup-helper.exe check-dir`）。満たさない場合は登録を行わずに中止します（**（未確認）**: 項目 47）。
   - **ファイルの配置**:
     - `C:\Program Files\ikari-ime\ikari-engine-host.exe`（常駐の変換エンジン）
     - `C:\Program Files\ikari-ime\ikari-lm-host.exe`、`llama*.dll`、`ggml*.dll`、`backends\`（言語モデル推論の子プロセスと DLL）
     - `C:\Program Files\ikari-ime\candidate\ikari-candidate.exe`（候補窓）
     - `C:\Program Files\ikari-ime\settings\`（設定画面）
     - `C:\Program Files\ikari-ime\ikari-data.exe`（辞書・モデル取得用の子プロセス）
     - `C:\Program Files\ikari-ime\tip\<版>\ikari_tip.dll`、`resources\`（各アプリに読み込まれる TIP の DLL とキーマップ等の資源。版ごとのフォルダに置きます）
     - `C:\Program Files\ikari-ime\ikari-setup-helper.exe`、`LICENSE`、`THIRD_PARTY_LICENSES_RUST.md`
     - （設計上の追加予定: VC++ 再頒布パッケージ `vc_redist` の同梱は [ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §4.5 で決定済みですが、現在の `ikari.iss` には未組み込みです）
   - **TIP の COM / TSF 登録**: `C:\Program Files\ikari-ime\tip\<版>\ikari_tip.dll` を `regsvr32.exe /s` で登録します（`HKLM\Software\Classes\CLSID\{...}`、日本語の入力プロファイル、TIP の分類）（**（未確認）**: 項目 48）。
   - **ファイアウォールの送信ブロック規則の追加**: `ikari-engine-host.exe`、`ikari-lm-host.exe`、`candidate\ikari-candidate.exe` が外部へ通信しないよう、Windows ファイアウォールに送信ブロックの規則（`ikari-ime: block outbound <exe名>`）を登録します（**（未確認）**: 項目 49）。
   - **自動起動の登録**: ログオン時にエンジンが自動起動するよう、`HKLM\Software\Microsoft\Windows\CurrentVersion\Run`（値の名前 `ikari-ime`）に `"C:\Program Files\ikari-ime\ikari-engine-host.exe"` を登録します（**（未確認）**: 項目 50）。
   - **OS のエラー報告（WER）からの除外**: エンジン等が異常終了した際に、入力中の読みや文脈・学習データを含むメモリダンプが Microsoft へ送信されないよう、機械全体の除外リスト（`ExcludedApplications`）に登録します（[ADR 0073](adr/0073-secret-input-and-user-data.md) §4.4）（**（未確認）**: 項目 53）。

### 2.4 入力方式（キーボード）の追加（初回のみ手動）

インストーラは管理者権限（標準ユーザーとは別アカウントになりうる権限）で動くため、利用者ごとの設定（`HKCU` や `%LOCALAPPDATA%`、入力方式の一覧）には一切触れません（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §3.3）。
新規インストールの完了画面には、次の案内が出ます（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）（**（未確認）**: 項目 54）:

> 入力方式に ikari を足してください（設定 → 時刻と言語 → 言語と地域 → 日本語 → 言語のオプション → キーボードの追加）。インストーラは入力方式の一覧を変えません。

初回のみ、利用者の手で入力方式を追加してください（メニューの名前は Windows 11 の版によって多少異なる場合があります）:

1. Windows の **設定** を開きます。
2. **時刻と言語** → **言語と地域** → **日本語** の「…」から **言語のオプション** を開きます。
3. 「キーボード」の **キーボードの追加** を押し、一覧から **ikari** を追加します（**（未確認）**: インストーラで入れた状態での追加は項目 48・54 で確認待ち。開発用スクリプトでの登録後は項目 13 で確認）。
4. `Win + Space`（またはタスクバーの入力インジケーター）で入力方式を **ikari** に切り替えます。

> [!NOTE]
> **初回導入直後のエンジンの起動について**
> インストーラ自身は管理者権限で動いているため、インストールの最後にはエンジンを起動しません（高い権限で起動されたエンジンは何もせずに終了する設計のため。[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §4.3）。
> 導入直後にサインアウトせずそのままメモ帳などのふつうのデスクトップアプリで入力を始めると、TIP がエンジン（`C:\Program Files\ikari-ime\ikari-engine-host.exe`）を起動します（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §4.2。実機での動作は項目 50 で **（未確認）**）。次回以降のサインインでは、自動起動（`HKLM\...\Run`）によってログオン時にエンジンが起動します（**（未確認）**: 項目 50）。

---

## 3. 更新（アップデート）

### 3.1 新しい版の確認と入手

ikari-ime はバックグラウンドでの自動更新確認や自動ダウンロードを行いません（NFR-17、[ADR 0070](adr/0070-update-and-vulnerability-response.md) §1）。新しい版は次の方法で確認します:

1. **GitHub Releases と Security Advisories**:
   - リポジトリ（`https://github.com/ikari-zen/ikari-ime`）の「Watch」→「Custom」から「Releases」と「Security alerts」にチェックを入れておくと、新しい版やセキュリティ修正の通知を受け取れます（[ADR 0070](adr/0070-update-and-vulnerability-response.md) §3.1）。
2. **設定画面の案内と確認ボタン**（[ADR 0070](adr/0070-update-and-vulnerability-response.md) §3.2・§3.3）:
   - 設定画面の「このアプリについて」には、現在の版とビルド日付が表示されます。ビルド日付から一定期間（6 か月。暫定値）が過ぎると、通信を行わずに「新しい版が出ているかもしれません。Releases を見てください」と表示します。
   - 「新しい版を確認」ボタンを押したときだけ、取得の部品（`ikari-data.exe`）が GitHub Releases の最新の版の番号を確認します（自動での通信やダウンロードは行いません）（実機での確認は **（未確認）**）。

更新するときは、`https://github.com/ikari-zen/ikari-ime/releases` から新しい `ikari-ime-<版>-windows-x64-setup.exe` を取得し、既存の環境の上からそのまま管理者として実行します（先にアンインストールする必要はありません）。

### 3.2 TIP の DLL の更新の仕組みと、サインアウトが要る理由（ADR 0063）

Windows の入力メソッド（TIP）の DLL（`ikari_tip.dll`）は、文字入力を行うすべてのアプリのプロセス（`explorer.exe` や `WindowsTerminal.exe` などを含む）に読み込まれます。Windows では、プロセスが読み込んでいる最中の DLL ファイルを上書きすることができず、アプリを閉じても `explorer.exe` が DLL を持ち続けるため、同じパスへの上書き更新は失敗します（[ADR 0063](adr/0063-windows-tip-dll-update.md) §1.1、VM の項目 29 で確認済み）。

そこで ikari-ime のインストーラは、OS の再起動を強制せずに更新できるよう、次の順序で更新を行います（[ADR 0063](adr/0063-windows-tip-dll-update.md) §3、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §3.2、[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)。インストーラでの実機動作は項目 48・51・54 で **（未確認）**）:

1. **エンジンの停止と上書き**:
   - 動いているエンジン（`ikari-engine-host.exe`）および子プロセス（`ikari-lm-host.exe`、`ikari-candidate.exe`）にすべてのセッションで終了の合図を送り、学習データを保存して終了させてから、固定のパスの実行ファイルを新しい版で上書きします（**（未確認）**: 項目 51）。
2. **新しい版のフォルダへの配置と COM 登録の向け直し**:
   - 新しい `ikari_tip.dll`（と `resources\`）を、版ごとの新しいフォルダ `C:\Program Files\ikari-ime\tip\<新しい版>\` に置きます。
   - 既存の登録を解除せずに（入力方式の一覧から ikari が外れないようにするため）、新しい DLL に対して `regsvr32.exe /s` を実行し、`HKLM\Software\Classes\CLSID\...\InprocServer32` の参照先を新しい版のパスへ向け直します（**（未確認）**: 項目 29・48）。
   - ※同じ版のインストーラを入れ直す場合（修復など）で、既存の `tip\<版>\ikari_tip.dll` の SHA-256 が同じときは、使用中のファイルへの上書きエラーを避けるため DLL のコピーを飛ばして登録だけを行います（[ADR 0063](adr/0063-windows-tip-dll-update.md) §5、`ikari.iss` の `TipNeedsCopy`）（**（未確認）**: 項目 48）。
3. **古い版の `tip\<旧版>\` の掃除**:
   - 今の版より前の `C:\Program Files\ikari-ime\tip\<旧版>\` フォルダを削除します。開いているアプリが使用中でその場で消せなかったファイルは、次の OS 再起動時に削除するよう予約（`MOVEFILE_DELAY_UNTIL_REBOOT`）します（**（未確認）**: 項目 48）。

#### 更新後、いつ新しい版に切り替わるか

- **エンジン（`ikari-engine-host.exe`）・候補窓・LM の子**: 更新の直後（次の入力時）から新しい版が動きます。
- **TIP（`ikari_tip.dll`）**:
  - 更新のあとに**新しく開いたアプリ**からは、再起動なしで新しい版の `ikari_tip.dll` が読み込まれます（**（未確認）**: 項目 29・48）。
  - 更新の前から**すでに開いていたアプリ**（`explorer.exe` などを含む）は、そのアプリを閉じるかサインアウトするまで、メモリ上の古い版の `ikari_tip.dll` で動き続けます（**（未確認）**: 項目 29・48）。
  - 古い TIP と新しいエンジンが共存している間も、通信規約の版（`schema_version`）が同じであればそのまま変換できます。互換性を壊す変更で `schema_version` が上がった版では、サインアウトするまでの間、開いていた古い TIP のアプリは「エンジンなし（かな候補のみ）」で動き、固まったり落ちたりしない設計です（その場合はリリースノートに「サインアウトが要る」と明記します。[ADR 0063](adr/0063-windows-tip-dll-update.md) §3.6）（**（未確認）**: ADR 0063 §7 手順 7）。
  - **サインアウトしてサインインし直す**と、`explorer.exe` を含むすべてのアプリが新しい版の TIP に切り替わります（**（未確認）**: 項目 29・48）。

### 3.3 通常更新とセキュリティ更新での完了画面の案内（ADR 0070 §4）

インストーラは作業中のアプリを勝手に閉じず（`CloseApplications=no`）、サインアウトや再起動も強制しません。代わりに、更新の種類に応じて完了画面に次のどちらかの案内を出します（[ADR 0070](adr/0070-update-and-vulnerability-response.md) §4、[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）（**（未確認）**: 項目 54）。

| 更新の種類 | インストーラ完了画面の案内 | リリースノート |
| --- | --- | --- |
| **通常の更新** | `サインアウトすると全部のアプリで新しい版になります。` | 通常の変更履歴を記載 |
| **セキュリティの修正を含む更新**（`/DSecurityUpdate=1` でビルドされた版） | `この更新にはセキュリティの修正が入っています。サインアウトするまで、開いているアプリには修正が効きません。` | 先頭に、サインアウトが必要であることを明記 |

セキュリティの修正を含む更新を入れたときは、開いているアプリ（および `explorer.exe`）の中の古い TIP に修正を反映させるため、作業を保存して**サインアウト（または PC の再起動）**を行ってください。

### 3.4 古い版へ戻す（ダウングレードする）ときの注意

新しい版が入っている環境に古い版のインストーラを実行すると、次の確認画面が出ます（[ADR 0070](adr/0070-update-and-vulnerability-response.md) §3.4、[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）（**（未確認）**: 項目 54）:

> 新しい版（<インストール済みの版>）が入っています。古い版（<インストーラの版>）を入れますか。
>
> 新しい版が書いた学習のファイルを、古い版は読めず、エンジンが止まることがあります。

新しい版で学習データ（`%LOCALAPPDATA%\ikari-ime\learning.tsv` など）の形式が変わっていた場合、古い版のエンジンはそのファイルを読めずに停止することがあります（#275）。

---

## 4. アンインストール（削除）

### 4.1 アンインストールの手順

1. Windows の **設定** → **アプリ** → **インストールされているアプリ**（またはコントロールパネルの「プログラムと機能」）から **ikari-ime** を選び、**アンインストール** を実行します（管理者権限が必要です）（**（未確認）**: 項目 54）。
2. アンインストーラは導入時と逆の順序で次を行います（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss) の `UninstallInOrder`、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §3.2）（**（未確認）**: 項目 48〜54・71）:
   1. 自動起動（`HKLM\Software\Microsoft\Windows\CurrentVersion\Run` の `ikari-ime`）の削除
   2. Windows ファイアウォールの送信ブロック規則（`ikari-ime: block outbound ...`）の削除
   3. OS のエラー報告（WER）の除外登録の解除（`ikari-setup-helper.exe wer-include`）
   4. TIP の COM / TSF 登録の解除（`regsvr32.exe /u /s`）
   5. すべてのセッションのエンジンと子プロセスの停止（`ikari-setup-helper.exe stop`）
   6. アンインストールを実行した利用者の隔離用（AppContainer）プロファイル（`io.github.ikari-zen.lm-host` と `io.github.ikari-zen.data-transfer`）の削除（`ikari-setup-helper.exe cage-profile-delete`。[ADR 0072](adr/0072-lm-cage-levels.md) §12.8、[ADR 0078](adr/0078-fetch-child-handle-and-appcontainer.md) §3.2）（**（未確認）**: 項目 71）
   7. `C:\Program Files\ikari-ime` 配下のファイルの削除（`explorer.exe` などが使用中でその場で消せない `tip\<版>\ikari_tip.dll` 等は、次の OS 再起動時に削除するよう予約されます）
3. アンインストールの最後に、次の案内が表示されます（[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）（**（未確認）**: 項目 54）:
   > 学習と設定（各利用者の %LOCALAPPDATA%\ikari-ime）は残してあります。要らなければ、各利用者が自分で消してください。使用中で消せなかったファイルは、次の再起動で消えます。
4. Windows の入力方式の一覧（**設定** → **時刻と言語** → **言語と地域** → **日本語** → **言語のオプション**）から **ikari** が消えていることを確かめます（残っている場合は一覧から削除してください）（**（未確認）**: 項目 12・54）。
5. 使用中だった `C:\Program Files\ikari-ime\tip\` 配下のファイルを完全に消すには、Windows を再起動してください（すぐの再起動は必須ではありません）（**（未確認）**: 項目 54）。

### 4.2 既定でのデータの扱いと、削除の範囲

アンインストーラは、**利用者のデータ（各利用者の `%LOCALAPPDATA%\ikari-ime`）を一切削除せず、既定でそのまま残します**（[ADR 0063](adr/0063-windows-tip-dll-update.md) Q3、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §3.3・§7、[ADR 0073](adr/0073-secret-input-and-user-data.md) §3.5、[`windows/installer/ikari.iss`](../windows/installer/ikari.iss)）。

これには 2 つの理由があります:
- 入れ直しや不具合の切り分けの際に、これまでの学習データやユーザー辞書を誤って失わないようにするため。
- セキュリティの原則（[ADR 0066](adr/0066-security-premises.md) P1・R4、[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §2・§3.3）により、管理者権限で動くインストーラ／アンインストーラは、利用者が書き込めるフォルダ（`%LOCALAPPDATA%`）の中身を辿ったり削除したりしない設計としているため（ジャンクションやリンクによる権限昇格を防ぎ、また管理者アカウントと標準ユーザーのアカウントが異なる場合にも対応するため）。

#### アンインストール後に残るもの・消えるものの一覧

| 対象 | 場所 | アンインストール時の扱い | 完全に消したいときの手順 |
| --- | --- | --- | --- |
| プログラム本体 | `C:\Program Files\ikari-ime` | **削除される**（使用中の TIP の DLL などは次の OS 再起動時に削除） | OS を再起動する |
| 機械全体の設定（自動起動、ファイアウォール規則、WER 除外、COM 登録） | `HKLM`、Windows ファイアウォール | **削除される** | 自動で消える |
| 利用者のデータ（学習、ユーザー辞書、設定、辞書、モデル、ログ） | 各利用者の `%LOCALAPPDATA%\ikari-ime` | **残る（触れない）** | 要らなければ、**各利用者が自分の権限で**エクスプローラー等から `%LOCALAPPDATA%\ikari-ime` フォルダを削除する |
| 隔離（AppContainer）のプロファイル | 各利用者の `%LOCALAPPDATA%\Packages\` 配下（`io.github.ikari-zen.lm-host`、`io.github.ikari-zen.data-transfer`） | アンインストールを実行した利用者の分だけ**削除される**。**ほかの利用者の分は残る**（中身は空のプロファイル。[ADR 0072](adr/0072-lm-cage-levels.md) §12.8、[ADR 0078](adr/0078-fetch-child-handle-and-appcontainer.md) §3.2） | ほかの利用者の分は空のままで害はない（または各利用者で `C:\Program Files\ikari-ime\ikari-setup-helper.exe cage-profile-delete` をアンインストール前に実行） |
| VC++ 再頒布パッケージ（将来同梱された場合） | システム共有 | **残る**（ほかのアプリも使う共有部品のため。[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §4.5） | 通常は削除不要 |

#### 利用者のデータ（`%LOCALAPPDATA%\ikari-ime`）を手で消す方法

- **学習データを残したい（別の PC へ移したい）場合**:
  - アンインストールする**前**に、設定画面（`C:\Program Files\ikari-ime\settings\ikari-settings.exe`）から「学習を書き出す…」でファイルに書き出してください（**（未確認）**: 項目 67）。
  - ※書き出したファイルは利用者が選んだ場所に置かれ、語彙と選んだ表記が入っています（[ADR 0073](adr/0073-secret-input-and-user-data.md) §3.4）。保管場所に注意してください。
- **利用者のデータをすべて削除する場合**:
  1. アンインストールを完了させます（エンジンが動いているとファイルが使用中になるため）。
  2. エクスプローラーのアドレスバー（または `Win + R` の「ファイル名を指定して実行」）に `%LOCALAPPDATA%` と入力して開きます。
  3. その中にある `ikari-ime` フォルダ（`%LOCALAPPDATA%\ikari-ime`）を削除します。
  4. 同じ PC を複数のWindows ユーザーアカウントで使っていた場合は、それぞれのユーザーでサインインして同様に `%LOCALAPPDATA%\ikari-ime` を削除してください（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §7）。

---

## 5. 困ったときのログの場所と確認方法

### 5.1 ログと記録ファイルの場所

ikari-ime のデータとログは、すべて利用者の `%LOCALAPPDATA%\ikari-ime` の下に置かれます（[ADR 0040](adr/0040-windows-feasibility.md) §9、[ADR 0073](adr/0073-secret-input-and-user-data.md) §3・§4、[windows-real-device-check.md](windows-real-device-check.md) §3）。このフォルダには、本人・SYSTEM・Administrators だけがアクセスできる権限（ACL）が自動で設定されます（[ADR 0073](adr/0073-secret-input-and-user-data.md) §3.1）（実機での ACL 確認は項目 41 で **（未確認）**）。

エクスプローラーのアドレスバー、または `Win + R`（「ファイル名を指定して実行」）に次を入力すると、ログのフォルダを直接開けます:

```text
%LOCALAPPDATA%\ikari-ime\logs
```

| ファイルのパス | 内容 | Issue 等で共有するときの注意 |
| --- | --- | --- |
| `%LOCALAPPDATA%\ikari-ime\logs\engine.log` | **エンジンの診断ログ**（常時記録）。時刻（ms）、要求の種類（`convert`、`reconvert`、`commit`、`hello`、`lm_load`、`prev_exit` など）、所要時間（µs）、LM の状態（`lm=`）、エラーの種類（`err=`）、固定の語の注記だけが記録されます。**入力した文字、読み、候補、周辺テキストは一切記録されません**（NFR-19、[ADR 0034](adr/0034-diagnostic-log.md)）。また、Windows では `app=` にも既知の実行ファイル名だけを記録し、未知のアプリは `app=other` になります（[ADR 0073](adr/0073-secret-input-and-user-data.md) §4.1）。パスワード欄などの秘密の欄（secure）では行自体を書きません | 入力文字を含まないため、不具合報告の Issue に貼って構いません |
| `%LOCALAPPDATA%\ikari-ime\logs\engine-YYYYMMDD-HHMMSS.log.gz` | **回された古い診断ログ**。`engine.log` が 4 MB を超えるか日付が変わると gzip で圧縮され、180 日または合計 100 MB のどちらか先に来た方で古い順に自動削除されます（[ADR 0034](adr/0034-diagnostic-log.md) §3.2） | 同上（いつでも手で削除しても構いません） |
| `%LOCALAPPDATA%\ikari-ime\logs\start-failure.log` | **エンジンの起動失敗の記録**（最後の 1 回分）。データの場所を開けない、パイプを作れない、辞書が開けないなどの理由が固定の語で残ります（入力内容は含みません） | **注意**: ファイル内のエラーメッセージに、データのフォルダのパス（Windows のユーザー名を含むパス）が入ることがあります（[ADR 0073](adr/0073-secret-input-and-user-data.md) §4.6）。Issue に貼るときは、ユーザー名の部分を伏せて（消して）から貼ってください |
| `%LOCALAPPDATA%\ikari-ime\logs\engine.run` | **実行中と前回終了状態の印**（`runmark`、[ADR 0051](adr/0051-crash-exit-record.md)）。前回の終了が正常（`exit=clean`）、終了要求（`exit=term`）、異常終了（`exit=crash`）のどれだったかを次の起動時に判定し、`engine.log` の `prev_exit` 行に記録します | 通常は直接見る必要はありません（`engine.log` の `prev_exit` に出ます） |

#### 製品のビルドでは出力されないログ

- **インストーラのログ**: インストーラ（`ikari.iss`）は、管理者権限で利用者の `TEMP` にファイルを書き込む危険（[ADR 0066](adr/0066-security-premises.md) P1）を避けるため、既定ではログを出しません（`SetupLogging=no`）。インストーラの失敗を調べる必要があるときだけ、管理者のコマンドプロンプトから `/LOG="C:\..."`（管理者だけが書き込める場所の絶対パス）を手で指定して実行します。
- **TIP のデバッグ出力（`OutputDebugString`）と往復時間の計測ログ（`tip-latency.log`）**: 開発用のビルド（`dev-knobs` つき）でのみ出力され、配布する製品のビルドでは出力されません（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §6、[ADR 0073](adr/0073-secret-input-and-user-data.md) §4.2・§4.3）。

### 5.2 よくある症状と確認方法

#### 1. ひらがな・カタカナしか出ず、漢字に変換されない

ikari-ime は、変換エンジン（`ikari-engine-host.exe`）につながらないときや辞書が開けないときでも、入力を止めないために TIP 単体で「かな候補（ひらがな・カタカナ）」を出す仕組みになっています（#329、[ADR 0062](adr/0062-windows-candidate-window.md) §5.4）。次を順に確かめてください。

1. **入力しているアプリの種類を確かめる**:
   - 「管理者として実行」したアプリや、Microsoft Store のアプリ（AppContainer）では、1.0.0 は安全のため**素通し**（変換しない）になります（[ADR 0067](adr/0067-windows-install-layout-and-launch-chain.md) §5）（**（未確認）**: 項目 42）。メモ帳などのふつうのデスクトップアプリで変換できるかを確かめてください。
2. **更新の直後かどうかを確かめる**:
   - ikari-ime を更新した直後で、通信規約の版（`schema_version`）が上がった場合は、更新前から開いていたアプリではサインアウトするまでかな候補のみになります（[ADR 0063](adr/0063-windows-tip-dll-update.md) §3.6）。一度サインアウトしてサインインし直してください。
3. **エンジンが動いているかと、ログを確かめる**:
   - タスクマネージャー（または PowerShell の `Get-Process ikari-engine-host`）で、`ikari-engine-host.exe` が動いているかを見ます。
   - `%LOCALAPPDATA%\ikari-ime\logs\start-failure.log` と `%LOCALAPPDATA%\ikari-ime\logs\engine.log` の末尾を確認します。
   - `%LOCALAPPDATA%\ikari-ime\dict\` に辞書ファイル（`grammar.bin`、`system.dict`、`vectors.bin`）が入っているかを確かめます（辞書がない場合、エンジンは常駐したまま `start-failure.log` に理由を残し、辞書が置かれると変換に戻ります。項目 3 で確認済み）。

#### 2. パスワード欄や端末で変換や学習を止めたい（秘密の入力の限界）

Windows 版の TIP は、入力欄に付いている入力スコープ（`IS_PASSWORD`、PIN、Chromium 系ブラウザの `IS_PRIVATE`）と「キーボードが無効」の印を見て秘密の欄（secure）を見分け、変換も記録も学習も行わないようにしています（[ADR 0073](adr/0073-secret-input-and-user-data.md) §1・§2）（実機での各欄の確認は [windows-real-device-check.md](windows-real-device-check.md) の項目 32 で **（未確認）**）。

ただし、OS やアプリの仕組み上、**次の欄は秘密の欄として見分けられません**（[ADR 0073](adr/0073-secret-input-and-user-data.md) §2.4）:
- 入力スコープを付けない独自描画の入力欄（一部の古いアプリやゲームなど）
- 端末（Windows Terminal、PowerShell、コマンドプロンプトなど）の中のパスワード入力（`ssh`、`sudo` など）
- リモートデスクトップや仮想マシンのウィンドウの中の入力欄
- ブラウザのプライベートウィンドウ内の通常の入力欄（パスワード欄以外の検索欄など）

これらの欄で秘密の文字列を打つときは、英語のモードや他の入力方式に切り替えて入力してください（なお、英語のモードでも、見分けられない欄で打った文字はエンジンのメモリ内の文脈には入りますが、学習ファイルやログには保存されません。[ADR 0073](adr/0073-secret-input-and-user-data.md) §2.5）。

#### 3. インストーラがエラーメッセージを出して止まる

[`windows/installer/ikari.iss`](../windows/installer/ikari.iss) が表示する主なエラーと対処方法は次のとおりです（実機での表示は項目 47・51・54 で **（未確認）**）:

| メッセージ | 原因と対処 |
| --- | --- |
| `管理者として実行してください。` | インストーラに管理者権限がありません。右クリックして「管理者として実行」するか、UAC の画面で許可してください |
| `インストール先は C:\Program Files\ikari-ime に固定です（変えられません）。` | `/DIR=` などで別のインストール先が指定されています。引数を外して実行してください |
| `インストール先（C:\Program Files\ikari-ime）に、管理者以外が書き込めるファイルかフォルダがあります。安全のため、登録せずに終わります。フォルダを消してから、もう一度実行してください。`（終了コード 10） | `C:\Program Files\ikari-ime` のアクセス権（ACL）または所有者が安全な状態（管理者・SYSTEM・TrustedInstaller のみ書き込み可）になっていません。管理者権限で `C:\Program Files\ikari-ime` フォルダを削除してから、もう一度インストーラを実行してください |
| `動いているエンジンを止めきれませんでした。ファイルを置き換えられないことがあります。` | 更新時に既存の `ikari-engine-host.exe` 等が終了しませんでした。サインアウトしてサインインし直すか、タスクマネージャーから終了させてから、もう一度インストーラを実行してください |

終了コード 10 や 11 で止まったとき（画面に「次の手順に失敗しました」などと出たとき）は、次の「5.3 インストールに失敗したとき」に進んでください。

### 5.3 インストールに失敗したとき

インストーラは、途中で失敗すると、そこで止まります。**元の状態への自動の巻き戻しはしません**（作るかどうかは、まだ決めていません）。止まり方は 2 通りで、インストーラの終了コード（コマンドから実行したときの戻り値。PowerShell なら実行の直後に `$LASTEXITCODE` を見る）で分かります。画面で実行したときは、最後の画面に、止まった手順が出ます。

以下の「状態を確かめるコマンド」は、**管理者の PowerShell** で実行します。手順は、2026-11-23 に Windows 11 の VM で確かめました（[windows-real-device-check.md](windows-real-device-check.md) の項目 79〜81）。確かめたのはコマンドからの実行（`/VERYSILENT`）です。画面の表示と、「設定」の「インストールされているアプリ」からの削除の操作は、**未確認**です（同じ削除の処理を、`unins000.exe` から実行して確かめました）。

#### 状態を確かめるコマンド

```powershell
# 入っている版（削除の入口。無ければ「見つかりません」と出る）
reg query "HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{1CE51AC3-4849-442B-BFBE-36FA3EEC855B}_is1" /v DisplayVersion
# 置いてある TIP の版
Get-ChildItem "C:\Program Files\ikari-ime\tip" | Select-Object -ExpandProperty Name
# 登録（行が出れば登録されている。出なければ登録されていない）
reg query "HKLM\Software\Classes\CLSID" /s /f ikari_tip.dll
# 自動起動
reg query HKLM\Software\Microsoft\Windows\CurrentVersion\Run /v ikari-ime
# ファイアウォールの規則（正常なら 4 行）
netsh advfirewall firewall show rule name=all | Select-String "ikari-ime: block outbound"
# エラー報告の除外（正常なら 4 つの exe が出る）
reg query "HKLM\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications"
```

正常に入ったあとは、上から順に「版が出る」「`tip\<版>` が 1 つ」「登録の行が `tip\<版>\ikari_tip.dll` を指す」「自動起動の行が出る」「規則が 4 行」「除外の exe が 4 つ」になります。

#### (a) 終了コード 10: インストール先の検査に落ちた

- **どうなっているか**: `C:\Program Files\ikari-ime` に、管理者以外が書き込めるファイルやフォルダがあったので、安全のため何もせずに止まりました。新しいファイルは置かれず、登録・規則・自動起動・エラー報告の除外も書かれません。「プログラムの追加と削除」の入口（削除の入口）も作られません。更新のときは、前の版がそのまま残ります（VM で確認）。
- **直し方**:
  1. 新しく入れるとき（前の版が無いとき）は、管理者の権限で `C:\Program Files\ikari-ime` フォルダを削除します。
  2. 更新のとき（前の版が入っているとき）は、削除の前に、前の版を使い続けるか決めます。そのまま使い続けるなら、何もしなくてかまいません。入れ直すなら、先に「設定」→「アプリ」から ikari-ime を削除し、残ったフォルダがあれば削除します。
  3. もう一度インストーラを実行します。
- VM では、`C:\Program Files\ikari-ime` に一般のユーザーの書き込みを許した状態で入れると終了コード 10 になり、フォルダを削除してから同じインストーラを実行すると、終了コード 0 で正常に入りました。

#### (b) 終了コード 11: 置いたあとの手順に失敗した

ファイルを置いたあと、「登録（`DllRegisterServer`）」「ファイアウォールの規則」「自動起動」「エラー報告の除外」を順に行い、どれかが失敗した時点で止まります。後ろの手順は行われません。止まった手順の名前は、最後の画面（「次の手順に失敗しました: …」）に出ます。置いたファイルと、削除の入口は、**残ります**。

どちらの場合も、原因が残っている（たとえば、ほかのセキュリティ製品が登録や規則を止めている）と、同じ手順でまた失敗します。原因を直してから、次の手順を行ってください。

##### 新しく入れたとき（前の版が無かったとき）

- **どうなっているか**: 新しい版のファイルは置かれ、「プログラムの追加と削除」には新しい版が出ます。ただし、止まった手順より後ろは行われていないので、ikari は使えない状態です（VM で、登録の手順で止めたところ、登録・規則・自動起動・エラー報告の除外のどれも書かれていませんでした）。
- **先へ進める（推奨）**: 原因を直してから、**同じ版のインストーラをもう一度実行**します（上書きできます）。VM では、失敗した版の上に、正常な同じ版のインストーラを実行すると、終了コード 0 で、登録・規則 4 つ・自動起動・エラー報告の除外がすべてそろいました。
- **最初からやり直す**: 「設定」→「アプリ」→「インストールされているアプリ」の ikari-ime から削除し、そのあと、インストーラを実行します。VM では、失敗した状態で `C:\Program Files\ikari-ime\unins000.exe /VERYSILENT` を実行すると終了コード 0 で削除でき（`C:\Program Files\ikari-ime` も消えました）、続けて正常なインストーラを実行すると正常に入りました。
- 状態の確認には、上のコマンドを使います。

##### 更新のとき（前の版が入っていたとき）

- **どうなっているか**: 前の版の `tip\<前の版>` と、登録（前の版を指したまま）は**消えずに残る**ので、前の版の ikari をそのまま使えると見込まれます（VM で、前の版のフォルダと、前の版を指したままの登録を確認しました。実際に入力して確かめたものではありません）。自動起動、規則、エラー報告の除外も前の版のままです。一方、「プログラムの追加と削除」の版の表示は新しい版になり、新しい版のファイルと、新しい版の `tip\<新しい版>` は置かれたままです（登録されていません）。
- **新しい版へ進める**: 原因を直してから、**新しい版のインストーラをもう一度実行**します。VM では、終了コード 0 になり、`tip\` は新しい版だけになり、登録は新しい版を指しました。
- **前の版に戻したいとき**: 自動では戻りません。前の版のインストーラがあれば、実行して上書きします（版が古いので、確かめの画面が出ます。ダウングレードの注意は 3.4 を参照）。前の版のインストーラの上書きは、この手順としては **未確認**です。

#### 学習と設定は消えません

どの手順でも、各利用者の `%LOCALAPPDATA%\ikari-ime`（学習と設定）には触れません。

#### インストーラのログについて

インストーラは、既定ではログを出しません（5.1 を参照）。原因を調べるときだけ、管理者のコマンドプロンプトから `/LOG="<管理者だけが書き込める場所の絶対パス>"` を付けて実行します（この指定でのログの出力は、今回の手順では **未確認**です）。

---

## 6. 実機で未確認の項目（確認後に更新する一覧）

この下書きのうち、Windows 11 の実機（VM）でまだ通し確認を行っていない項目の一覧です。確認の手順は [windows-real-device-check.md](windows-real-device-check.md) にあります。

1. **未署名のインストーラと DLL に対する OS の警告画面（項目 31、52）**:
   - SmartScreen の実際の警告画面・文言・進める操作。
   - Smart App Control が有効／評価モードの環境でのインストーラの起動可否と、TIP の DLL の読み込み可否。
2. **インストーラによる新規導入・更新・アンインストールの一連の動作（項目 47〜55、71）**:
   - インストール先（`C:\Program Files\ikari-ime`）の ACL 検査と、違反時に登録せず終了コード 10 で止まること（項目 47）。
   - `tip\<版>\ikari_tip.dll` の登録、上書き更新時の登録の向け直し、開いているアプリと新しく開くアプリでの版の切り替わり、再起動後の古い版の削除（項目 29、48）。
   - Windows ファイアウォールの送信ブロック規則の登録・更新・削除と、実機での送信ゼロの確認（項目 49、57）。
   - `HKLM\Software\Microsoft\Windows\CurrentVersion\Run` によるログオン時の自動起動と、導入直後にサインアウトせず入力したときの TIP からのエンジン起動（項目 50）。
   - 更新・削除時のインストーラ（`ikari-setup-helper.exe stop`）からの全セッションのエンジン停止と学習の保存（項目 51）。
   - OS のエラー報告（WER）からの除外登録と、クラッシュ時にレポートが作られないこと（項目 53）。
   - 新規導入・通常更新・セキュリティ更新・ダウングレード・削除のそれぞれの完了／確認画面の表示と、削除後に `%LOCALAPPDATA%\ikari-ime` が残ること（項目 54）。
   - インストーラ実行時の一時フォルダ（`C:\Program Files\ikari-ime.setup` と Inno Setup 自身の一時ファイル）の挙動（項目 55）。
   - アンインストール時の AppContainer プロファイル削除（`cage-profile-delete`）（項目 71）。
3. **権限・欄の種類ごとの動き（項目 32、41、42、56〜70）**:
   - `%LOCALAPPDATA%\ikari-ime` の ACL の自動設定（項目 41）。
   - 高い権限のアプリ・ストアのアプリ・サインイン画面での TIP の素通し動作（項目 42）。
   - パスワード欄・PIN 欄・ブラウザの `type=password` 欄での secure 判定の実機確認（項目 32）。
   - 設定画面（WinUI 3）からの設定変更、学習データの書き出し・取り込み、モデルの取得と版の確認（項目 56、67、68、70）。
