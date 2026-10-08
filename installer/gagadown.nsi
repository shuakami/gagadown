Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!define APP "GaGaDown"
!define EXE "GagaDown.exe"
!define BIN "..\target\x86_64-pc-windows-gnu\release"
!define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\GagaDown"

Name "${APP}"
Caption "${APP}"
UninstallCaption "${APP}"
OutFile "..\dist\GagaDown-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\GagaDown"
BrandingText " "
ShowInstDetails nevershow
ShowUninstDetails nevershow

!include MUI2.nsh
!include nsDialogs.nsh
!include LogicLib.nsh
!include FileFunc.nsh

!define MUI_ICON "..\assets\icon.ico"
!define MUI_UNICON "..\assets\icon.ico"

!insertmacro MUI_PAGE_INSTFILES
UninstPage custom un.OptionsPage un.OptionsLeave
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

; NSIS selects the matching system UI language; unsupported languages use English.
LangString UninstallTitle ${LANG_ENGLISH} "Uninstall ${APP}"
LangString UninstallTitle ${LANG_SIMPCHINESE} "卸载 ${APP}"
LangString KeepDownloads ${LANG_ENGLISH} "Downloaded files will not be deleted"
LangString KeepDownloads ${LANG_SIMPCHINESE} "已下载的文件不会被删除"
LangString PurgeSettings ${LANG_ENGLISH} "Also delete task history and settings"
LangString PurgeSettings ${LANG_SIMPCHINESE} "同时删除任务记录和设置"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "${APP}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "${APP} Setup"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductName" "${APP}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileDescription" "${APP} 安装程序"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductVersion" "${VERSION}"

Var PurgeBox
Var Purge

!macro StopApp
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM ${EXE}'
  Pop $0
  ${If} $0 == 0
    Sleep 600
  ${EndIf}
!macroend

Section "Install"
  !insertmacro StopApp
  SetOutPath "$INSTDIR"
  File "${BIN}\${EXE}"
  SetOutPath "$INSTDIR\extension"
  File /r "..\extension\*.*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"

  WriteRegStr HKCU "${UNKEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNKEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNKEY}" "EstimatedSize" $0
SectionEnd

Function .onInstSuccess
  IfSilent +2
  Exec '"$INSTDIR\${EXE}"'
FunctionEnd

Function un.OptionsPage
  !insertmacro MUI_HEADER_TEXT "$(UninstallTitle)" "$(KeepDownloads)"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateCheckbox} 0 0 100% 24u "$(PurgeSettings)"
  Pop $PurgeBox
  nsDialogs::Show
FunctionEnd

Function un.OptionsLeave
  ${NSD_GetState} $PurgeBox $Purge
FunctionEnd

Section "Uninstall"
  !insertmacro StopApp
  Delete "$DESKTOP\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}.lnk"
  DeleteRegKey HKCU "${UNKEY}"
  RMDir /r "$INSTDIR"
  ${If} $Purge == ${BST_CHECKED}
    RMDir /r "$APPDATA\gagadown"
    RMDir /r "$LOCALAPPDATA\gagadown"
  ${EndIf}
SectionEnd
