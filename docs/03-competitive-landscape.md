# 03 · 경쟁 프로그램 조사 — 국내·국외 전수 목록 · 기능 매트릭스 · 장단점

> **목적**: "제로 컨피그 LAN 메신저"와 **동일 기능을 제공하는 국내·국외 프로그램을 전수 목록화**하고,
> 각 제품의 기능·장단점을 정리해 [00 비전](00-vision.md)·[05 요구사항](05-requirements.md)의 근거로 삼는다.
> 재발명 금지 원칙([16 §2-7](16-doc-git-conventions.md#2-문서-작성--필수-규칙-8))의 실행 문서다.
>
> **조사일: 2026-08-08** · 출처는 [§7](#7-출처)에 일괄 표기. 1차 출처(공식 사이트·저장소)를 우선하고, 확인하지 못한 항목은 **"미확인"** 으로 남긴다(추정 금지).

---

## 1. 조사 범위와 분류

"동일 기능"의 기준을 **DR-1(제로 컨피그)** 로 잡으면 대상이 지나치게 좁아지고, "사내 메신저"로 잡으면 클라우드 협업툴까지 다 들어온다. 그래서 **거리순 4계층**으로 나눠 전수 조사했다.

| 계층 | 정의 | 우리와의 관계 |
|---|---|---|
| **A. 직접 경쟁** | 서버 불필요 + 자동 발견 + 메시징 | ★ 기능 기준선을 여기서 뽑는다 |
| **B. 서버형 사내 메신저** | 자체 서버 설치 필요(온프레미스) | 기능은 참고, 구조는 반면교사 |
| **C. 인접 — 로컬 자동 발견** | 같은 발견 문제를 푸는 파일 전송·표준 | 프로토콜·UX 참고 |
| **D. P2P 보안 메신저** | LAN 우선은 아니나 서버 없는 E2E 암호화 | **보안 모델**을 여기서 배운다 |

---

## 2. 국외 — 전수 목록

### 2-A. 직접 경쟁 (서버 불필요 · 자동 발견 · P2P)

| # | 제품 | 개발 | 플랫폼 | 기술 | 라이선스 | 최신 확인 |
|---|---|---|---|---|---|---|
| 1 | **IP Messenger** (ipmsg) | 일본, H. Shirouzu | Windows(32/64), macOS 별도 앱 | UDP/TCP **2425** | 무료(+Pro 감사기능) | **5.8.3 · 2026-07-12** — 현역 |
| 2 | **BeeBEEP** | 이탈리아, Marco Mastroddi | Win/macOS/Linux/Raspbian/**OS-2·eCS** | C++/Qt | 오픈소스 | 5.8.6 · 2023-01-13 |
| 3 | **LAN Messenger** (lanmessenger.github.io) | Dilip Radhakrishnan 외 | Win/macOS/Linux | C++/Qt, UDP | 오픈소스(GPL) | Win 1.2.39 / mac·Linux 1.2.37 — **정체** |
| 4 | **Squiggle** | Hasan Khan | Win/Linux/macOS **x64+ARM64** | **C# .NET 9 + Avalonia UI**, protobuf-net, SQLite | **MIT** | 973 커밋 · 활동 중 |
| 5 | **KouChat** | Christian Ihle | Win/Linux/macOS + Android | Java | LGPL-3.0 | 1.3.0 · 2016 — **정체** |
| 6 | **Technitium Mesh** | Technitium | Windows 중심 | C#/.NET, **DHT** | GPL-3.0 | 1.1 — 저활동 |
| 7 | **Softros LAN Messenger** | Softros Systems | Win XP~11/Server 2003~2025, macOS 10.9~26, Android 5.1~16 | 상용 | 유료(체험판) | **12.6.4** — 현역 |
| 8 | **LanTalk NET** | CEZEO software | Windows | 상용 | 유료 | 현역 |
| 9 | **Vypress Chat** | Vypress Research | Windows | 상용 | 유료 | 현역(SOHO 대상) |
| 10 | **RealPopup** | — | Windows | 상용 | 유료 | 현역 |
| 11 | **BORGChat** | — | Windows | 프리웨어 | 무료 | 저활동 |

### 2-B. 서버형 사내 메신저 (온프레미스 — 자동 발견 아님)

| # | 제품 | 구조 | 특징 |
|---|---|---|---|
| 12 | **Output Messenger** | 사내 Windows 장비에 **전용 서버 설치** | Win/mac/Linux/iOS/Android · 채팅방·파일공유·원격데스크톱 · **$70 평생** |
| 13 | **Bopup Messenger** | **IM 서버 필수** | 오프라인 메시지·파일 보관·서식/이모티콘·다지점 사무실 통합 |
| 14 | **BigAnt** | 자체 서버 배포 | 종합 기업 IM·읽지 않음 푸시·관리 통제 |
| 15 | **Brosix · Troop · TrueConf · Mattermost · Rocket.Chat · Openfire(XMPP)** | 서버/클라우드 | 협업 기능은 풍부하나 **설치·계정·관리가 전제** |

### 2-C. 인접 — 같은 "자동 발견" 문제를 푸는 제품·표준

| # | 대상 | 핵심 | 우리에게 주는 것 |
|---|---|---|---|
| 16 | **LocalSend** | **mDNS 발견 + 실패 시 로컬 IP HTTP 스캔 폴백** · REST API over HTTPS(**기기마다 즉석 생성 TLS 인증서**) · 파일 **+ 메시지** · Flutter(Dart)+Rust · Apache-2.0 · **v1.13.0부터 포터블 모드**(실행 파일 옆 `settings.json`)·`--hidden` 트레이 시작 | ★ 발견·전송·포터블의 **현대적 정답지**. 크로스플랫폼 6종 지원 |
| 17 | **XMPP XEP-0174 Serverless Messaging** | mDNS + DNS-SD로 발견, XML 스트림으로 대화. Apple iChat Bonjour(2002~), Gajim·Kopete 지원 | ★ **표준 프로토콜 선택지**. 상호운용 카드 |
| 18 | **AirDrop / Quick Share** | OS 내장 근접 공유 | "설정 없음" UX의 사용자 기대 수준 |
| 19 | **Snapdrop / PairDrop** | 웹 기반 — **시그널링 서버 필요** | 제로 컨피그처럼 보이나 서버 의존 = 우리가 피할 함정 |
| 20 | **Windows Messenger service / `net send` / WinPopup** | 레거시(Vista 이후 제거) | 상용 LAN 메신저 다수가 아직도 "WinPopup 대체품"을 자처 = **이 시장의 수요는 20년째 미충족** |

### 2-D. P2P 보안 메신저 (LAN 우선은 아님 — 보안 모델 참고)

| # | 제품 | 발견 방식 | 암호화 |
|---|---|---|---|
| 21 | **Briar** | Bluetooth · Wi-Fi · Tor · 이동식 저장소 (Bramble 프로토콜) | 전 구간 E2E |
| 22 | **Jami** | **OpenDHT + 로컬 네트워크 자동 피어 발견** | E2E · 음성/영상/화면공유 |
| 23 | **Tox / qTox** | DHT + LAN 발견 | E2E(메타데이터 노출 지적 있음) |
| 24 | **Berty** | BLE + IPFS | E2E · 인터넷 없이 동작 |
| 25 | **Ricochet Refresh** | Tor 직접 P2P | E2E · 서버 없음 |

---

## 3. 국내 — 전수 목록

> **핵심 발견**: 국내에는 **"제로 컨피그 P2P LAN 메신저" 상용/오픈소스 제품이 사실상 없다.**
> 국산 제품군은 전부 **서버 기반 그룹웨어 메신저**로 수렴했고, 그 빈자리를 **일본산 IP Messenger**가 20년 넘게 채우고 있다.

| # | 제품 | 개발사 | 구조 | 비고 |
|---|---|---|---|---|
| 26 | **IP Messenger 한국어 사용** | (일본산) | P2P·서버 없음 | 국내 사무실에서 **사실상의 표준** — "회사 안에서만 쓰는 메신저", 대용량 파일 전송·폴더 드래그가 호평 이유. 스마트폰 연동 없음·외부 사용 불가가 한계로 언급됨 |
| 27 | **하이웍스 메신저** | 가비아 | 클라우드 그룹웨어 | 조직도 기반(부서·직위 노출)·PC/모바일 |
| 28 | **다우오피스 PC메신저** | 다우기술 | 클라우드/설치형 그룹웨어 | 실시간 대화·파일 공유·공지·투표 |
| 29 | **잔디 (JANDI)** | 토스랩 | 클라우드 | 국내 개발·중소기업 강세 |
| 30 | **카카오워크** | 카카오 | 클라우드 | 카카오톡 유사 UI = 학습 비용 0 · 공공·금융 공략 |
| 31 | **네이버웍스** | 네이버 | 클라우드 | 메일·캘린더·드라이브 통합 |
| 32 | **NHN 두레이** | NHN | 클라우드/설치형 | 프로젝트 단위 권한 세분화 강점 |
| 33 | **더존 위하고 · 닥스웨이브** 등 | 각사 | 그룹웨어 | 메신저는 부속 기능 |
| 34 | **쿨메신저** | 쿨메신저 | 서버 기반 | 학교·학사업무 특화(교육 현장 점유) |
| 35 | **NopsUC** | — | 사내망 전용 | 기업 무료·일정/조직/파일전송/연락처 |
| 36 | **온-나라 메신저 → 온톡메신저 · 바로톡** | 행정안전부 | 폐쇄망·서버 | 공공 표준. 바로톡은 캡처 방지·다운로드 방지·암호화 등 보안 기능 강조 |
| 37 | **소프트메신저** (1997, 최초 국산 메신저 — 서비스 종료) | — | 쪽지 팝업 | ⚠️ **교훈**: 화면에 바로 펼쳐지는 쪽지 UX가 **스팸 폭탄 공격**에 악용되어 사용자 이탈 → 서비스 종료. **무인증 브로드캐스트 UX의 위험을 보여주는 국내 실사례** |

---

## 4. 기능 매트릭스 — "잘 쓰이는 인기 기능" 집계

> 열은 A계층 대표 6종 + 인접 1종. **○ = 있음 · △ = 부분/조건부 · ✕ = 없음 · ? = 미확인**.
> 마지막 두 열이 이 표의 목적 — **몇 곳이 갖췄나(보편성)** 와 **우리가 채택할 것인가**.

| 기능 | IPMsg | BeeBEEP | LANMsgr | Squiggle | Softros | KouChat | LocalSend | 보편성 | 채택 |
|---|:--:|:--:|:--:|:--:|:--:|:--:|:--:|:--:|:--:|
| **사전 등록 없는 자동 사용자 발견** | ○ | ○ | ○ | ○ | ○ | ○ | ○ | **7/7** | ★ 필수 |
| 1:1 즉시 메시지 | ○ | ○ | ○ | ○ | ○ | ○ | ○ | **7/7** | ★ 필수 |
| 파일 전송(드래그앤드롭) | ○ | ○ | ○ | ○ | ○ | ○ | ○ | **7/7** | ★ 필수 |
| **폴더 전송** | ○ | ○ | ? | ? | ○ | ? | ○ | 4/7 | ★ 채택 |
| 그룹/채팅방 | ? | ○ | △ | ○ | ○ | ○ | ✕ | 5/7 | ★ 채택 |
| 전체 브로드캐스트 공지 | ○ | ? | ○ | ? | ○ | ○ | ✕ | 4/7 | ★ 채택 |
| **오프라인/부재 시 메시지 큐** | ○(Send Queue) | ○ | ? | ? | ○ | ✕ | ✕ | 3/7 | ★ 채택 |
| 대화 기록 저장 | ○(10만건+ 뷰어) | ○ | ○ | ○(SQLite) | ○(검색·인쇄) | ○ | ✕ | **6/7** | ★ 필수 |
| 기록 전문 검색 | ○ | ? | ? | ○ | ○ | ? | ✕ | 3/7 | ★ 채택 |
| **암호화** | ○ RSA2048+AES256 | ○ AES(Rijndael) | ○ AES+RSA | ? | ○ AES-256 | ? | ○ HTTPS/즉석 TLS | 5/7 | ★★ 필수(사용자 확정) |
| 상태(자리비움·타이핑) | ? | ○ | ? | ? | ○ | ○ | ✕ | 3/7 | ★ 채택 |
| 트레이 상주·알림 팝업/사운드 | ○ | ○ | ○ | ○ | ○ | ○ | ○(`--hidden`) | **7/7** | ★ 필수 |
| 화면 캡처·주석 후 전송 | ○(그리기 편집) | ○ | ✕ | ○ | ✕ | ✕ | ✕ | 3/7 | 후순위 |
| 화면 공유·원격 지원 | ✕ | ○ | ✕ | ○ | ○(Remote Assistance) | ✕ | ✕ | 3/7 | 후순위 |
| 음성/영상 통화 | ✕ | ✕ | ✕ | ○(NAudio) | ○(다자 화상) | ✕ | ✕ | 2/7 | 범위 밖(1차) |
| **서브넷/라우터 너머 연결** | ○(Member Master) | ? | ✕ | ○(브리지) | ○(WAN/VPN/VLAN) | ✕ | △(IP 스캔 폴백) | 4/7 | ★ 채택(2차) |
| 다국어 | ○ | ○ | ○ | ○ | ○ | ○ | ○ | **7/7** | ★ 필수 |
| CLI/자동화·SDK | ? | ? | ✕ | ? | ○(CLI·SDK API) | ○(콘솔 모드) | △ | 3/7 | 후순위 |
| AD/Entra 연동·GPO 배포 | ✕ | ✕ | ✕ | ✕ | ○ | ✕ | ✕ | 1/7 | 범위 밖 |
| 관리자 감사 로그 | △(Pro) | ✕ | ✕ | ✕ | ○(정책 제한) | ✕ | ✕ | 2/7 | 범위 밖(1차) |
| 외부 연동(Slack 등) | ○(전달) | ✕ | ✕ | ○(번역) | ✕ | ✕ | ✕ | 2/7 | 범위 밖 |
| **포터블(무설치) 실행** | △ | ○(압축 풀고 실행) | ✕ | ? | ? | ? | ○(v1.13.0~) | 2/7 | ★★ 필수(사용자 확정) |
| **Windows ARM 지원** | ✕ | ✕ | ✕ | ○(ARM64) | ✕ | △(JVM) | △ | 1/7 | ★★ 필수(사용자 확정) |

**집계 결론 — 7/7 또는 6/7인 "당연히 있어야 하는 것" 6가지**: 자동 발견 · 1:1 메시지 · 파일 드래그 전송 · 대화 기록 · 트레이 상주 알림 · 다국어.
이 6개는 없으면 제품으로 취급받지 못하는 **입장권**이고, 차별화는 그 위에서 만들어야 한다.

---

## 5. 제품별 장단점

### A계층 (직접 경쟁)

| 제품 | 장점 | 단점 |
|---|---|---|
| **IP Messenger** | · **현역 중 가장 활발**(2026-07 갱신)·20년 넘게 유지된 신뢰<br>· 대용량 파일/폴더 전송 속도가 실사용 최고 평가<br>· RSA2048+AES256 적용<br>· 10만 건+ 로그 뷰어(인라인 이미지·탭·전문 검색)<br>· Member Master로 **라우터 너머 발견** | · **UI가 1990년대 Win32 그대로** — 현대 사용자 기준 매우 이질적<br>· Windows 중심, macOS는 **별도 앱**(동일 UI 아님)·Linux는 Wine<br>· 모바일 없음<br>· 국내 사용자 평가에서도 "스마트폰 연동 안 됨"이 반복 지적 |
| **BeeBEEP** | · **압축 풀고 실행 = 진짜 포터블**<br>· 지원 OS 폭이 비상식적으로 넓음(OS/2까지)<br>· 그룹·오프라인 메시지·화면 공유까지 기능 밀도 높음<br>· AES 암호화 | · **2023-01 이후 릴리스 없음** — 사실상 정체<br>· Qt 의존 → 배포 크기 큼<br>· UI가 플랫폼별로 Qt 테마를 타서 **동일하지 않음** |
| **LAN Messenger** | · Win/mac/Linux 3종 · AES+RSA<br>· 설치본 7.21MB로 가벼움<br>· 오픈소스 | · **개발 정체**(Windows 1.2.39 / 나머지 1.2.37로 버전조차 어긋남)<br>· 기능이 최소선(그룹·오프라인 큐 약함)<br>· 발견 메커니즘이 문서화되어 있지 않음 |
| **Squiggle** | · **유일하게 현대적 크로스플랫폼**(.NET 9 + Avalonia, **ARM64 포함**)<br>· 멀티캐스트 P2P·음성·화면 캡처·번역·**서브넷 브리지**<br>· MIT · 활발한 커밋 | · **.NET 런타임 의존** → 크기·메모리에서 우리 목표와 정면 충돌<br>· 암호화 문서화 미흡(공개 문서에 명시 없음)<br>· Avalonia = 프레임워크 UI라 우리 "자체 렌더링" 방침과 다른 노선 |
| **Softros LAN Messenger** | · **기능 완성도 1위**(브로드캐스트·오프라인·원격지원·다자 화상·CLI·SDK·AD/Entra·GPO)<br>· AES-256 · 서버 불필요 P2P<br>· Windows 지원 범위가 압도적(XP~11) | · **유료 상용**<br>· Windows/macOS/Android — **Linux 없음**<br>· 기업 IT 관리 기능 위주라 개인·소규모에는 과함<br>· 포터블 언급 없음 |
| **KouChat** | · 제로 컨피그 철학이 명확<br>· 데스크톱+Android<br>· LGPL | · **2016년 이후 사실상 멈춤**<br>· **암호화 없음**(문서상 확인 불가) → 사내 사용 불가<br>· Java/JVM 의존 = 메모리·크기 불리 |
| **Technitium Mesh** | · DHT 기반 P2P·**LAN 단독(인터넷 없이) 동작**·E2E 암호화<br>· GPL-3.0 | · 저활동<br>· Windows 중심<br>· 익명성 지향 설계가 사내 용도(누가 누군지 알아야 함)와 어긋남 |
| **LanTalk NET / Vypress / RealPopup / BORGChat** | · "설정 없음"·WinPopup 대체를 정면으로 내세움<br>· 예약 발송·알림 등 실무 편의 기능 | · **전부 Windows 전용**<br>· 대부분 유료·소규모 벤더<br>· UI·기술 스택이 오래됨 |

### B계층 (서버형) — 공통 장단점

| 장점 | 단점 |
|---|---|
| 조직도·권한·감사 로그·오프라인 보관 등 **관리 기능이 근본적으로 강함**. 서버가 있으니 이력·정책·백업이 자연스럽다 | **서버 구축·계정 발급·운영 인력이 전제** — 우리가 겨냥하는 "지금 당장 두 사람이 대화" 상황에서는 도입 자체가 불가능. Output Messenger·Bopup·BigAnt 모두 여기 해당 |

### C계층 — LocalSend의 교훈 (가장 참고할 제품)

| 장점 | 단점 |
|---|---|
| · **mDNS 우선 + 실패 시 IP 스캔 폴백** = 현실 네트워크에 대한 정직한 설계<br>· 기기마다 **즉석 생성 TLS 인증서**로 HTTPS — 서버 없이 암호화를 푸는 실증 사례<br>· **6개 플랫폼** 단일 코드베이스·Apache-2.0<br>· **포터블 모드**(실행 파일 옆 `settings.json`) — 우리 요구사항과 동일한 해법 | · **파일 전송 중심** — 대화(스레드·기록·상태)는 부속<br>· **Flutter/Dart 런타임** → 설치 크기·메모리가 네이티브 대비 큼<br>· 즉석 TLS = 암호화는 되지만 **상대가 진짜 그 사람인지는 보증하지 못함**(아래 §6-2) |

### D계층 — 보안 모델에서 가져올 것

Briar·Jami·Tox·Berty·Ricochet은 **중앙 서버 없이 E2E를 성립시킨 선례**다. 공통적으로 (1) 기기마다 **키쌍을 자체 생성**하고, (2) **공개키 지문(fingerprint)이 곧 신원**이며, (3) 최초 접촉 시 신뢰를 확정하는 절차를 UX로 밀어넣는다. 반면 DHT/Tor 기반 발견은 우리에게 과하다 — 우리는 발견 범위가 LAN으로 한정되어 훨씬 단순하다.

---

## 6. 시사점 — 우리 제품에 주는 결론

### 6-1. 시장의 빈칸이 명확하다

- 국내에는 제로 컨피그 P2P LAN 메신저가 **없고**, 그 자리를 **일본산 IP Messenger(Windows 중심·구식 UI·모바일 없음)** 가 20년째 대신하고 있다.
- 현대적 크로스플랫폼은 **Squiggle**(.NET+Avalonia)과 **LocalSend**(Flutter) 둘뿐인데, **둘 다 런타임 의존**이라 "최소 크기·최소 메모리"라는 우리 축에서는 비어 있다.
- 상용 LAN 메신저 다수가 아직도 **"WinPopup 대체품"** 을 마케팅 문구로 쓴다 = 이 수요는 20년째 제대로 채워지지 않았다.

> **⇒ 우리 좌표: "IP Messenger의 실용성 + LocalSend의 현대성"을, 런타임 없는 네이티브 단일 바이너리로.**

### 6-2. ★ 제로 컨피그와 암호화는 정면으로 충돌한다 — 이 프로젝트 최대 난제

사용자 확정 요구는 두 개고, 둘은 서로를 밀어낸다.

| | 요구 | 함의 |
|---|---|---|
| DR-1 | 사전 등록 없이 자동으로 뜨고 바로 대화 | **신뢰 앵커가 없다** — 인증서 발급자도, 계정도, 사전 공유 비밀도 없다 |
| 신규 | 단말 간 통신은 반드시 암호화 | 암호화하려면 **상대의 키가 진짜인지**를 어떻게든 판단해야 한다 |

암호화 자체는 쉽다(키쌍 자체 생성 + 세션 키 교환). 어려운 것은 **인증**이다. LocalSend의 "즉석 TLS 인증서"는 도청은 막지만 **중간자(MITM)는 막지 못한다** — 상대가 자기 인증서를 스스로 만들었으므로, 같은 LAN의 공격자도 똑같이 만들 수 있다. 경쟁 제품 대부분이 "AES-256 암호화"라고만 표기하고 **키 인증 방식은 문서화하지 않는데**, 이는 대체로 여기서 멈췄다는 뜻이다.

**해법 후보 3안** — ADR-0002에서 결정:

| 안 | 방식 | 장점 | 단점 |
|---|---|---|---|
| **① TOFU** (Trust On First Use) | 기기별 키쌍 자체 생성 → 첫 접촉 시 공개키 고정, 이후 키가 바뀌면 경고 | 사용자 개입 0 = **제로 컨피그 무손상** | 최초 접촉 시점의 MITM은 막지 못함 |
| **② TOFU + 지문 확인(SAS)** | ①에 더해 짧은 인증 문자열/이모지를 양쪽 화면에 표시, 사용자가 육안 대조 | 실질적 MITM 차단·Briar/Signal 계열 검증된 UX | 확인 절차가 "설정 없음" 감각을 조금 해침 → **선택적(중요 대화만)** 으로 완화 가능 |
| **③ 공유 비밀(사무실 코드)** | 조직 단위 사전 공유 키로 대화방 격리 | 강력·단순 | **사전 등록 없음(DR-1) 위반** → 기본값으로는 채택 불가, 옵션으로만 |

> **잠정 권고: ①을 기본, ②를 선택적 상향**. ③은 기업 옵션으로만 남긴다. 근거는 DR-1이 정체성이고, 정체성을 깨는 보안은 제품을 없애는 보안이기 때문이다. 확정은 [ADR-0002](10-decision-record.md)에서.

### 6-3. 발견(discovery)은 낙관하면 안 된다

기업 무선망의 **클라이언트 격리(client isolation)** 가 켜져 있으면 무선 단말은 L2 발견 프로토콜로 서로를 찾지 못하고, VLAN 분리 역시 멀티캐스트를 기본 차단한다. 즉 **"멀티캐스트 하나로 끝"이라는 설계는 실제 사무실에서 깨진다.**

- LocalSend가 **mDNS → IP 스캔 폴백**을 둔 이유, IP Messenger가 **Member Master(라우터 너머 발견)** 를 둔 이유가 정확히 이것이다.
- ⇒ **다단 발견은 선택이 아니라 필수 요구사항**이고, 설계 전에 [15 §1-3](15-dev-methodology.md) 규율대로 **스파이크로 실측**해야 한다.

### 6-4. 크기·메모리에서 이길 자리가 비어 있다

| 제품 | 런타임 | 크기·메모리 관점 |
|---|---|---|
| Squiggle | .NET 9 | 런타임 포함 시 수십 MB |
| LocalSend | Flutter/Dart | 런타임 포함 |
| BeeBEEP · LAN Messenger | Qt | Qt DLL 동반(LAN Messenger 설치본 7.21MB) |
| KouChat | JVM | 별도 설치 필요 |
| **Nexa Beep(목표)** | **없음** | ★ 유일한 자리 |

같은 조직의 [nexa-dir2](../../nexa-dir2/README.md)가 **단일 exe 3.65MB · 유휴 RSS 16.86MB**를 실증했다 — 이 축에서 우리가 이길 수 있다는 근거가 이미 사내에 있다.

### 6-5. 국내 실사례에서 얻는 경고 — 소프트메신저

1997년 최초 국산 메신저 **소프트메신저**는 "쪽지가 상대 화면 상단에 바로 펼쳐지는" UX가 강점이었으나, 바로 그 UX가 **스팸 폭탄 공격**에 악용되어 사용자가 대거 이탈하고 서비스가 종료됐다.

> ⇒ 우리의 "누구나 즉시 메시지를 보낼 수 있다"는 **정확히 같은 공격면**을 가진다. **속도 제한·차단 목록·미확인 발신자 격리**를 나중이 아니라 **1차 요구사항**에 넣어야 한다.

### 6-6. 1차 범위 권고

| 넣는다 (§4에서 5/7 이상 + 사용자 확정) | 뺀다 (1차) |
|---|---|
| 자동 발견 · 1:1 메시지 · 파일/폴더 드래그 전송 · 그룹 · 브로드캐스트 공지 · 오프라인 큐 · 대화 기록+검색 · 트레이 알림 · 상태/타이핑 · 다국어 · **E2E 암호화** · **포터블+설치본** · **Win/mac/Linux/WinARM** | 음성·영상 통화 · 원격 데스크톱 · AD/GPO · 감사 로그 · 외부 연동(Slack) · 모바일 앱 |

---

## 7. 출처

- 종합 목록·비교 — [TrueConf LAN Messenger 리뷰](https://trueconf.com/blog/reviews-comparisons/lan-messenger) · [medevel: 13 Open-source LAN Messengers](https://medevel.com/12-os-network-messangers/) · [AlternativeTo: LAN Messenger 대안](https://alternativeto.net/software/lan-messenger-2-/?license=opensource) · [Aniesoft Top 10](https://www.aniesoft.com/top-lan-messenger-free-window/)
- IP Messenger — [공식 사이트](https://ipmsg.org/) · [영문 도움말](https://ipmsg.org/help/ipmsghlp_eng.htm) · [포트 정보(IMFirewall)](https://www.imfirewall.com/en/protocols/IPMSG.htm) · [국내 사용기(바다야크)](https://badayak.com/entry/IP-Messenger)
- BeeBEEP — [공식 사이트](https://beebeep.sourceforge.io/) · [SourceForge](https://sourceforge.net/projects/beebeep/)
- LAN Messenger — [공식 사이트](https://lanmessenger.github.io/) · [SourceForge](https://sourceforge.net/projects/lanmsngr/) · [국내 소개(KOROMOON)](https://koromoon.blogspot.com/2020/07/lan-messenger.html)
- Squiggle — [GitHub hasankhan/Squiggle](https://github.com/hasankhan/Squiggle)
- KouChat — [공식 사이트](https://www.kouchat.net/)
- Technitium Mesh — [mesh.im](https://mesh.im/) · [GitHub](https://github.com/TechnitiumSoftware/Mesh)
- Softros — [공식 사이트](https://messenger.softros.com/)
- Output Messenger — [공식 사이트](https://lan-chat.srimax.com/) · [SoftwareSuggest 가격](https://www.softwaresuggest.com/output-messenger)
- LanTalk NET — [lantalk.net](https://www.lantalk.net/lan-messenger/)
- LocalSend — [공식 사이트](https://localsend.org/) · [GitHub](https://github.com/localsend/localsend) · [한국어 README](https://github.com/localsend/localsend/blob/main/readme_i18n/README_KO.md) · [동작 원리 해설](https://dev-post.com/localsend-guide/)
- XMPP 표준 — [XEP-0174 Serverless Messaging](https://xmpp.org/extensions/xep-0174.html)
- P2P 보안 메신저 — [Briar](https://briarproject.org/) · [Briar(위키백과)](https://en.wikipedia.org/wiki/Briar_(software)) · [Jami 로컬 피어 자동 발견](https://jami.net/automatic-peer-discovery-on-local-networks/) · [Tox 소개(gHacks)](https://www.ghacks.net/2020/03/25/tox-is-a-peer-to-peer-instant-messaging-protocol-with-end-to-end-encryption/)
- 국내 — [하이웍스 메신저](https://biz-solution.hiworks.com/product/function/messenger) · [다우오피스 PC메신저](https://helpdesk.daouoffice.co.kr/hc/ko/sections/42604387108505-PC-%EB%A9%94%EC%8B%A0%EC%A0%80) · [쿨메신저](https://school.coolmessenger.com/) · [국산 그룹웨어 비교](https://impactflow.kr/post/comparison-of-five-representative-groupware) · [소프트메신저·인스턴트 메신저 역사(나무위키)](https://namu.wiki/w/%EC%9D%B8%EC%8A%A4%ED%84%B4%ED%8A%B8%20%EB%A9%94%EC%8B%A0%EC%A0%80) · [온-나라 서비스(위키백과)](https://ko.wikipedia.org/wiki/%EC%98%A8-%EB%82%98%EB%9D%BC_%EC%84%9C%EB%B9%84%EC%8A%A4) · [바로톡 관련(행정안전부)](https://www.mois.go.kr/frt/bbs/type010/commonSelectBoardArticle.do?bbsId=BBSMSTR_000000000008&nttId=109593)
- 네트워크 제약 — [Cisco Meraki 무선 클라이언트 격리](https://documentation.meraki.com/Wireless/Operate_and_Maintain/How_Tos/Firewall_and_Traffic_Shaping/Wireless_Client_Isolation) · [UniFi 브로드캐스트 트래픽 관리](https://help.ui.com/hc/en-us/articles/27384925962647-Managing-Broadcast-Traffic-with-UniFi)
