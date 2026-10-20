; The per-user Windows installer: quiet, with no pages. It puts the app in place, adds it to the
; Start menu and the installed apps, and opens it. The uninstaller asks the running strip to
; quit first, so its edge is given back (ABM_REMOVE), then removes the login item and every file
; it wrote. The data folder stays: its observations cannot be rebuilt.
;
; `cargo xtask package` passes every value with -D: the words come from the app's copy module
; and the names from its pinned identifiers, so this script holds neither.
;
;   PRODUCT, PUBLISHER, LICENCE        the copy module's words, and what the installer
;   STILL_RUNNING                      says when the strip will not quit
;   VERSION                            the workspace version, three numbers
;   BINARY                             the executable's name, without .exe
;   INSTALL_DIR                        the folder under %LOCALAPPDATA%\Programs
;   UNINSTALL_KEY                      the installed apps' entry, under HKCU
;   RUN_VALUE                          the login item's value in HKCU's Run key
;   EXE, LICENSES, ICON                the files to install, and the installer's icon
;   OUTFILE                            where the installer is written

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user
SilentInstall silent
SilentUnInstall silent

Name "${PRODUCT}"
OutFile "${OUTFILE}"
Icon "${ICON}"
UninstallIcon "${ICON}"
InstallDir "$LOCALAPPDATA\Programs\${INSTALL_DIR}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT}"
VIAddVersionKey "FileDescription" "${PRODUCT}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "${LICENCE}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define UNINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\${UNINSTALL_KEY}"
!define RUN "Software\Microsoft\Windows\CurrentVersion\Run"
; How long a file the strip held may stay held after it quits: 40 tries, a quarter second apart.
!define TRIES 40

; Asks a strip running from the install folder to quit, and waits for it (`--quit` waits up to
; a minute). One that will not quit stops the change here, with everything as it was. nsExec
; runs it with no console window.
!macro QUIT_RUNNING prefix
Function ${prefix}QuitRunning
  ${If} ${FileExists} "$INSTDIR\${BINARY}.exe"
    nsExec::Exec '"$INSTDIR\${BINARY}.exe" --quit'
    Pop $0
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONINFORMATION "${STILL_RUNNING}"
      SetErrorLevel 2
      Quit
    ${EndIf}
  ${EndIf}
FunctionEnd
!macroend
!insertmacro QUIT_RUNNING ""
!insertmacro QUIT_RUNNING "un."

; Removes the installed exe, trying again while Windows still holds it for a moment after the
; strip's process ends.
!macro REMOVE_EXE prefix
Function ${prefix}RemoveExe
  StrCpy $1 0
  ${Do}
    ClearErrors
    Delete "$INSTDIR\${BINARY}.exe"
    ${IfNot} ${Errors}
      ${Break}
    ${EndIf}
    IntOp $1 $1 + 1
    ${If} $1 >= ${TRIES}
      MessageBox MB_OK|MB_ICONINFORMATION "${STILL_RUNNING}"
      SetErrorLevel 2
      Quit
    ${EndIf}
    Sleep 250
  ${Loop}
FunctionEnd
!macroend
!insertmacro REMOVE_EXE ""
!insertmacro REMOVE_EXE "un."

Section
  SetOutPath "$INSTDIR"
  Call QuitRunning
  Call RemoveExe
  File "/oname=${BINARY}.exe" "${EXE}"
  File "/oname=LICENSES.txt" "${LICENSES}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortCut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\${BINARY}.exe"

  WriteRegStr HKCU "${UNINSTALL}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTALL}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL}" "DisplayIcon" "$INSTDIR\${BINARY}.exe,0"
  WriteRegStr HKCU "${UNINSTALL}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL}" "EstimatedSize" "$0"

  ; A person's double-click opens the app; a scripted install (/S) leaves it closed.
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/S" $1
  ${If} ${Errors}
    Exec '"$INSTDIR\${BINARY}.exe"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  Call un.QuitRunning
  DeleteRegValue HKCU "${RUN}" "${RUN_VALUE}"
  Call un.RemoveExe
  Delete "$INSTDIR\LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
  DeleteRegKey HKCU "${UNINSTALL}"
SectionEnd
