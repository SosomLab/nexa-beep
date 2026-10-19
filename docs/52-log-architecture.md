# 52 · 로그 아키텍처 — 개발자 상세 로그(기능·전달인자·판단 기준/값·실패 원인) · nexa-sql docs/48 이식

> **상태**: 📐→✅ 1차 구현(10-19 · 사용자 요청 *"nexa-sql을 참고해서 로그 기능 구성 · 개발자 레벨에서는 상세한 로그 — 기능, 전달인자, 판단 기준 및 판단 값, 실패 시 실패 여부 및 실패 원인"*).
> 관련: [nexa-sql docs/48](../../nexa-sql/docs/48-logging-architecture.md)(원본 설계 · 저비용 게이트 deep research) · [TODO M3-22](TODO.md)(상태 로그 — 이 문서의 **싱크**) · [13 §7 마스킹](13-code-design-standards.md) · [39 netmon 기록부](39-netmon-records.md)(계측은 횟수만 — 봉투 원리) · [49 점검 체계](49-inspection-framework.md).

## 0. 결론 여섯 줄

1. **두 겹이다** — 아래층 = 기존 **상태 로그(M3-22 · `statuslog.rs`)**: 상태바 문구를 bounded 큐·전용 스레드·일자 파일(`data/logs/beep-YYYYMMDD.log`)로 남긴다(기본 off · `log.enabled`). 위층 = **개발자 상세 로그(이 문서)**: 판단 레코드를 **같은 파일에 `dev ` 접두로** 섞어 넣는다. 새 스레드 0 · 새 파일 0 · 새 설정 2개.
2. **끄면 비용 = 원자 정수 load 1회 + 분기 1개**(nsql-log 그대로) — `nbeep_core::devlog::wants(layer, level)` `#[inline(always)]` · 마스크 `u64`(층 10 × 수준 4). 레코드 조립(문자열·할당)은 `dlog!` 매크로의 게이트 **뒤**에서만.
3. **레코드 = 한 줄 `k=v`** — `층/수준 기능 인자… | rule 기준=값 … | → 결과` 또는 `| FAIL 원인`. 사용자가 요구한 네 요소(기능·전달인자·판단 기준/값·실패 여부/원인)가 자리로 고정돼 있어 grep·필터가 선다.
4. **봉투 원리**: 레코드는 **지문 short·개수·길이·판정값**만. 메시지 본문·파일명·경로·키·암호는 넣지 않는다(파일명은 글자 수 · 설정값은 `pass`/`secret`/`key`가 든 키면 길이만).
5. **켜는 길 둘** — 설정 `log.dev_mode`(on · `log.enabled` 종속) + `log.dev_layers`(층 문법 · 빈 값 = 전부) → 파일 / 환경변수 `NEXA_DEVLOG=<층 문법|1|*>` → **stderr 거울 + 파일(켜져 있으면)**. 환경변수가 설정을 이긴다(실기 도구 `NEXA_SCRIPT`와 짝).
6. **커버리지**: 게이트는 `wants` 한 함수 · 매크로 한 곳 — 테스트가 마스크를 켜고/끄고 두 갈래를 다 밟는다(`mask_parse_and_gate_both_branches`). 계측 지점은 §3 표(12곳 · 10-19 1차)가 SSOT.

## 1. 모델 — 층 × 수준

| 층(`Layer`) | 무엇 | 1차 계측 지점 |
|---|---|---|
| `app` | 부팅·설정 적용·마스크 변경 | `apply_setting`(키·값 — 비밀은 길이만) · `devlog`(마스크) |
| `disc` | 발견 등장·개명·이탈 | `discovery`(observe 변화 · goodbye · 행 유지 판정) |
| `conn` | 연결 사다리·재연결 백오프 | `start_connect`(래치·발견·수동·릴레이·psk) · `connect_failed`(사유·도달 가능성·단계·다음 지연) |
| `sess` | 세션 성립·TOFU·종료 | `outbound_established`(경로 등급·서버·psk) · `tofu`(판정/거절) · `closed`(형제·대기 큐·행 유지) |
| `chat` | 발신·수신·dedup·대기 | `send_text`(살아 있는 기기 수 → 전송/대기) · `recv`(dedup · 공지 무시) |
| `xfer` | 파일 오퍼 판정 | `offer_judge`(신뢰·왕래·정책·경로 → Accept/Ask/Deny 사유) |
| `user` | 사용자 인증·형제 | `user_test`(핸들 유무 · 키 생성 · 결과/원인) — 따라잡기는 `NEXA_USER_TRACE` 기존 줄 유지 |
| `relay` | 서버 attach | `server_attach`(주소 · 첫 핀 · 핀 쓰기 실패 / 실패 원인) |
| `store` | 기록·설정 영속 | `record_history`(바이트 · 원천 · 봉인 실패) |
| `ui` | 창·뷰·IME | (후속 — `NEXA_WIN_TRACE`·`NEXA_IME_TRACE`가 이미 있음) |

| 수준(`Level`) | 뜻 | 양 |
|---|---|---|
| `basic` | 상태바 문구(마스크 밖 · `log.enabled`면 늘) | 사건당 1줄 |
| `decide` | **판단**: 기능 + 인자 + 기준·값 + 결과/실패 원인 | 기능당 1~2줄 |
| `detail` | 중간값·개수·바이트 | 단계마다 |
| `trace` | 호출 단위(수신 프레임마다) | 많음 — 켤 때만 |

