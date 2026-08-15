# 36. 알림(Notification) 아키텍처 — 개념·구조·OS 분기·동작 순서

> **상태: 구현 문서(M3-8 최소 슬라이스 · 2026-08-15).** 등급·강등표·수신자 릴레이는
> [24 ADR-0010](24-adr-0010-message-priority-notification.md)(🔴 D-23 확정 대기) 몫이고,
> 이 문서는 **그 밑에서 이미 돌아가는 표시 계층**의 정확한 구조와 동작을 기록한다.
> 관련 결정: **DR-25**(등급 = 발신자 요청 / 강도 = 수신자 판정 / **미검증은 소리 없음**) ·
> DR-21(외부 기술은 이음새 뒤에) · FR-U-15~17 · FR-S-41/42.

## 1. 개념 — 알림은 "화면 밖의 배지"다

앱이 **화면에 없을 때**(다른 앱에 포커스·창 숨김) 수신 사실을 OS 채널로 알리는 계층이다.
앱이 화면에 있을 때의 알림은 이미 다른 것이 맡고 있다 — 읽지 않음 배지(목록 행) ·
창 제목 카운트 · 상태바. 그래서 이 계층의 첫 번째 규칙은:

> **앱의 어느 창이든 포커스를 갖고 있으면 OS 알림은 침묵한다.**

두 번째 규칙은 신뢰 게이트다(DR-25 — 이미 확정): **알림의 강도는 발신자가 아니라
수신자(의 신뢰 판정)가 정한다.** v1 최소 구현에서 강도 축은 "소리 유무" 하나 —
미검증(Unverified) 발신자는 배너만 뜨고 소리가 없다.

세 번째 규칙은 내용 정책이다(FR-S-41/42): 알림은 잠금 화면·화면 공유에 노출되는
채널이므로 **기본은 내용 비노출**("새 메시지"만), 옵트인 시에도 무해화·절단을 거치고,
**파일명은 어떤 경우에도 싣지 않는다.**

## 2. 구조 — 두 층 + 이벤트 소스 셋

```
  [이벤트 소스]                [정책층 · app.rs]                 [어댑터층 · OS]
  1:1 수신(AppEvent::Recv) ─┐
  그룹 수신(SGroupMsg::Msg) ─┼─▶ notify_user(key, 제목, 본문, 무음)
  파일 오퍼(XferOffer·Ask) ─┘      │ ① notify.enabled == on ?
                                   │ ② 앱 전 창 비포커스 ?
                                   │ ③ 키별 3초 스로틀 통과 ?
                                   ▼
                       ┌─ Windows ─ TrayHandle::notify → 트레이 풍선(NIF_INFO)
                       ├─ macOS ──── nbeep_plat::notify → osascript 스폰
                       └─ Linux ──── nbeep_plat::notify → notify-send 스폰(fail-soft)
```

### 2-1. 정책층 (`app.rs` — `notify_user` / `notify_body`)

**판정은 전부 여기서 끝난다** — 어댑터는 "띄우라면 띄우는" 표시 기계다(파일 오퍼
판정을 앱 한 곳에 모은 것과 같은 결 · 정책이 두 벌로 갈라지지 않는다).

| 판정 | 규칙 | 근거 |
|---|---|---|
| 켜짐 | `notify.enabled`(기본 on) | 사용자 소유권 |
| 포커스 | 앱의 **어느 창이든** `has_focus()`면 침묵 | 배지·제목이 맡는 중(§1) |
| 스로틀 | 키(`p:{peer}` / `g:{gid}` / `x:{peer}`)별 **3초** | 폭주 방지(NFR-B 결) |
| 무음 | 발신자 `TrustLevel == Unverified` → `silent` | **DR-25 신뢰 게이트** |
| 본문 | `notify.preview` off(기본) = "새 메시지" · on = 무해화 본문 80자 절단 | FR-S-42(화면 공유 안전) |
| 제목 | 1:1·파일 = 상대 표시 이름 · 그룹 = **방 이름** | 발신 맥락 식별 |
| 파일 오퍼 | 본문 = "파일 수신 요청" 고정 — **파일명 금지** | FR-S-41 금지 목록 |

### 2-2. 어댑터층 — OS별 분기와 그 이유

