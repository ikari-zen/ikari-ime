; ikari-ime の Windows のインストーラ（Inno Setup）。骨組み（レーン C-2）。
;
; 設計: docs/security-design.md §8.4、ADR 0067 §3・§4、ADR 0068 §6.2、ADR 0070 §4、ADR 0063、ADR 0073 §4、ADR 0069 §3。
; 判断の置き場所: 順番と画面はここ（Pascal）。置き場所の検査、エンジンを止める、エラー報告の除外は
; ikari-setup-helper.exe（Rust。windows/installer/helper）。理由は helper/src/lib.rs の冒頭。
;
; 作り方（Windows で。手順は windows/installer/README.md）:
;   ISCC /DAppVersion=0.1.0 /DStageDir=<配置の形に並べたフォルダ> /DTipSha256=<ikari_tip.dll の sha256> [/DSecurityUpdate=1] ikari.iss
;
; 守ること（cargo xtask check が、この文書の文字列を検査する。xtask/src/check/installer.rs）:
;   - インストール先は固定。利用者ごとのもの（HKCU、利用者のフォルダ、入力方式の一覧）に触れない（ADR 0067 §3.3）
;   - Inno 自身の一時の場所に、こちらのファイルを展開しない（利用者の TEMP を信用しない）。道具は Program Files の
;     下に作る作業用のフォルダに置く（管理者だけが書ける。親から継承した ACL）
;   - 外のプログラムは RunTrusted の 1 か所からだけ、絶対パスで起動する（system32 の道具と helper だけ）
;   - DLL の登録は Program Files に置いたものに対して、明示の regsvr32 で行う（regserver の印は使わない）

#if Ver < EncodeVer(6, 6, 0)
  #error Inno Setup 6.6.0 以上が要る（一時の場所の名前を CSPRNG で作る、SYSTEM のときの SystemTemp）。推奨は 6.7.4 以上（RedirectionGuard が既定）
#endif
#ifndef AppVersion
  #error /DAppVersion=<版> を渡す（例 0.1.0）
#endif
#ifndef StageDir
  #error /DStageDir=<配置の形に並べたフォルダ> を渡す
#endif
#ifndef TipSha256
  #error /DTipSha256=<StageDir\tip\ikari_tip.dll の sha256> を渡す（同じ版の入れ直しで、使用中の DLL を置き直さないため。ADR 0063 §5）
#endif
#ifndef SecurityUpdate
  ; 1 にすると「セキュリティの修正を含む更新」の案内になる（ADR 0070 §4 の 1。版を出すときに作者が決める）
  #define SecurityUpdate 0
#endif

