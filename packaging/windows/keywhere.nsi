; Keywhere per-user installer (NSIS). Built by build-installers.ps1:
;   makensis /DVERSION=0.1.3 /DEXE=... /DICON=... /DLICENSE=... /DOUT=... keywhere.nsi
;
; Installs into %LOCALAPPDATA%\Programs\Keywhere without admin rights, so the
; in-app updater can replace Keywhere.exe there later (Settings → Updates).
; Adds a Start menu shortcut, an uninstaller (Settings → Apps) and offers
; Keywhere for .psafe3 files. It only becomes the default app for .psafe3 when
; no other program is.

Unicode true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Keywhere"
!define PROGID "Keywhere.psafe3"

Name "Keywhere ${VERSION}"
OutFile "${OUT}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Keywhere"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
BrandingText "Keywhere ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Keywhere"
VIAddVersionKey "FileDescription" "Keywhere installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Keywhere.exe"

!insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Romanian"

; Keywhere.exe can't be replaced while it runs.
!macro WaitUntilClosed
  ${If} ${FileExists} "$INSTDIR\Keywhere.exe"
    retry:
    ClearErrors
    FileOpen $0 "$INSTDIR\Keywhere.exe" a
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Keywhere is running. Quit it (tray icon → Quit), then choose Retry." IDRETRY retry
      Abort
    ${EndIf}
    FileClose $0
  ${EndIf}
!macroend

Section "Keywhere"
  SetOutPath "$INSTDIR"
  !insertmacro WaitUntilClosed
  File "/oname=Keywhere.exe" "${EXE}"
  File "/oname=LICENSE.txt" "${LICENSE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortCut "$SMPROGRAMS\Keywhere.lnk" "$INSTDIR\Keywhere.exe"

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Keywhere"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Keywhere"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Keywhere.exe,0"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/vcostin/keywhere"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  ; "Open with" for .psafe3; default only if nothing else owns it.
  WriteRegStr HKCU "Software\Classes\${PROGID}" "" "Password Safe database"
  WriteRegStr HKCU "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\Keywhere.exe,0"
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\Keywhere.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\.psafe3\OpenWithProgids" "${PROGID}" ""
  ReadRegStr $1 HKCR ".psafe3" ""
  ${If} $1 == ""
    WriteRegStr HKCU "Software\Classes\.psafe3" "" "${PROGID}"
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\Keywhere.exe"
  ; Copies the in-app updater moved aside.
  Delete "$INSTDIR\.Keywhere.exe.old-*"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Keywhere.lnk"

  ReadRegStr $1 HKCU "Software\Classes\.psafe3" ""
  ${If} $1 == "${PROGID}"
    DeleteRegValue HKCU "Software\Classes\.psafe3" ""
  ${EndIf}
  DeleteRegValue HKCU "Software\Classes\.psafe3\OpenWithProgids" "${PROGID}"
  DeleteRegKey HKCU "Software\Classes\${PROGID}"
  DeleteRegKey HKCU "${UNINST_KEY}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Settings (%APPDATA%\keywhere) and vault files are left alone.
SectionEnd