| OS | 방식 | 왜 이 방식인가(대안 탈락 근거) | 클릭 = 앱 열기 |
|---|---|---|---|
| **Windows** | **트레이 풍선** — `TrayHandle::notify` → `WM_APP_BALLOON` → 트레이 스레드가 `Shell_NotifyIconW(NIM_MODIFY, NIF_INFO)` | 정식 WinRT 토스트는 **AppUserModelID(시작 메뉴 바로가기) 요구** = 설치본 전용 → T0·포터블(DR-4) 불합. 트레이 아이콘(M3-2a)이 이미 떠 있어 자산 재사용 · 의존 0 | ✅ `NIN_BALLOONUSERCLICK`(0x405) → `TrayEvent::Open`(트레이 좌클릭과 같은 복원 경로) |
| **macOS** | **이원화(08-15 2차 — 사용자 확정 "정식 알림")**: ① **번들(.app) 실행 = `UNUserNotificationCenter` 정식 경로** — `objc2-user-notifications`(keytap과 같은 objc2 판) · 부팅 시 `notify::init`(번들 판정 = `NSBundle.bundleIdentifier`) → 권한 요청 + 클릭 delegate(`NbeepNotifyDelegate` — didReceive → 창 복원) · 알림 소유 = 우리 앱 ② **비번들(포터블·개발) = `osascript` 스폰 폴백** | UN은 **번들 신원(Info.plist) 필수**라 이원화가 구조적 정답. 서드파티(terminal-notifier) 의존 탈락 | 번들 = ✅ delegate · ★**Cask 실증 완료(08-15 사용자 확인)** — 무서명(ad-hoc)이어도 **정식 설치(/Applications · LS 등록)면 권한·배너 정상**(아이콘 포함 · /tmp 임시 번들 미표시는 위치·등록 변수였음) · 클릭-투-오픈 육안은 잔여 / 비번들 폴백 = ❌(소유자 = Script Editor) |
| **Linux** | `notify-send` 스폰(libnotify 도구 · 대부분의 데스크톱 존재) — 없으면 조용히 실패(**fail-soft**: 알림은 보조 채널) | zbus 직접 D-Bus 호출 대비 코드 최소 · 데몬 부재 환경에서도 무해 | ⏸ `notify-send -A`(0.7.10+ · 클릭 액션 대기) 검토 잔여 — 버전 의존이라 보류 |

**무음(silent)의 OS별 구현**: mac = `sound name` 절 생략 · Windows = `NIIF_NOSOUND`
플래그 · Linux = `--hint=boolean:suppress-sound:true`(데몬 재량 · 미지원 무해).

## 3. 동작 순서 (시퀀스)

**표시 경로** — 1:1 메시지 기준(그룹·파일 오퍼도 소스만 다르고 동일):

1. 세션 액터가 복호·검증을 끝낸 수신을 `AppEvent::Recv`로 메인에 넘긴다.
2. 메인: dedup(중복 창) → 왕래 장부·최근 대화 기록 → **미리보기 문자열을 여기서
   확정**(`notify_body` — 본문이 ChatLine으로 move되기 전) → 스레드·배지 계상.
3. `notify_user` 판정 사다리(§2-1 ①→②→③) — 하나라도 걸리면 조용히 종료.
4. 스로틀 시각 기록 후 OS 분기: Windows는 트레이 핸들이 있으면 풍선(없으면 plat
   어댑터 = no-op), 그 외 OS는 `nbeep_plat::notify::notify` 스폰. 반환값은 버린다
   (**fail-soft** — 알림 실패가 수신 처리를 막으면 안 된다).

**클릭 경로**(Windows 풍선 + mac 번들 UN · 08-15 2차 — "클릭 = 해당 대화까지"):

1. 알림 발신 시 **불투명 대상 토큰**(= 알림 키 `p:…`/`g:…`)을 실어 보낸다 —
   mac = UN 요청 identifier(`"{일련}|{토큰}"` · `parse_target`과 한 쌍) ·
   Windows = 풍선 표시 시 `LAST_TARGET`(아이콘당 풍선 1개라 마지막이 곧 화면).
2. 클릭 → mac delegate `didReceive`가 identifier에서 토큰 파싱 / Windows
   `NIN_BALLOONUSERCLICK`이 `LAST_TARGET` 회수 → `TrayEvent::OpenTarget(토큰)`.
