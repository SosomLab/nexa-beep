# 04 · 안전 송수신 설계 — 메시지 페이로드 3종 · 수신 파일 무해화

> **목적**: 메시지 페이로드(**Text / Image / File**)를 정의하고, **수신 파일이 수신자 PC에서 실행·감염 동작을 할 수 없도록** 하는 메커니즘을 확정한다.
> 사용자 확정(2026-08-08): *"수신은 하되 exe면 파일 헤더에 의도적으로 binary 값을 추가하는 등으로 절대 실행되지 못하게 먼저 방어하고, 수신자가 확인·승인해야 실제 실행 가능한 프로그램으로 저장되게" · "가장 잘 알려진 방법으로 안전하게"*
>
> **작성 2026-08-08.** 결정 요약은 [10 DR-13](10-decision-record.md), 구현 순서는 [TODO](TODO.md). 출처 [§9](#9-출처).
> 여기서 다루는 것은 **수신 측 방어**다. 전송 구간 기밀성(E2E)은 DR-7·ADR-0002 소관.

---

## 1. 메시지 페이로드 3종

| 종류 | 내용 | 수신 처리 |
|---|---|---|
| **Text** | 유니코드 텍스트 | 즉시 표시 — 단 제어문자·양방향 오버라이드 무해화([§6](#6-text--보이는-것과-실제가-다를-때)) |
| **Image** | 대화 안에 바로 보이는 이미지 | **디코드 자체가 공격면** — 격리 디코드 후 **재인코딩본만** 표시([§5](#5-image--우리가-직접-디코드하기-때문에-생기는-위험)) |
| **File** | 임의 파일·폴더 | **4단계 수신 게이트**를 통과해야만 실체화([§4](#4-우리-설계--4단계-수신-게이트)) |

Text와 Image도 "그냥 표시"가 아니다. **셋 다 신뢰할 수 없는 입력**으로 동일하게 취급한다.

---

## 2. 위협 모델 — 무엇을 막는가

| # | 위협 | 설명 |
|---|---|---|
| **T-1** | **수신자의 오실행** | 받은 `보고서.pdf.exe`를 더블클릭. ★ 사용자 확정 요구가 정면으로 겨냥하는 위협 |
| **T-2** | **자동 실행 경로** | 앱이 임시 폴더에 쓰고 곧바로 열기, 자동 압축 해제, 시작 프로그램 경로 쓰기 |
| **T-3** | **파서 취약점(RCE)** | 이미지·문서 디코더의 메모리 버그. **DR-6(자체 렌더링)이라 우리가 디코더를 직접 다룬다 = 이 위험이 남의 일이 아니다** |
| **T-4** | **파일명 기반 공격** | 경로 순회(`../`), **RLO(U+202E)** 로 `exe.txt`처럼 보이게 뒤집기, 이중 확장자, Windows 예약 장치명(`CON` `PRN` `COM1`), 끝의 점·공백, NTFS ADS(`a.txt:evil.exe`), NUL 바이트 |
| **T-5** | **압축 폭탄 · Zip Slip** | 해제 시 디스크 고갈 / 아카이브 항목 이름의 경로 순회로 대상 디렉터리 밖에 쓰기 |
| **T-6** | **자원 고갈** | 초대형 파일·무한 전송으로 디스크·메모리 소진 |
| **T-7** | **송신자 위장** | 같은 LAN의 공격자가 남 행세 → DR-7/ADR-0002(TOFU·지문) 소관. **여기서는 "보낸 사람을 믿을 수 없다"를 전제로 설계** |
| **T-8** | **스팸 폭탄** | 무인증 즉시 발신의 필연적 부작용 — 국내 소프트메신저 서비스 종료 실사례([03 §6-5](03-competitive-landscape.md)) |

> **설계 전제**: T-7 때문에 **보낸 사람이 아는 사람이라도 파일은 믿지 않는다.** 무해화는 발신자 신원과 무관하게 항상 적용된다.

---

## 3. 업계 표준 기법 — "가장 잘 알려진 방법" 5종

우리가 새로 발명할 영역이 아니다. 브라우저·메일 클라이언트·백신이 20년간 쓰는 확립된 수단이 있고, 사용자가 제시한 헤더 변조 아이디어도 그중 하나(2번)의 실제 구현이다.

| # | 기법 | 실체 | 채택 |
|---|---|---|---|
| **1** | **OS 격리 표식** | · **Windows: Mark-of-the-Web(MotW)** — 파일에 `Zone.Identifier` **대체 데이터 스트림(ADS)** 을 붙여 `ZoneId=3`(인터넷)으로 표시. 이후 실행 시 **SmartScreen 경고**, Office는 **보호된 보기**로 열린다. 앱이 붙이는 정식 방법은 **`IAttachmentExecute` 인터페이스**(Chrome 방식·권장), 직접 ADS 쓰기도 가능(Firefox 방식)<br>· **macOS: `com.apple.quarantine` 확장 속성** — Info.plist에 **`LSFileQuarantineEnabled`** 를 넣으면 그 앱이 만든 파일에 시스템이 자동 부착, 또는 **`qtn_file_apply_to_path`** 로 직접. 이 속성이 **Gatekeeper·XProtect 검사를 발동**시킨다<br>· **Linux**: 실행 비트(`+x`) 미부여 | ★★ **필수** — OS가 준 마지막 안전망. `curl`/`wget`처럼 표식을 안 붙이는 게 오히려 예외적 동작이다 |
| **2** | **디팽잉(defanging)** | 격리 보관 시 **실행 가능성 자체를 제거**: 확장자 중화·파일명 변경·헤더 변조. 백신 격리소가 실제로 쓰는 방식이며, Defender 격리 복원 도구조차 **파일명 끝에 `_` 를 붙여 실수로 더블클릭되는 것을 막는다** | ★★ **필수** — 사용자 확정 요구의 실체 |
| **3** | **AMSI**(Windows) | Antimalware Scan Interface — `AmsiInitialize` → `AmsiOpenSession` → **`AmsiScanBuffer`** 로 **디스크에 쓰기 전 버퍼 상태의 내용을** 설치된 백신(Defender 또는 서드파티)에 검사 의뢰하는 Microsoft 표준 | ★ 채택(Windows 한정·있으면 사용) |
| **4** | **CDR** (Content Disarm & Reconstruction) | 탐지에 의존하지 않고 파일을 **분해 → 형식 명세 대비 검증 → 매크로·스크립트·능동 콘텐츠 제거 → 재구성**하는 제로 트러스트 기법. Type 1(PDF 변환)/Type 2(능동 콘텐츠 제거)/Type 3(템플릿 재구성). **미국 NSA가 파일 형식별 Inspection and Sanitization Guidance를 공개**할 정도로 표준화된 영역 | △ **부분 채택** — 이미지 **재인코딩**([§5](#5-image--우리가-직접-디코드하기-때문에-생기는-위험))은 사실상 CDR Type 3. 문서(Office/PDF) CDR은 파서를 다 떠안아야 해서 **DR-5(최소 크기) 위반 → 1차 범위 밖**([§7](#7-하지-않는-것--정직한-한계)) |
| **5** | **입력 검증** | 파일명 정규화, **매직바이트 vs 선언 확장자 대조**, 아카이브 항목의 **대상 경로가 추출 디렉터리의 자식인지 검증**(Zip Slip 방지), 압축률·항목 수·깊이 상한 | ★★ **필수** |

---

## 4. 우리 설계 — 4단계 수신 게이트

> 핵심 원칙 **3**개:
> ① **바이트가 디스크에 닿는 순간부터 이미 무해화되어 있다** — "일단 저장하고 나중에 처리"는 없다.
> ② **원본 파일명은 승인 전까지 파일시스템에 존재하지 않는다** — T-4 공격면을 통째로 제거한다.
> ③ **우리 앱은 어떤 경우에도 수신 파일을 실행하지 않는다** — 실행 API 호출 자체가 코드에 없다.

```
[0] 협상        메타데이터만 먼저 → 수신자가 수락해야 바이트 전송 시작
     ↓
[1] 격리 수신    .beepq 컨테이너로 저장 — 헤더 봉인·원본명 미사용·실행권한 없음·OS 격리표식
     ↓
[2] 검사        해시 검증 · 매직 대조 · 위험등급 판정 · AMSI 스캔 · 이미지 격리 디코드
     ↓
[3] 승인·실체화  수신자 명시 승인 → 원본 복원. 위험등급별 마찰 차등 + 복원 후에도 MotW 유지
```

### [0] 협상 — 받기 전에 정한다

- 송신자는 **메타데이터만** 먼저 보낸다: 원본 파일명 · 크기 · **SHA-256** · 선언 MIME/확장자.
- **수신자가 수락하기 전에는 단 1바이트도 받지 않는다.** (T-6·T-8 1차 차단)
- 크기 상한·동시 전송 수·발신자별 속도 제한을 여기서 건다.

### [1] 격리 수신 — `.beepq` 컨테이너

받은 바이트는 **원본 형태로 디스크에 존재한 적이 없다**. 격리 폴더에 자체 컨테이너로 쓴다.

| 요소 | 내용 | 막는 위협 |
|---|---|---|
| **파일명** | **콘텐츠 주소 방식** `<sha256>.beepq` — 원본 파일명을 파일시스템에 쓰지 않는다. 원본명은 컨테이너 **내부 메타**로만 보관하고 UI에서 문자열로 표시 | **T-4 전부**(경로순회·RLO·이중확장자·예약장치명·ADS·NUL) — 파일명을 안 쓰면 파일명 공격이 성립하지 않는다 |
| **헤더 봉인** | 컨테이너 선두는 우리 매직 `NXBQ` + 버전 + 메타. **원본의 선두 N바이트는 잘라내 메타 영역에 별도 보관**하고 본문에는 남기지 않는다 | **T-1** — 어떤 OS 로더·인터프리터도 이 파일을 실행 이미지로 해석하지 못한다. 확장자를 바꿔도, 이름을 바꿔도 실행되지 않는다 |
| **확장자** | `.beepq` — OS에 연결 프로그램을 **등록하지 않는다** | T-1 — 더블클릭해도 열릴 대상이 없다 |
| **권한** | 실행 비트 없음, 소유자 전용(0600 상당) | T-1 |
| **OS 격리 표식** | Windows **MotW**(`Zone.Identifier`, ZoneId=3) · macOS **`com.apple.quarantine`** · Linux `+x` 미부여 | T-1·T-2 — 마지막 안전망 |
| **위치** | 전용 격리 폴더. 임시 폴더·시작 프로그램·자동 실행 경로 **절대 금지** | **T-2** |

> **사용자 아이디어와의 관계**: "exe 헤더에 의도적으로 binary 값을 추가"가 정확히 이 **헤더 봉인**이다.
> 다만 **헤더 변조만으로는 부족하다** — Windows는 **확장자로 실행 여부를 결정**하고, `.bat` `.cmd` `.ps1` `.vbs` `.js` `.hta` `.lnk`는 **매직바이트가 아예 없다**(헤더를 건드릴 것이 없다). 그래서 헤더 봉인 + **확장자 중화** + **원본명 미사용** + 실행권한 제거 + OS 격리 표식을 **동시에** 건다. 하나가 뚫려도 나머지가 남는 구조다.

### [2] 검사 — 승인 화면에 보여줄 근거를 만든다

| 검사 | 내용 |
|---|---|
| 무결성 | 수신 바이트의 SHA-256이 협상 시 값과 일치하는가 |
| **형식 대조** | 실제 **매직바이트**로 판정한 형식 vs 송신자가 **선언한 확장자** 불일치 → 경고("PDF라고 했지만 실행 파일입니다") |
| **위험 등급** | 아래 4등급 판정 |
| **AMSI**(Win) | 설치된 백신에 버퍼 검사 의뢰. 미설치/미지원이면 "검사 안 됨"으로 정직하게 표시(조용히 통과 금지) |
| 이미지 | [§5](#5-image--우리가-직접-디코드하기-때문에-생기는-위험) 별도 경로 |
| 아카이브 | **자동 해제 금지.** 목록만 읽되 항목 이름을 정규화해 **추출 경로가 대상 디렉터리의 자식인지 검증**, 압축률·항목수·깊이 상한 초과 시 거부 (T-5) |

**위험 등급 4단계**

| 등급 | 대상 | 승인 마찰 |
|---|---|---|
| 🔴 **실행형** | `exe dll scr com pif bat cmd ps1 psm1 vbs js jse wsf wsh hta lnk msi msp reg jar app pkg dmg sh deb rpm AppImage` 등 | **최고** — 아래 §4-[3] |
| 🟠 **능동 콘텐츠 문서** | 매크로 포함 Office(`docm xlsm pptm`), PDF 등 | 높음 — 경고 + 복원 후 MotW 유지(보호된 보기 발동) |
| 🟡 **아카이브** | `zip 7z rar tar gz` 등 | 중간 — **저장만, 자동 해제 없음** |
| 🟢 **데이터** | 이미지·텍스트·일반 문서 | 낮음 — 1클릭 저장 |

### [3] 승인·실체화 — 수신자가 결정할 때만

- **명시 승인 전에는 원본이 존재하지 않는다.** 승인 시에만 헤더를 복원하고, 정규화한 원본 파일명을 부여해 사용자가 지정한 위치에 쓴다.
- **복원 후에도 MotW / `com.apple.quarantine`를 유지한다** → 실제 실행 시 **SmartScreen / Gatekeeper가 한 번 더** 막는다. 우리 승인이 OS 방어를 해제하는 일은 없다.
- 🔴 실행형은 **2단계 확인** — 위험 고지 + 명시적 재확인. 기본 버튼은 항상 "취소".
- **우리 앱은 실행하지 않는다.** 실행/열기 API 호출을 코드에 두지 않고, 제공하는 것은 **"폴더에서 보기"** 뿐이다. (T-2)
- 승인하지 않은 격리물은 보존 기간 후 자동 삭제.

---

## 5. Image — 우리가 직접 디코드하기 때문에 생기는 위험

DR-6(모든 렌더링 자체 구현)의 대가다. **이미지 디코더는 역사적으로 RCE 취약점 1순위**이고, 우리는 그 디코더를 직접 다룬다(T-3).

| 대책 | 내용 |
|---|---|
| **형식 화이트리스트** | PNG · JPEG · GIF · WebP만. **SVG 금지**(스크립트·외부 참조 실행 가능한 문서 형식이지 이미지가 아니다) |
| **격리 디코드** | 디코딩은 권한을 낮춘 **별도 프로세스**에서. 크래시가 나도 메신저 본체는 살아 있고, 그 자체가 탐지 신호가 된다 |
| **상한** | 픽셀 수·메모리·디코드 시간 상한(디컴프레션 밤 방어) |
| **재인코딩만 표시** | 원본 바이트를 UI에 넘기지 않는다. **정제한 픽셀을 우리 포맷으로 재인코딩한 것만** 렌더 — 이것이 사실상 **CDR Type 3(재구성)** 이다 |
| **메타데이터 제거** | EXIF·GPS 등 제거(수신 표시용). 원본 저장은 [§4](#4-우리-설계--4단계-수신-게이트) File 경로를 그대로 탄다 |
| **송신 측 배려** | 보내는 쪽에서도 EXIF/GPS를 기본 제거(위치 유출 방지) — 옵션으로 원본 유지 |

---

## 6. Text — 보이는 것과 실제가 다를 때

| 위협 | 대책 |
|---|---|
| **RLO/양방향 제어문자**(U+202E 등)로 문자열 뒤집기 | 렌더 전 제거 또는 가시화. **파일명 표시에도 동일 적용**(T-4의 표시 계층) |
| 제어문자·0폭 문자 | 제거/가시화 |
| 동형이의 문자(homoglyph)로 사용자명 위장 | 사용자 목록에서 의심 문자 표기 + **키 지문으로 최종 식별**(DR-7) |
| 링크 | **자동 열기 절대 금지.** 클릭 시 전체 URL을 보여주고 확인받는다 |
| 마크업 주입 | **자체 렌더링이 여기서는 유리하다** — HTML 파서도 스크립트 엔진도 없으므로 XSS류 공격면이 구조적으로 없다 |
| 길이 | 표시·저장 상한 |

---

## 7. 하지 않는 것 — 정직한 한계

명시해 두지 않으면 나중에 "왜 안 되냐"가 된다.

| 안 하는 것 | 이유 |
|---|---|
| **자체 백신 엔진** | 범위를 벗어나고 DR-5(최소 크기)와 정면 충돌. **OS·설치된 백신에 위임**(AMSI·Gatekeeper·XProtect) |
| **문서 CDR(Office/PDF 재구성)** | 형식별 파서를 전부 떠안아야 한다 = 크기·유지보수 폭발. 대신 **격리 + MotW 유지로 보호된 보기 발동**까지만 |
| **샌드박스 실행** | 우리는 실행하지 않는다. 실행은 OS와 사용자 몫 |
| **헤더 변조만으로 안전 주장** | 위 §4-[1] 주석대로 스크립트·`.lnk`는 매직이 없다. **다층 방어의 한 겹**으로만 취급 |
| **"검사 통과 = 안전" 표기** | AMSI 미설치·미탐지가 있으므로 **"검사됨/검사 안 됨"** 사실만 표시한다. 안전을 보증하지 않는다 |

---

## 8. 요구사항 반영

[05 요구사항](05-requirements.md) 작성 시 아래를 그대로 옮긴다.

| 후보 ID | 내용 | 근거 |
|---|---|---|
| FR-Payload-1~3 | Text · Image · File 페이로드 | 사용자 확정 |
| **FR-Safe-1** | 수신 파일은 **협상 → 격리 → 검사 → 승인** 4단계를 반드시 거친다 | 사용자 확정 |
| **FR-Safe-2** | 격리물은 **`.beepq` 컨테이너**(헤더 봉인·콘텐츠 주소 파일명·실행권한 없음)로만 존재 | §4-[1] |
| **FR-Safe-3** | 복원 시 **OS 격리 표식(MotW/quarantine)을 유지**한다 | §3-1 |
| **FR-Safe-4** | 앱은 수신 파일을 **실행하지 않는다**(실행 API 미사용) | §4-[3] |
| **FR-Safe-5** | 아카이브 **자동 해제 금지** + Zip Slip·압축폭탄 방어 | T-5 |
| **FR-Safe-6** | 이미지는 **격리 디코드 + 재인코딩본만** 표시, SVG 미지원 | §5 |
| **FR-Safe-7** | 파일명·텍스트의 **RLO·제어문자 무해화** | §6 |
| **FR-Safe-8** | Windows에서 **AMSI** 사용 가능 시 스캔, 결과를 사실대로 표시 | §3-3 |
| NFR-Safe-1 | 무해화 처리가 전송 처리량을 과도하게 떨어뜨리지 않을 것(수치는 05에서 확정) | DR-5 |

---

## 9. 출처

- **Mark-of-the-Web** — [Downloads and the Mark-of-the-Web (text/plain)](https://textslashplain.com/2016/04/04/downloads-and-the-mark-of-the-web/) · [MotW: Some Technical Details (SANS ISC)](https://isc.sans.edu/diary/31732) · [Mark of the Web Bypass (Red Canary)](https://redcanary.com/threat-detection-report/techniques/mark-of-the-web-bypass/) · [MotW from a Red Team's Perspective (Outflank)](https://www.outflank.nl/blog/2020/03/30/mark-of-the-web-from-a-red-teams-perspective/)
- **macOS 격리·Gatekeeper** — [Gatekeeper / Quarantine / XProtect (HackTricks)](https://hacktricks.wiki/en/macos-hardening/macos-security-and-privilege-escalation/macos-security-protections/macos-gatekeeper.html) · [Gatekeeping in macOS (Red Canary)](https://redcanary.com/blog/threat-detection/gatekeeper/) · [com.apple.quarantine 해설](https://www.isscloud.io/guides/macos-security-and-com-apple-quarantine-extended-attribute/)
- **AMSI** — [Antimalware Scan Interface 포털 (Microsoft Learn)](https://learn.microsoft.com/en-us/windows/win32/amsi/antimalware-scan-interface-portal) · [How AMSI helps (Microsoft Learn)](https://learn.microsoft.com/en-us/windows/win32/amsi/how-amsi-helps) · [Better know a data source: AMSI (Red Canary)](https://redcanary.com/blog/threat-detection/better-know-a-data-source/amsi/)
- **CDR** — [What is Content Disarm and Reconstruction (OPSWAT)](https://www.opswat.com/blog/what-is-content-disarm-and-reconstruction) · [CDR는 정부 요구 통제 항목 (Sasa Software)](https://www.sasa-software.com/blog/content-disarm-and-reconstruction-cdr-is-not-a-vendor-claim-its-a-government-mandated-security-control/) · [Deep CDR (OPSWAT)](https://www.opswat.com/technologies/deep-cdr)
- **격리·디팽잉 실무** — [Windows Defender Quarantine Forensics (NCC Group/Fox-IT)](https://blog.fox-it.com/2023/12/14/reverse-reveal-recover-windows-defender-quarantine-forensics/) — 복원 파일명에 `_`를 붙여 오실행을 막는 관행 · [Restore quarantined files (Microsoft Learn)](https://learn.microsoft.com/en-us/defender-endpoint/restore-quarantined-files-microsoft-defender-antivirus)
- **아카이브 공격** — [Zip Path Traversal (Android Developers)](https://developer.android.com/privacy-and-security/risks/zip-path-traversal) · [Zip Slip 해설 (ASEC)](https://asec.ahnlab.com/en/89890/) · [ZIP 라이브러리 취약점 사례 (Ostorlab)](https://blog.ostorlab.co/zip-packages-exploitation.html)
