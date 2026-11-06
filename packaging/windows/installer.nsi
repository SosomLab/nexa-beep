; Nexa Beep — Windows 설치본(NSIS)
;
; @VERSION@ / @SLUG@ 는 릴리스 워크플로가 치환한다.
;
; 설계 의도
; - **사용자 단위 설치**(HKCU · $LOCALAPPDATA) — 관리자 권한을 요구하지 않는다.
;   T0(무권한)에서 전 기능이 돌아야 한다는 원칙(DR-14)을 설치 단계에도 적용한다.
;   winget/choco의 무인 설치도 권한 상승 없이 통과한다.
; - `/S` **무인 설치**를 지원한다(winget·choco가 이걸 쓴다). 커스텀 입력 페이지를
;   두지 않아 무인 모드에서 물어볼 것이 없다.
; - 제거 정보를 레지스트리에 남긴다 — 없으면 winget이 설치 여부·버전을 알 수 없다.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APPNAME    "Nexa Beep"
!define COMPANY    "SosomLab"
!define VERSION    "@VERSION@"
!define SLUG       "@SLUG@"
!define EXENAME    "nexa-beep.exe"
!define REGKEY     "Software\Microsoft\Windows\CurrentVersion\Uninstall\NexaBeep"

Name "${APPNAME} ${VERSION}"
OutFile "nexa-beep-${VERSION}-${SLUG}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\NexaBeep"
InstallDirRegKey HKCU "Software\${COMPANY}\NexaBeep" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "0.0.0.0"
VIAddVersionKey "ProductName"     "${APPNAME}"
VIAddVersionKey "CompanyName"     "${COMPANY}"
VIAddVersionKey "FileDescription" "제로 컨피그 로컬 네트워크 메신저"
VIAddVersionKey "FileVersion"     "${VERSION}"
VIAddVersionKey "ProductVersion"  "${VERSION}"
VIAddVersionKey "LegalCopyright"  "PolyForm Noncommercial 1.0.0"

!define MUI_ICON   "nexa-beep.ico"
!define MUI_UNICON "nexa-beep.ico"
!define MUI_ABORTWARNING
; GUI 실행 인자를 명시한다 — 런타임 감지(M5-4d)가 첫 방어선이고, 이건 두 번째 방어선이다
; (바로가기·완료 실행이 인자 없이 부르면 콘솔이 번쩍이던 문제 · 08-11 실기).
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXENAME}"
!define MUI_FINISHPAGE_RUN_PARAMETERS "--window --live"
; ★ FW-1(11-06) 방화벽 허용 등록 — **선택**(기본 꺼짐) · 체크하면 UAC 1회로 netsh 인바운드 허용 규칙(프로그램 경로 기준)을
;   추가한다. 설치 자체는 여전히 무권한(T0) · /S 무인 설치(winget·choco)는 완료 페이지가 없어 승격 0 · 제거기는 표식이 있을 때만 지운다.
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Windows 방화벽 허용 규칙 등록 (관리자 권한 1회 · LAN 발견 도달률 향상)"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION FwAdd

!insertmacro MUI_PAGE_LICENSE "LICENSE.md"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Korean"
!insertmacro MUI_LANGUAGE "English"

Section "설치"
  SetOutPath "$INSTDIR"
  File "${EXENAME}"
  File "nbeep-imgdec.exe" ; 이미지 격리 디코드(M4-5) — 없으면 아바타 이니셜 폴백
  File "nexa-beep.ico"
  File "README.md"
  File "LICENSE.md"

  ; 시작 메뉴 바로가기 = GUI 실행(인자 명시 · M5-4d 두 번째 방어선).
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "--window --live" "$INSTDIR\nexa-beep.ico"

  WriteRegStr HKCU "Software\${COMPANY}\NexaBeep" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\${COMPANY}\NexaBeep" "Version"    "${VERSION}"

  ; winget/choco가 읽는 제거 정보. EstimatedSize는 KB.
  WriteRegStr   HKCU "${REGKEY}" "DisplayName"     "${APPNAME}"
  WriteRegStr   HKCU "${REGKEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${REGKEY}" "Publisher"       "${COMPANY}"
  WriteRegStr   HKCU "${REGKEY}" "DisplayIcon"     "$INSTDIR\nexa-beep.ico"
  WriteRegStr   HKCU "${REGKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKCU "${REGKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr   HKCU "${REGKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegDWORD HKCU "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${REGKEY}" "NoRepair" 1

  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; ⚠️ 사용자가 만든 것은 지우지 않는다 — 설치가 놓은 파일만 이름으로 지운다.
  ;    포터블 규약(실행 파일 옆 영속물)을 생각하면 RMDir /r는 위험하다.
  Delete "$INSTDIR\${EXENAME}"
  Delete "$INSTDIR\nbeep-imgdec.exe"
  Delete "$INSTDIR\nexa-beep.ico"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"                 ; 비어 있을 때만 지워진다

  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  ; FW-1 — 설치기 체크박스로 만든 규칙만 지운다(표식 1 · UAC 1회 · 거부해도 제거는 계속).
  ReadRegDWORD $0 HKCU "Software\${COMPANY}\NexaBeep" "FirewallRule"
  ${If} $0 == 1
    ExecShellWait "runas" "$SYSDIR\cmd.exe" '/c netsh advfirewall firewall delete rule name="Nexa Beep"' SW_HIDE
  ${EndIf}
  DeleteRegKey HKCU "${REGKEY}"
  DeleteRegKey HKCU "Software\${COMPANY}\NexaBeep"
SectionEnd

; FW-1 — 완료 페이지 체크박스 함수: 같은 이름·경로 규칙을 지운 뒤 추가(멱등) · 표식 기록(제거기 근거).
Function FwAdd
  ExecShellWait "runas" "$SYSDIR\cmd.exe" '/c netsh advfirewall firewall delete rule name="Nexa Beep" program="$INSTDIR\${EXENAME}" & netsh advfirewall firewall add rule name="Nexa Beep" dir=in action=allow program="$INSTDIR\${EXENAME}" enable=yes profile=any' SW_HIDE
  WriteRegDWORD HKCU "Software\${COMPANY}\NexaBeep" "FirewallRule" 1
FunctionEnd