[Setup]
; AppId は変えない（同じ製品の更新と削除の鍵）
AppId={{1CE51AC3-4849-442B-BFBE-36FA3EEC855B}
AppName=ikari-ime
AppVersion={#AppVersion}
AppPublisher=ikari-zen
AppPublisherURL=https://github.com/ikari-zen/ikari-ime
AppSupportURL=https://github.com/ikari-zen/ikari-ime/issues
AppUpdatesURL=https://github.com/ikari-zen/ikari-ime/releases
VersionInfoVersion={#AppVersion}
; 管理者で動く。利用者ごとの導入には切り替えさせない（PrivilegesRequiredOverridesAllowed を書かない）
PrivilegesRequired=admin
; インストール先は固定（ADR 0067 §3.1）。/DIR= で変えられても PrepareToInstall で断る
DefaultDirName={commonpf64}\ikari-ime
DisableDirPage=yes
UsePreviousAppDir=no
DisableProgramGroupPage=yes
DisableReadyPage=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; 使用中のファイルを持つアプリを閉じさせない（TIP の DLL は explorer などが持つ。ADR 0063 §3 の 3）
CloseApplications=no
RestartApplications=no
; ログは管理者の権限で、利用者の TEMP に書かれるので出さない（P1。調べるときは /LOG=<管理者だけが書ける場所> を手で渡す）
SetupLogging=no
WizardStyle=modern
OutputDir=out
OutputBaseFilename=ikari-ime-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
Uninstallable=yes
UninstallDisplayName=ikari-ime
LicenseFile={#StageDir}\LICENSE

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
japanese.NotAdmin=管理者として実行してください。
english.NotAdmin=Run this installer as an administrator.
japanese.FixedDir=インストール先は %1 に固定です（変えられません）。
english.FixedDir=The install location is fixed to %1 and cannot be changed.
japanese.WorkDirFailed=作業用のフォルダ（%1）を作れませんでした。
english.WorkDirFailed=Could not create the working folder (%1).
japanese.Untrusted=インストール先（%1）に、管理者以外が書き込めるファイルかフォルダがあります。安全のため、登録せずに終わります。フォルダを消してから、もう一度実行してください。
english.Untrusted=The install location (%1) contains files or folders that non-administrators can write to. For safety, setup stops without registering ikari. Delete the folder and run setup again.
japanese.StepFailed=次の手順に失敗しました: %1
english.StepFailed=This step failed: %1
japanese.EnginesRemain=動いているエンジンを止めきれませんでした。ファイルを置き換えられないことがあります。
english.EnginesRemain=Some running engines could not be stopped. Files may not be replaced.
japanese.Downgrade=新しい版（%1）が入っています。古い版（%2）を入れますか。%n%n新しい版が書いた学習のファイルを、古い版は読めず、エンジンが止まることがあります。
english.Downgrade=A newer version (%1) is installed. Install the older version (%2)?%n%nThe older version may not read learning files written by the newer one, and the engine may stop.
japanese.OtherVersion=インストール済みの版: %1
english.OtherVersion=Installed version: %1
japanese.SmartAppControl=この PC では Smart App Control が有効です。ikari には署名がまだ無いので、署名が付くまで、この PC では ikari を使えません。
english.SmartAppControl=Smart App Control is on for this PC. ikari is not signed yet, so it cannot be used on this PC until it is signed.
japanese.FinishedFresh=入力方式に ikari を足してください（設定 → 時刻と言語 → 言語と地域 → 日本語 → 言語のオプション → キーボードの追加）。インストーラは入力方式の一覧を変えません。
english.FinishedFresh=Add ikari as an input method (Settings → Time & language → Language & region → Japanese → Language options → Add a keyboard). Setup does not change your input method list.
japanese.FinishedUpdate=サインアウトすると全部のアプリで新しい版になります。
english.FinishedUpdate=Sign out to use the new version in all apps.
japanese.FinishedSecurityUpdate=この更新にはセキュリティの修正が入っています。サインアウトするまで、開いているアプリには修正が効きません。
english.FinishedSecurityUpdate=This update contains security fixes. The fixes do not apply to apps that are already open until you sign out.
japanese.FinishedFailed=インストールは完了していません。%1
english.FinishedFailed=Setup did not complete. %1
japanese.UninstallDataKept=学習と設定（各利用者の %LOCALAPPDATA%\ikari-ime）は残してあります。要らなければ、各利用者が自分で消してください。使用中で消せなかったファイルは、次の再起動で消えます。
english.UninstallDataKept=Learning data and settings (%LOCALAPPDATA%\ikari-ime for each user) were kept. Each user can delete them if no longer needed. Files that were in use are removed at the next restart.

[Files]
; 1. 道具を、作業用のフォルダ（Program Files の下に、このインストーラが新しく作る）に置く。以後の検査と停止はこれで行う
Source: "{#StageDir}\ikari-setup-helper.exe"; DestDir: "{code:SetupWorkDir}"; Flags: ignoreversion
; 2. インストール先。最初の項目の Check（AppDirReady）で、置き場所の検査とエンジンの停止を先に行う（ADR 0067 §3.2 の 2・3）
Source: "{#StageDir}\ikari-setup-helper.exe"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion
Source: "{#StageDir}\ikari-engine-host.exe"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion
Source: "{#StageDir}\ikari-lm-host.exe"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion
Source: "{#StageDir}\*.dll"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#StageDir}\backends\*"; DestDir: "{app}\backends"; Check: AppDirReady; Flags: ignoreversion recursesubdirs skipifsourcedoesntexist
; 候補窓（ADR 0062。W3 でできるまで無い）
Source: "{#StageDir}\candidate\*"; DestDir: "{app}\candidate"; Check: AppDirReady; Flags: ignoreversion recursesubdirs skipifsourcedoesntexist
; TIP は版ごとのフォルダ（ADR 0063 §3 の 1）。同じ版で中身が同じなら置き直さない（使用中で上書きできない。§5）
Source: "{#StageDir}\tip\ikari_tip.dll"; DestDir: "{app}\tip\{#AppVersion}"; Check: TipNeedsCopy; Flags: ignoreversion uninsrestartdelete
Source: "{#StageDir}\tip\resources\*"; DestDir: "{app}\tip\{#AppVersion}\resources"; Check: TipNeedsCopy; Flags: ignoreversion recursesubdirs uninsrestartdelete
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_LICENSES_RUST.md"; DestDir: "{app}"; Check: AppDirReady; Flags: ignoreversion skipifsourcedoesntexist

[Code]
const
  HelperExe = 'ikari-setup-helper.exe';
  EngineExe = 'ikari-engine-host.exe';
  TipDllName = 'ikari_tip.dll';
  { ikari-setup-helper の終了コード（helper/src/lib.rs の exit と同じ値。xtask が照らす） }
  HelperOk = 0;
  HelperUntrusted = 2;
  HelperRemaining = 3;
  { このインストーラの終了コード（GetCustomSetupExitCode。0 は成功） }
  ExitUntrusted = 10;
  ExitStepFailed = 11;
  { 自動起動（ADR 0067 §4.1。HKLM の Run。値は引用符つきの絶対パス、引数なし） }
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValue = 'ikari-ime';
  { ファイアウォールの規則の名前の前置き。版を入れない（入れ直しで重ならない。ADR 0067 §3.2 の 5） }
  FirewallRulePrefix = 'ikari-ime: block outbound ';
  { Inno の削除の情報（AppId と _is1） }
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{1CE51AC3-4849-442B-BFBE-36FA3EEC855B}_is1';
  { Smart App Control の状態（1 = 有効）。TODO(ADR 0069 §9): 値の場所と意味は実機で確かめる（項目 52） }
  SacPolicyKey = 'SYSTEM\CurrentControlSet\Control\CI\Policy';
  SacPolicyValue = 'VerifiedAndReputablePolicyState';
  SacEnforced = 1;
  AttrDirectory = $10;

var
  InstallStarted: Boolean;
  WorkDirCreated: Boolean;
  AppDirPrepared: Boolean;
  AppDirTrusted: Boolean;
  TipChecked: Boolean;
  TipCopy: Boolean;
  IsUpdate: Boolean;
  FailureCode: Integer;
  FailureText: String;

{ ---- 場所 ---- }

function ExpectedAppDir: String;
begin
  Result := ExpandConstant('{commonpf64}\ikari-ime');
end;

{ 作業用のフォルダ。Program Files の直下（管理者だけが作れる・書ける）。利用者の TEMP は使わない }
function WorkDir: String;
begin
  Result := ExpandConstant('{commonpf64}\ikari-ime.setup');
end;

function SetupWorkDir(Param: String): String;
begin
  Result := WorkDir;
end;

function WorkHelper: String;
begin
  Result := WorkDir + '\' + HelperExe;
end;

function AppHelper: String;
begin
  Result := ExpectedAppDir + '\' + HelperExe;
end;

function SysTool(Name: String): String;
begin
  Result := ExpandConstant('{sys}\') + Name;
end;

function TipDll: String;
begin
  Result := ExpectedAppDir + '\tip\{#AppVersion}\' + TipDllName;
end;

{ ファイアウォールの規則とエラー報告の除外を入れる exe（インストール先からの相対。ADR 0066 §6.2）。
  helper の ENGINE_EXE と CHILD_EXES と同じ並び（xtask が照らす）。
  TODO(ADR 0071): 設定画面の exe の名前が決まったら足す。 }
function BlockedExe(I: Integer): String;
begin
  case I of
    0: Result := 'ikari-engine-host.exe';
    1: Result := 'ikari-lm-host.exe';
    2: Result := 'candidate\ikari-candidate.exe';
  else
    Result := '';
  end;
end;

{ ---- 外のプログラム（ここだけで起動する） ---- }

function RunTrusted(Exe, Params: String; var Code: Integer): Boolean;
begin
  Result := Exec(Exe, Params, ExtractFileDir(Exe), SW_HIDE, ewWaitUntilTerminated, Code);
end;

procedure Fail(Code: Integer; Text: String);
begin
  if FailureCode = 0 then
  begin
    FailureCode := Code;
    FailureText := Text;
  end;
  SuppressibleMsgBox(Text, mbError, MB_OK, IDOK);
end;

function StepOk(Ok: Boolean; Code: Integer; Step: String): Boolean;
begin
  Result := Ok and (Code = 0);
  if not Result then
    Fail(ExitStepFailed, FmtMessage(CustomMessage('StepFailed'), [Step]));
end;

{ ---- 起動の前 ---- }

function InstalledVersion: String;
begin
  if not RegQueryStringValue(HKLM64, UninstallKey, 'DisplayVersion', Result) then
    Result := '';
end;

function InitializeSetup: Boolean;
var
  Installed: String;
  InstalledPacked, ThisPacked: Int64;
  Sac: Cardinal;
begin
  Result := True;
  if not IsAdmin then
  begin
    SuppressibleMsgBox(CustomMessage('NotAdmin'), mbError, MB_OK, IDOK);
    Result := False;
    exit;
  end;
  { Smart App Control（ADR 0069 §3 の 5。切る手順は案内しない） }
  if RegQueryDWordValue(HKLM64, SacPolicyKey, SacPolicyValue, Sac) and (Sac = SacEnforced) then
    if SuppressibleMsgBox(CustomMessage('SmartAppControl'), mbInformation, MB_OKCANCEL, IDOK) = IDCANCEL then
    begin
      Result := False;
      exit;
    end;
  { ほかの版が入っていれば出す。古い版を上に入れるときは確かめる（ADR 0070 §3 の 4。止めはしない） }
  Installed := InstalledVersion;
  IsUpdate := Installed <> '';
  if IsUpdate and StrToVersion(Installed, InstalledPacked) and StrToVersion('{#AppVersion}', ThisPacked) then
  begin
    if ComparePackedVersion(InstalledPacked, ThisPacked) > 0 then
      Result := SuppressibleMsgBox(FmtMessage(CustomMessage('Downgrade'), [Installed, '{#AppVersion}']),
        mbConfirmation, MB_YESNO, IDNO) = IDYES;
  end;
end;

procedure InitializeWizard;
begin
  if IsUpdate then
    WizardForm.WelcomeLabel2.Caption := WizardForm.WelcomeLabel2.Caption + #13#10#13#10 +
      FmtMessage(CustomMessage('OtherVersion'), [InstalledVersion]);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if CompareText(ExpandConstant('{app}'), ExpectedAppDir) <> 0 then
  begin
    Result := FmtMessage(CustomMessage('FixedDir'), [ExpectedAppDir]);
    exit;
  end;
  { 作業用のフォルダは毎回作り直す（前の失敗の残りを使わない）。新しく作れたときだけ、親の ACL を継承している }
  if DirExists(WorkDir) then
    DelTree(WorkDir, True, True, True);
  if not CreateDir(WorkDir) then
  begin
    Result := FmtMessage(CustomMessage('WorkDirFailed'), [WorkDir]);
    exit;
  end;
  WorkDirCreated := True;
end;

{ ---- インストール先の準備（最初の {app} の項目の Check から 1 回だけ） ---- }

procedure StopEngines(Helper: String);
var
  Code: Integer;
begin
  { 止める口に合図し、待ち、残れば強制の終了（helper の中。ADR 0068 §6.2）。すべてのセッションが対象 }
  if not RunTrusted(Helper, 'stop "' + ExpectedAppDir + '"', Code) or (Code = HelperRemaining) then
    SuppressibleMsgBox(CustomMessage('EnginesRemain'), mbInformation, MB_OK, IDOK);
end;

function CheckAppDir(Helper: String): Boolean;
var
  Code: Integer;
begin
  Result := RunTrusted(Helper, 'check-dir "' + ExpectedAppDir + '"', Code) and (Code = HelperOk);
end;

function AppDirReady: Boolean;
begin
  { インストールの前に呼ばれたときは何もしない（ディスクの見積もりなど） }
  if not InstallStarted then
  begin
    Result := True;
    exit;
  end;
  if not AppDirPrepared then
  begin
    AppDirPrepared := True;
    { 既にあるときも、同じ検査を先に行う（ADR 0067 §3.1）。無ければ、Program Files から継承して作られる }
    AppDirTrusted := (not DirExists(ExpectedAppDir)) or CheckAppDir(WorkHelper);
    if AppDirTrusted then
      StopEngines(WorkHelper)
    else
      Fail(ExitUntrusted, FmtMessage(CustomMessage('Untrusted'), [ExpectedAppDir]));
  end;
  Result := AppDirTrusted;
end;

function TipNeedsCopy: Boolean;
begin
  Result := AppDirReady;
  if not Result or not InstallStarted then
    exit;
  if not TipChecked then
  begin
    TipChecked := True;
    TipCopy := True;
    if FileExists(TipDll) then
      try
        TipCopy := CompareText(GetSHA256OfFile(TipDll), '{#TipSha256}') <> 0;
      except
        TipCopy := True;
      end;
  end;
  Result := TipCopy;
end;

{ ---- 置いたあと（ADR 0067 §3.2 の 3〜7 の順） ---- }

function VerifyAppDir: Boolean;
begin
  { 最後に確かめる（ADR 0067 §3.1）。満たさなければ登録しない }
  Result := CheckAppDir(WorkHelper);
  if not Result then
    Fail(ExitUntrusted, FmtMessage(CustomMessage('Untrusted'), [ExpectedAppDir]));
end;

procedure RegisterTip;
var
  Code: Integer;
begin
  { Program Files に置いた DLL の DllRegisterServer。更新では解除しない（ADR 0063 §3 の 2） }
  StepOk(RunTrusted(SysTool('regsvr32.exe'), '/s "' + TipDll + '"', Code), Code, 'DllRegisterServer');
end;

function FirewallRuleName(I: Integer): String;
begin
  Result := FirewallRulePrefix + ExtractFileName(BlockedExe(I));
end;

procedure RemoveFirewallRules;
var
  I, Code: Integer;
begin
  I := 0;
  while BlockedExe(I) <> '' do
  begin
    { 無ければ失敗で返るが、それでよい }
    RunTrusted(SysTool('netsh.exe'), 'advfirewall firewall delete rule name="' + FirewallRuleName(I) + '"', Code);
    I := I + 1;
  end;
end;

procedure AddFirewallRules;
var
  I, Code: Integer;
begin
  { 送信をブロックする規則を exe のパスごとに（ADR 0066 §6.2 の L2）。同じ名前を消してから入れる }
  RemoveFirewallRules;
  I := 0;
  while BlockedExe(I) <> '' do
  begin
    StepOk(RunTrusted(SysTool('netsh.exe'),
      'advfirewall firewall add rule name="' + FirewallRuleName(I) + '" dir=out action=block profile=any enable=yes program="' +
      ExpectedAppDir + '\' + BlockedExe(I) + '"', Code), Code, FirewallRuleName(I));
    I := I + 1;
  end;
end;

procedure AddAutostart;
begin
  StepOk(RegWriteStringValue(HKLM64, RunKey, RunValue, '"' + ExpectedAppDir + '\' + EngineExe + '"'), 0, 'Run');
end;

procedure RemoveAutostart;
begin
  if RegValueExists(HKLM64, RunKey, RunValue) then
    RegDeleteValue(HKLM64, RunKey, RunValue);
end;

function ErrorReportingNames: String;
var
  I: Integer;
begin
  Result := '';
  I := 0;
  while BlockedExe(I) <> '' do
  begin
    Result := Result + ' ' + ExtractFileName(BlockedExe(I));
    I := I + 1;
  end;
end;

procedure ExcludeFromErrorReporting;
var
  Code: Integer;
begin
  { OS のエラー報告から外す（機械全体。ADR 0073 §4。WerAddExcludedApplication(名前, TRUE) を helper が呼ぶ） }
  StepOk(RunTrusted(WorkHelper, 'wer-exclude' + ErrorReportingNames, Code), Code, 'WerAddExcludedApplication');
end;

procedure IncludeInErrorReporting;
var
  Code: Integer;
begin
  RunTrusted(AppHelper, 'wer-include' + ErrorReportingNames, Code);
end;

{ 消せないもの（使用中）は、次の再起動で消す予約をする。管理者だけが書ける場所（検査済みのインストール先）にだけ使う }
procedure ScheduleDeleteTree(Dir: String);
var
  Rec: TFindRec;
begin
  if FindFirst(Dir + '\*', Rec) then
  try
    repeat
      if (Rec.Name <> '.') and (Rec.Name <> '..') then
      begin
        if (Rec.Attributes and AttrDirectory) <> 0 then
          ScheduleDeleteTree(Dir + '\' + Rec.Name)
        else
          RestartReplace(Dir + '\' + Rec.Name, '');
      end;
    until not FindNext(Rec);
  finally
    FindClose(Rec);
  end;
  RestartReplace(Dir, '');
end;

procedure RemoveTipVersionsExcept(Keep: String);
var
  Rec: TFindRec;
  Root, Dir: String;
begin
  Root := ExpectedAppDir + '\tip';
  if FindFirst(Root + '\*', Rec) then
  try
    repeat
      if ((Rec.Attributes and AttrDirectory) <> 0) and (Rec.Name <> '.') and (Rec.Name <> '..') and
        (CompareText(Rec.Name, Keep) <> 0) then
      begin
        Dir := Root + '\' + Rec.Name;
        if not DelTree(Dir, True, True, True) then
          ScheduleDeleteTree(Dir);
      end;
    until not FindNext(Rec);
  finally
    FindClose(Rec);
  end;
end;

procedure RemoveOldTipVersions;
begin
  { 今の版より前の tip\<版>\ を消す（ADR 0063 §3 の 4） }
  RemoveTipVersionsExcept('{#AppVersion}');
end;

procedure FinishInstall;
begin
  if not AppDirTrusted then
    exit;
  if not VerifyAppDir then
    exit;
  RegisterTip;
  AddFirewallRules;
  AddAutostart;
  ExcludeFromErrorReporting;
  RemoveOldTipVersions;
  { エンジンはここで起動しない（管理者の権限で起動すると、エンジンは何もせずに終わる。ADR 0067 §4.3）。
    次のログオンの Run か、次の入力で TIP が起動する（§4.2） }
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    InstallStarted := True
  else if CurStep = ssPostInstall then
    FinishInstall;
end;

procedure CurPageChanged(CurPageID: Integer);
var
  Text: String;
begin
  if CurPageID <> wpFinished then
    exit;
  if FailureCode <> 0 then
    Text := FmtMessage(CustomMessage('FinishedFailed'), [FailureText])
  else if not IsUpdate then
    Text := CustomMessage('FinishedFresh')
#if SecurityUpdate
  else
    Text := CustomMessage('FinishedSecurityUpdate');
#else
  else
    Text := CustomMessage('FinishedUpdate');
#endif
  WizardForm.FinishedLabel.Caption := Text;
end;

function GetCustomSetupExitCode: Integer;
begin
  Result := FailureCode;
end;

procedure DeinitializeSetup;
begin
  if WorkDirCreated then
    DelTree(WorkDir, True, True, True);
end;

{ ---- 削除（ADR 0067 §3.2 の逆順: 自動起動 → 規則 → 登録の解除 → エンジンを止める → ファイル） ---- }

procedure UnregisterTip;
var
  Code: Integer;
begin
  if FileExists(TipDll) then
    RunTrusted(SysTool('regsvr32.exe'), '/u /s "' + TipDll + '"', Code);
end;

procedure UninstallInOrder;
begin
  RemoveAutostart;
  RemoveFirewallRules;
  IncludeInErrorReporting;
  UnregisterTip;
  StopEngines(AppHelper);
  { ファイルは、このあと Inno が消す（使用中の TIP は uninsrestartdelete で再起動のときに）。
    利用者のデータ（%LOCALAPPDATA%\ikari-ime）には触れない（ADR 0067 §3.3、ADR 0063 Q3） }
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    UninstallInOrder
  else if CurUninstallStep = usPostUninstall then
  begin
    { 前の版の tip\<版>\ の残り（使用中で消せなかったもの）も、再起動で消す }
    if DirExists(ExpectedAppDir + '\tip') then
    begin
      RemoveTipVersionsExcept('');
      ScheduleDeleteTree(ExpectedAppDir + '\tip');
    end;
    SuppressibleMsgBox(CustomMessage('UninstallDataKept'), mbInformation, MB_OK, IDOK);
  end;
end;