3. 메인: 창 표시+포커스 후 **`notify_targets` 맵으로만 토큰 해석**(OS에는 불투명 —
   봉투 원리) → `Peer` = `activate`(단일 모드 = 메인 안 그 대화로 전환 · 분리 모드 =
   대화 창 열어 마지막 스레드 — DR-26이 이력 유지) · `Group` = `open_group_thread`.
   맵 미스(재시작 등) = 메인 표시만(안전 폴백). 알림 클릭은 **명시적 사용자 행위**라
   "자동 열림 금지" 규칙과 충돌하지 않는다.

## 4. 설정

| 키 | 종류 | 기본 | 뜻 |
|---|---|---|---|
| `notify.enabled` | Toggle | **on** | 데스크톱 알림 전체 스위치 |
| `notify.preview` | Toggle | **off** | 본문 미리보기(off = "새 메시지"만 · 파일명은 on이어도 금지) |

둘 다 대화(Conversation) 카테고리 · 핫 스왑(판정이 사용 시점 읽기라 배선 불요).

## 5. 보안 점검 (봉투 원리)

- **이 계층의 봉투** = "누가(표시 이름/방 이름) · 무슨 종류(메시지/파일)". 내용은
  기본 미포함, 옵트인 시에도 무해화 텍스트뿐. 파일명·이미지·링크 금지(FR-S-41).
- **인젝션**: mac 경로는 수신 본문이 AppleScript 문자열에 들어간다 — 따옴표·역슬래시
  이스케이프가 없으면 **수신 메시지가 임의 osascript 실행**이 된다.
  `escape_applescript` + 회귀 테스트가 이를 고정한다(스폰은 no-shell이라 셸 이스케이프
  불요 — `Command` 인자 전달).
- 알림은 외부로 나가는 경로가 아니다(로컬 OS 표시) — FR-S-20 예외 심사 비대상.
  단, **수신자 릴레이**(외부 채널 팬아웃)는 전혀 다른 축이다 → [24 §5](24-adr-0010-message-priority-notification.md).

## 6. 한계와 잔여 (정직하게)

| 항목 | 상태 |
|---|---|
| 등급 3종(Normal/Notice/Urgent)·강등표·`Urgent` 24h 카운트 | 🔴 **D-23 확정 → M2-4b**(와이어 자리와 한 몸 — 앞지르면 포맷 변경) |
| 수신자 릴레이(Webhook 등 외부 팬아웃) | D-23 ⓑ + SP-2(M3-10) |
| mac 정식(UN) 경로의 권한 | ✅ **Cask 실증 완료(08-15 사용자 확인)** — 무서명이어도 정식 설치면 정상. ⚠ 번들을 임시 경로에서 직접 실행하면 권한이 프롬프트 없이 보류된다(개발 시 함정 — `open`+정식 위치로) |
| mac 비번들 폴백의 소속 표기 | Notification Center에 "Script Editor"로 뜬다(osascript 소유 — 폴백 한계) |
| Linux 실기 · 클릭 액션(`-A`) | 데스크톱 환경 잔여 |
| Windows 풍선·클릭 실기 | Windows PC 잔여(WNOTI — 26 §7 결) |
| 소리 포트 정식 3어댑터(winmm/NSSound/D-Bus) | DR-25 명세 잔여 — 지금은 OS 알림 채널의 기본 소리만 |
| 잠금 화면 정책 | OS 위임(내용 비노출 기본이 방어) — 자체 제어는 등급과 함께 |

## 7. 실측 기록 (2026-08-15 · macOS)

같은 PC 3신원 + CLI 발신으로 3중 확인: ① 앱 창 포커스 중 발신 = **억제**(배지·제목만)
② Finder로 디포커스 후 발신 = 배너 "★테스트단말 — New message"(미리보기 off 동작)
③ 발신자 = CLI 임시 신원(미검증) = **무음 배너**(신뢰 게이트). 알림 클릭 무반응의
원인(= Script Editor 소유)도 이 실기에서 확정 — §2-2 mac 열의 탈락 근거가 됐다.

---

> 관련: [24 ADR-0010](24-adr-0010-message-priority-notification.md)(등급·릴레이 — D-23) ·
> [10 DR-25·DR-21](10-decision-record.md) · [05 FR-U-15~17·FR-S-41/42](05-requirements.md) ·
> 트레이 = [journal/2026-08-15](journal/2026-08-15.md) M3-2 절.