마스크 = `1 << (layer*4 + level)` · 설정 문법 `layer[:level+level]` 쉼표 · `*`/`all`/`1`/`on` = 전부 · 예 `conn,xfer:decide,user:decide+detail`.

## 2. 코드 경로

```rust
// nbeep-core/src/devlog.rs — 게이트(뜨거운 경로에 남는 전부) + 레코드(순수 · 의존 0)
static MASK: AtomicU64;
#[inline(always)] pub fn wants(layer, level) -> bool { MASK.load(Relaxed) & bit(layer, level) != 0 }
pub struct Rec { layer, level, func, args, rules, outcome }   // .arg(k,v) .rule(k,v) .ok(what) .fail(why) .render()

// nexa-beep/src/app.rs — 호출 지점(매크로 · 게이트 뒤에서만 평가)
dlog!(self, DL::Conn, DV::Decide, "connect_failed", |r| r
    .arg("peer", peer.short()).arg("why", &why).arg("stage", stage)
    .rule("reachable(manual|discovered|relay)", reachable)
    .rule("backoff_ms", delay)
    .fail("backoff exhausted — auto retry stopped"));
// 느린 경로(호출 드묾 · 코드 배치 분리)
#[cold] #[inline(never)] fn devlog_push(statuslog, stderr, rec) { line = rec.render(); eprintln!("[dev] …") · statuslog.log("dev …") }
```

- 싱크 = `StatusLog::log`(논블로킹 · 큐 포화 = 드롭 계상 · 2초 flush · 종료 flush · 날짜 회전·보존·총량은 기존 `log.retain_days`/`log.max_total_mb` 공유).
- 마스크 갱신 `refresh_devlog()` = 부팅(`refresh_statuslog` 직후) · `log.*` 설정 변경. `log.enabled`가 꺼지면 설정 경로 마스크는 0(싱크 없음 = 비용 0). 환경변수 경로는 stderr가 싱크라 파일 설정과 무관.
- 설정 = `log.dev_mode`(Toggle · 기본 off · `log.enabled` 종속) · `log.dev_layers`(Text · 기본 빈 = 전부 · `log.dev_mode` 종속) — 고급 › 로그. 4개 국어.

## 3. 읽는 법 — 한 줄의 자리

```text
[11:32:07] dev conn/decide start_connect peer=b7147214 auto=false discovered=false manual_addr=false relay=true psk=false | rule latch=free | → dial: direct → manual → relay
[11:32:08] dev conn/decide connect_failed peer=b7147214 why="NotFound" stage=0 | rule reachable(manual|discovered|relay)=true backoff_ms=2000 | → retry in 2s
[11:32:08] dev chat/decide send_text peer=b7147214 chars=12 grade=0 | rule live_devices=0 queue_len=1 | → queued offline (M4-6) — delivered when peer appears; connect scheduled
[11:40:21] dev xfer/decide offer_judge peer=5016194e name_len=9 size=1048576 path=Local | rule sibling=false trust=Pinned mutual_exchange=false policy=Basic(Ask) | → Ask
[11:40:22] dev sess/decide tofu peer=5016194e | rule trust_store=reject | FAIL key changed for pinned peer
```

- `[HH:MM:SS]` = 상태 로그 스레드가 찍는 기록 시각(상태바 줄과 같은 시계) · `dev` = 이 문서의 줄 · 그 뒤가 `Rec::render()`.
- **실패 찾기** = `grep "FAIL"` · **기능별** = `grep "conn/"` · **한 상대** = `grep "peer=b7147214"`.
- 값에 공백·`=`·`|`가 있으면 `"…"`로 감싸고 개행은 `␤` — **한 줄 = 레코드 하나** 불변.

## 4. 부하 점검(DR-5 게이트)

| 항목 | 끔 | 켬(decide) | 켬(trace) |
|---|---|---|---|
| CPU | 지점당 load+분기 ≈ 0.3ns × 12곳 | 사건당 문자열 1줄 ≈ 1µs + 큐 push | 수신 프레임마다 1줄(dedup 드롭 포함) |
| 메모리 | 0 | 큐 256줄 상한(포화 = 드롭) | 같음 |
| 스레드·파일·네트워크 | 0 | 0(기존 statuslog) | 0 |

## 5. 후속

- **로그 창**(nexa-sql `log_win` 상당 — 링 버퍼 · 종류/층 필터 · 줄바꿈 · 복사): 지금은 `log.view`(OS 기본 프로그램으로 파일 열기)로 본다. 창이 필요해지면 `Role::Log` + 링 버퍼(`log.max_lines`)를 얹는다 — 생산자는 같은 `devlog_push`.
- **계측 확장**: 따라잡기(`[user]` eprintln 14곳 → `User/Detail`로 승격 · 시나리오 도구가 stderr를 grep하므로 eprintln은 유지) · 파일 전송 단계(`Xfer/Detail` 청크·속도·재개) · UI(`Ui/Trace` 창 생성·포커스) · 릴레이 roster.
- **완전 지연 포맷**(docs/48 §5): `Rec`를 구조체 그대로 링에 넣고 텍스트는 소비 시점에 — 켠 상태의 문자열 비용까지 0.
- **컴파일 타임 제거**: feature `devlog` off면 `wants()` = `false` 상수.
