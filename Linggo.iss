; Linggo —— 离线划词翻译（Windows x64）安装脚本
; Inno Setup 6 脚本：安装包 exe -> 开始菜单 + 可选桌面快捷方式 + 卸载项。
; 安装时可选择首选/次选语言与若干偏好，写入 %APPDATA%\Linggo\settings.json。
; 编译：ISCC.exe Linggo.iss   （默认输出到仓库上一级的 发布包\ 目录）

[Setup]
AppId={{7A2E9C41-9F3E-4A5B-BF8C-2D1E6C4A8B11}
AppName=Linggo
AppVersion=0.1.0
AppVerName=Linggo 0.1.0
AppPublisher=WaneDay
AppCopyright=Copyright (C) 2026 WaneDay
VersionInfoVersion=0.1.0.0
VersionInfoCompany=WaneDay
VersionInfoDescription=Linggo Setup
DefaultDirName={userpf}\Linggo
DefaultGroupName=Linggo
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
AllowNoIcons=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\发布包
OutputBaseFilename=Linggo-Setup-0.1.0
SetupIconFile=src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\Linggo.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; 版本更新覆盖安装（保留用户设置，见 [Code]）
UsePreviousAppDir=yes
Uninstallable=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional tasks:"; Flags: unchecked
Name: "startup"; Description: "Run Linggo at Windows startup (silent, tray only)"; GroupDescription: "Additional tasks:"; Flags: unchecked
Name: "quicklaunchicon"; Description: "Create a Quick Launch icon"; GroupDescription: "Additional tasks:"; Flags: unchecked

[Files]
Source: "src-tauri\target\release\Linggo.exe"; DestDir: "{app}"; Flags: ignoreversion
; 源码目录即当前脚本目录下的相对路径；编译前需先构建出 src-tauri\target\release\Linggo.exe

[Dirs]
Name: "{app}\models"

[Registry]
; 开机自启（随安装 task 勾选写入；卸载时自动删除该值；应用内亦可自行管理同一键）
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Linggo"; ValueData: """{app}\Linggo.exe"" --silent"; Flags: uninsdeletevalue; Tasks: startup

[Icons]
Name: "{group}\Linggo"; Filename: "{app}\Linggo.exe"
Name: "{group}\卸载 Linggo"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Linggo"; Filename: "{app}\Linggo.exe"; Tasks: desktopicon
Name: "{userappdata}\Microsoft\Internet Explorer\Quick Launch\Linggo"; Filename: "{app}\Linggo.exe"; Tasks: quicklaunchicon

[Run]
Filename: "{app}\Linggo.exe"; Description: "立即启动 Linggo"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 清理用户运行数据（卸载即清理，避免残留；模型文件在安装目录 models\ 或用户自选路径，不在此列）
Type: files; Name: "{userappdata}\Linggo\settings.json"
Type: files; Name: "{userappdata}\Linggo\history.json"
Type: dirifempty; Name: "{userappdata}\Linggo"

[Code]
const
  LangCount = 33;

var
  PageLangs: TWizardPage;
  PagePrefs: TWizardPage;
  ComboPreferred: TNewComboBox;
  ComboSecondary: TNewComboBox;
  ComboIdle: TNewComboBox;
  ComboHistory: TNewComboBox;
  ComboTheme: TNewComboBox;
  ChkGame: TNewCheckBox;
  ChkNmt: TNewCheckBox;
  ChkFallback: TNewCheckBox;
  ChkPinOverlay: TNewCheckBox;
  ChkPinTip: TNewCheckBox;
  ChkOverwrite: TNewCheckBox;
  LangNames: array[0..32] of String;
  LangCodes: array[0..32] of String;
  IdleNames: array[0..3] of String;
  IdleVals: array[0..3] of Integer;
  HistNames: array[0..4] of String;
  HistVals: array[0..4] of Integer;
  ThemeNames: array[0..2] of String;
  ThemeVals: array[0..2] of String;

procedure InitConsts();
begin
  LangNames[0] := '中文（简体） (zh)';  LangCodes[0] := 'zh';
  LangNames[1] := '英语 (en)';          LangCodes[1] := 'en';
  LangNames[2] := '日语 (ja)';          LangCodes[2] := 'ja';
  LangNames[3] := '韩语 (ko)';          LangCodes[3] := 'ko';
  LangNames[4] := '法语 (fr)';          LangCodes[4] := 'fr';
  LangNames[5] := '德语 (de)';          LangCodes[5] := 'de';
  LangNames[6] := '西班牙语 (es)';      LangCodes[6] := 'es';
  LangNames[7] := '意大利语 (it)';      LangCodes[7] := 'it';
  LangNames[8] := '葡萄牙语 (pt)';      LangCodes[8] := 'pt';
  LangNames[9] := '俄语 (ru)';          LangCodes[9] := 'ru';
  LangNames[10] := '阿拉伯语 (ar)';     LangCodes[10] := 'ar';
  LangNames[11] := '印地语 (hi)';       LangCodes[11] := 'hi';
  LangNames[12] := '越南语 (vi)';       LangCodes[12] := 'vi';
  LangNames[13] := '泰语 (th)';         LangCodes[13] := 'th';
  LangNames[14] := '印尼语 (id)';       LangCodes[14] := 'id';
  LangNames[15] := '马来语 (ms)';       LangCodes[15] := 'ms';
  LangNames[16] := '土耳其语 (tr)';     LangCodes[16] := 'tr';
  LangNames[17] := '荷兰语 (nl)';       LangCodes[17] := 'nl';
  LangNames[18] := '波兰语 (pl)';       LangCodes[18] := 'pl';
  LangNames[19] := '乌克兰语 (uk)';     LangCodes[19] := 'uk';
  LangNames[20] := '瑞典语 (sv)';       LangCodes[20] := 'sv';
  LangNames[21] := '丹麦语 (da)';       LangCodes[21] := 'da';
  LangNames[22] := '芬兰语 (fi)';       LangCodes[22] := 'fi';
  LangNames[23] := '挪威语 (no)';       LangCodes[23] := 'no';
  LangNames[24] := '捷克语 (cs)';       LangCodes[24] := 'cs';
  LangNames[25] := '匈牙利语 (hu)';     LangCodes[25] := 'hu';
  LangNames[26] := '罗马尼亚语 (ro)';   LangCodes[26] := 'ro';
  LangNames[27] := '保加利亚语 (bg)';   LangCodes[27] := 'bg';
  LangNames[28] := '克罗地亚语 (hr)';   LangCodes[28] := 'hr';
  LangNames[29] := '斯洛伐克语 (sk)';   LangCodes[29] := 'sk';
  LangNames[30] := '斯洛文尼亚语 (sl)'; LangCodes[30] := 'sl';
  LangNames[31] := '希伯来语 (he)';     LangCodes[31] := 'he';
  LangNames[32] := '希腊语 (el)';       LangCodes[32] := 'el';

  IdleNames[0] := '5 秒 (5 s)';        IdleVals[0] := 5;
  IdleNames[1] := '15 秒 (15 s)';      IdleVals[1] := 15;
  IdleNames[2] := '30 秒 (30 s)';      IdleVals[2] := 30;
  IdleNames[3] := '永久常驻 (keep loaded)'; IdleVals[3] := 0;

  HistNames[0] := '不保留 (none)';  HistVals[0] := 0;
  HistNames[1] := '20';             HistVals[1] := 20;
  HistNames[2] := '50';             HistVals[2] := 50;
  HistNames[3] := '100';            HistVals[3] := 100;
  HistNames[4] := '200';            HistVals[4] := 200;

  ThemeNames[0] := '跟随系统 (System)';  ThemeVals[0] := 'auto';
  ThemeNames[1] := '浅色 (Light)';        ThemeVals[1] := 'light';
  ThemeNames[2] := '深色 (Dark)';         ThemeVals[2] := 'dark';
end;

function JsonBool(const B: Boolean): String;
begin
  if B then
    Result := 'true'
  else
    Result := 'false';
end;

procedure CreateLangsPage(const PreviousPageID: Integer);
var
  Page: TWizardPage;
  Lbl: TNewStaticText;
  I: Integer;
begin
  Page := CreateCustomPage(PreviousPageID, '翻译语言 / Translation languages',
    '选择首选语言与次选语言：识别到首选语言时译为次选语言，识别到其他语言时译为首选语言。');
  PageLangs := Page;

  Lbl := TNewStaticText.Create(Page);
  Lbl.Parent := Page.Surface;
  Lbl.Left := 0;
  Lbl.Top := 10;
  Lbl.Caption := '首选语言 / Preferred language';

  ComboPreferred := TNewComboBox.Create(Page);
  ComboPreferred.Parent := Page.Surface;
  ComboPreferred.Left := 0;
  ComboPreferred.Top := Lbl.Top + Lbl.Height + 4;
  ComboPreferred.Width := Page.SurfaceWidth;
  ComboPreferred.Style := csDropDownList;
  for I := 0 to LangCount - 1 do
    ComboPreferred.Items.Add(LangNames[I]);
  ComboPreferred.ItemIndex := 0; // 中文

  Lbl := TNewStaticText.Create(Page);
  Lbl.Parent := Page.Surface;
  Lbl.Left := 0;
  Lbl.Top := ComboPreferred.Top + ComboPreferred.Height + 16;
  Lbl.Caption := '次选语言 / Secondary language';

  ComboSecondary := TNewComboBox.Create(Page);
  ComboSecondary.Parent := Page.Surface;
  ComboSecondary.Left := 0;
  ComboSecondary.Top := Lbl.Top + Lbl.Height + 4;
  ComboSecondary.Width := Page.SurfaceWidth;
  ComboSecondary.Style := csDropDownList;
  for I := 0 to LangCount - 1 do
    ComboSecondary.Items.Add(LangNames[I]);
  ComboSecondary.ItemIndex := 1; // 英语
end;

procedure CreatePrefsPage(const PreviousPageID: Integer);
var
  Page: TWizardPage;
  Lbl: TNewStaticText;
  I, Top: Integer;
begin
  Page := CreateCustomPage(PreviousPageID, '偏好设置 / Preferences',
    '以下选项会写入 %APPDATA%\Linggo\settings.json，安装后可在软件「设置」中随时修改。');
  PagePrefs := Page;
  Top := 10;

  Lbl := TNewStaticText.Create(Page);
  Lbl.Parent := Page.Surface;
  Lbl.Left := 0;
  Lbl.Top := Top;
  Lbl.Caption := '闲置释放 / Idle unload（大模型与 NLLB 无操作后自动释放内存）';

  ComboIdle := TNewComboBox.Create(Page);
  ComboIdle.Parent := Page.Surface;
  ComboIdle.Left := 0;
  ComboIdle.Top := Lbl.Top + Lbl.Height + 4;
  ComboIdle.Width := Page.SurfaceWidth;
  ComboIdle.Style := csDropDownList;
  for I := 0 to 3 do
    ComboIdle.Items.Add(IdleNames[I]);
  ComboIdle.ItemIndex := 1; // 15 秒
  Top := ComboIdle.Top + ComboIdle.Height + 14;

  Lbl := TNewStaticText.Create(Page);
  Lbl.Parent := Page.Surface;
  Lbl.Left := 0;
  Lbl.Top := Top;
  Lbl.Caption := '历史条数 / History limit';

  ComboHistory := TNewComboBox.Create(Page);
  ComboHistory.Parent := Page.Surface;
  ComboHistory.Left := 0;
  ComboHistory.Top := Lbl.Top + Lbl.Height + 4;
  ComboHistory.Width := Page.SurfaceWidth;
  ComboHistory.Style := csDropDownList;
  for I := 0 to 4 do
    ComboHistory.Items.Add(HistNames[I]);
  ComboHistory.ItemIndex := 2; // 50
  Top := ComboHistory.Top + ComboHistory.Height + 14;

  Lbl := TNewStaticText.Create(Page);
  Lbl.Parent := Page.Surface;
  Lbl.Left := 0;
  Lbl.Top := Top;
  Lbl.Caption := '外观主题 / Theme';

  ComboTheme := TNewComboBox.Create(Page);
  ComboTheme.Parent := Page.Surface;
  ComboTheme.Left := 0;
  ComboTheme.Top := Lbl.Top + Lbl.Height + 4;
  ComboTheme.Width := Page.SurfaceWidth;
  ComboTheme.Style := csDropDownList;
  for I := 0 to 2 do
    ComboTheme.Items.Add(ThemeNames[I]);
  ComboTheme.ItemIndex := 0; // 跟随系统
  Top := ComboTheme.Top + ComboTheme.Height + 16;

  ChkGame := TNewCheckBox.Create(Page);
  ChkGame.Parent := Page.Surface;
  ChkGame.Left := 0;
  ChkGame.Top := Top;
  ChkGame.Width := Page.SurfaceWidth;
  ChkGame.Caption := '游戏模式：全屏游戏时自动禁用全局热键';
  ChkGame.Checked := True;
  Top := ChkGame.Top + ChkGame.Height + 6;

  ChkNmt := TNewCheckBox.Create(Page);
  ChkNmt.Parent := Page.Surface;
  ChkNmt.Left := 0;
  ChkNmt.Top := Top;
  ChkNmt.Width := Page.SurfaceWidth;
  ChkNmt.Caption := '启用快速翻译引擎（OPUS / NLLB）';
  ChkNmt.Checked := True;
  Top := ChkNmt.Top + ChkNmt.Height + 6;

  ChkFallback := TNewCheckBox.Create(Page);
  ChkFallback.Parent := Page.Surface;
  ChkFallback.Left := 0;
  ChkFallback.Top := Top;
  ChkFallback.Width := Page.SurfaceWidth;
  ChkFallback.Caption := '快速引擎不可用时回退大模型翻译';
  ChkFallback.Checked := True;
  Top := ChkFallback.Top + ChkFallback.Height + 6;

  ChkPinOverlay := TNewCheckBox.Create(Page);
  ChkPinOverlay.Parent := Page.Surface;
  ChkPinOverlay.Left := 0;
  ChkPinOverlay.Top := Top;
  ChkPinOverlay.Width := Page.SurfaceWidth;
  ChkPinOverlay.Caption := '截图贴图默认显示翻译覆盖图';
  ChkPinOverlay.Checked := True;
  Top := ChkPinOverlay.Top + ChkPinOverlay.Height + 6;

  ChkPinTip := TNewCheckBox.Create(Page);
  ChkPinTip.Parent := Page.Surface;
  ChkPinTip.Left := 0;
  ChkPinTip.Top := Top;
  ChkPinTip.Width := Page.SurfaceWidth;
  ChkPinTip.Caption := '截图贴图显示快捷键提示';
  ChkPinTip.Checked := True;
  Top := ChkPinTip.Top + ChkPinTip.Height + 14;

  ChkOverwrite := TNewCheckBox.Create(Page);
  ChkOverwrite.Parent := Page.Surface;
  ChkOverwrite.Left := 0;
  ChkOverwrite.Top := Top;
  ChkOverwrite.Width := Page.SurfaceWidth;
  ChkOverwrite.Caption := '重新安装时覆盖已有设置（不勾选则保留原设置）';
  ChkOverwrite.Checked := False;
end;

procedure InitializeWizard();
begin
  InitConsts();
  CreateLangsPage(wpSelectTasks);
  CreatePrefsPage(PageLangs.ID);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = PageLangs.ID then
  begin
    if ComboPreferred.ItemIndex = ComboSecondary.ItemIndex then
    begin
      MsgBox('首选语言与次选语言不能相同，请重新选择。', mbError, MB_OK);
      Result := False;
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Path, Json: String;
begin
  if CurStep = ssPostInstall then
  begin
    Path := ExpandConstant('{userappdata}\Linggo\settings.json');
    if (not ChkOverwrite.Checked) and FileExists(Path) then
    begin
      Log('Linggo: existing settings.json kept (overwrite not selected).');
      Exit;
    end;
    ForceDirectories(ExpandConstant('{userappdata}\Linggo'));
    Json :=
      '{' + #13#10 +
      '  "theme": "' + ThemeVals[ComboTheme.ItemIndex] + '",' + #13#10 +
      '  "idleTimeoutSecs": ' + IntToStr(IdleVals[ComboIdle.ItemIndex]) + ',' + #13#10 +
      '  "historyLimit": ' + IntToStr(HistVals[ComboHistory.ItemIndex]) + ',' + #13#10 +
      '  "gameModeBlock": ' + JsonBool(ChkGame.Checked) + ',' + #13#10 +
      '  "nmtEnabled": ' + JsonBool(ChkNmt.Checked) + ',' + #13#10 +
      '  "nmtLlmFallback": ' + JsonBool(ChkFallback.Checked) + ',' + #13#10 +
      '  "pinDefaultOverlay": ' + JsonBool(ChkPinOverlay.Checked) + ',' + #13#10 +
      '  "pinShowTip": ' + JsonBool(ChkPinTip.Checked) + ',' + #13#10 +
      '  "autostart": ' + JsonBool(WizardIsTaskSelected('startup')) + ',' + #13#10 +
      '  "preferredLang": "' + LangCodes[ComboPreferred.ItemIndex] + '",' + #13#10 +
      '  "secondaryLang": "' + LangCodes[ComboSecondary.ItemIndex] + '"' + #13#10 +
      '}' + #13#10;
    if SaveStringToFile(Path, Json, False) then
      Log('Linggo: wrote ' + Path)
    else
      Log('Linggo: failed to write ' + Path);
  end;
end;
