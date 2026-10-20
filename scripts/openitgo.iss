; openitgo.iss — OpenItGo Windows 安装包（Inno Setup 6）。
;
; 由 scripts/package-windows.ps1 调用，直接在仓库根目录手动编译：
;   ISCC.exe /DMyAppVersion=0.1.0 scripts/openitgo.iss
;
; 前置：先运行 scripts/package-windows.ps1 生成 dist/OpenItGo 暂存目录。

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

#define MyAppName "OpenItGo"
#define MyAppPublisher "OpenItGo"
#define MyAppExeName "OpenItGo.exe"
#define MyStage "..\dist\OpenItGo"

[Setup]
; AppId 是卸载/升级识别的唯一键，发布后不要修改。
AppId={{EA549162-A655-4017-B823-1738C957BA7E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile={#MyStage}\LICENSE
; 默认按当前用户安装（无需 UAC，方便测试）；安装页可切换为所有用户。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=OpenItGo-{#MyAppVersion}-setup
SetupIconFile=..\assets\icon\AppIcon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
; 官方 Inno Setup 不自带简体中文，翻译文件随仓库保存（scripts/languages）。
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#MyStage}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyStage}\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyStage}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyStage}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MyStage}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
