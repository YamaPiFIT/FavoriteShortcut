; お気に入りショートカット インストーラー定義（Inno Setup 6）
;
; 使い方:
;   1. scripts\build.ps1 を実行して build\self-contained\FavoriteShortcut.exe を作る
;   2. Inno Setup 6 をインストールする  https://jrsoftware.org/isinfo.php
;   3. 次を実行する
;      & "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\FavoriteShortcut.iss
;
; 生成物: installer\FavoriteShortcut-Setup.exe

#define AppName "お気に入りショートカット"
#define AppVersion "1.0.0"
#define AppExeName "FavoriteShortcut.exe"
#define SourceDir "..\build\self-contained"

[Setup]
AppId={{8F3C6A21-6F5E-4C8B-9E2A-2B7D4F9A1C33}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
DefaultDirName={autopf}\FavoriteShortcut
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=.
OutputBaseFilename=FavoriteShortcut-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; 管理者権限を必要としない（ユーザー単位インストールも選べる）
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
DisableProgramGroupPage=yes
AppPublisher={#AppName}

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "デスクトップにショートカットを作成する"; GroupDescription: "追加のタスク:"
Name: "startup"; Description: "Windows のサインイン時に自動起動する（トレイ常駐）"; GroupDescription: "追加のタスク:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{#AppName} をアンインストール"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--tray"; Tasks: startup

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{#AppName} を起動する"; Flags: nowait postinstall skipifsilent

[Code]
// アンインストール時に、登録データを消すかどうかをユーザーに確認する。
//
// データは通常 {app}\Data に作られるが、Program Files のように書き込みが
// できない場所にインストールされた場合は %APPDATA% 側へ退避されるため、
// 両方を確認する。
procedure AskAndRemove(DataDir: String);
begin
  if not DirExists(DataDir) then
    Exit;

  if MsgBox('登録したショートカットのデータも削除しますか？' + #13#10 + #13#10 +
            DataDir + #13#10 + #13#10 +
            '「いいえ」を選ぶとデータは残り、再インストール時にそのまま使えます。',
            mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
  begin
    DelTree(DataDir, True, True, True);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    AskAndRemove(ExpandConstant('{app}\Data'));
    AskAndRemove(ExpandConstant('{userappdata}\お気に入りショートカット'));
  end;
end;
