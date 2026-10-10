# 51 · 성능 검토 2026-10-10 — 대화 스크롤·우클릭 메뉴 "느림" 실측과 처방 + 성능 척도(dir3·sql 대조)

> **상태**: 🚧 1차(mac Intel Retina · 개발 세션 10-10) — 실측·처방 4건 적용 · 척도 16항목 등재 · **Windows 실측 ✓(10-10 Win 1차 · §3-1 · 프레임 ≈2ms)** · 잔여 = Linux 동일 시나리오 실측 · 더티 영역 present · RL-10.
> **왜**: 사용자 10-10 보고 "대화 내용 스크롤이 너무 느리다 · 우클릭 메뉴가 뜨는 것도, 메뉴 호버 추적도 너무 느리다 · 성능 6축(UX·전환·실행·갱신/스크롤·메모리 회수·파일 I/O)을 dir3·sql 척도와 함께 검토해 달라".
> **원칙**: 추정 금지·실측 필수([CLAUDE.md §2](../CLAUDE.md)) — 계측 seam을 먼저 넣고(§2) 숫자로 원인을 가른 뒤 고쳤다.
> 관련: [05 NFR-B](05-requirements.md) · [49 점검 체계](49-inspection-framework.md) · [26 §3-9 자동화 seam](26-run-and-manual-test.md) · [TODO RL-9·RL-10](TODO.md) · nexa-sql `docs/71`(성능 검토 절차) · `docs/62`(mac present).

## 1. 결론 한 장

| 증상 | 실측 원인(근거) | 처방(커밋) | 효과(실측) |
| --- | --- | --- | --- |
| 스크롤·호버·우클릭 전부 느림 | **프레임 = 전체 창 재래스터 + softbuffer CoreGraphics present**(프레임마다 새 버퍼 할당·DeviceRGB 색 변환) — 1600×1200에서 present **15~17ms** | mac present = **IOSurface**(`present.rs` · nexa-sys `LayerPresenter` · 표면 풀 3장) + **유휴 풀 해제**(nexa-ui 193차 `trim_idle`) | present **16 → 0.9ms** · 프레임 28~30 → ≈8.5ms |
| 스크롤이 기록 길이에 비례해 느림 | `ChatViewWidget::paint`가 **보이든 말든 기록 전체**(≤2000줄)를 프레임마다 글자 단위 `text_width(&c.to_string())`로 재측정·줄바꿈(캐시 0) | **줄바꿈 캐시**(줄별 본문·폭·글꼴 키) + `text_prefix_widths` O(n)(`48d3392`) | paint 12.9 → 7.5ms(30줄 · 긴 기록일수록 이득 큼) |
| 호버 추적 느림 | 호버 행 변경 = 위젯 전체 무효화 = 전체 프레임(위 두 항이 비용) · 호버 불변 MouseMove가 본문으로 새어 처리 | 위 두 처방 + 메뉴 열림 중 MouseMove/MouseUp 흡수 | 프레임 ≈ 9ms |
| 마우스 휠 이동량이 작음 | 노치 = 물리 40px 고정 → Retina에서 논리 20pt(메시지 한 줄 22pt 미만) | `MouseWheel` LineDelta × 창 배율 | 노치당 40pt |
| 우클릭 전 클립보드 읽기 | 의심됐으나 실측 **0.06ms**(첫 1회 9.6ms) | 변경 없음 | — |
| 유휴 CPU | 캐럿 530ms 틱이 포커스 창을 **무조건** 재도색 → 목록 창 유휴 12s에 26프레임(≈2% CPU · RL-9) · IOSurface 유휴 해제도 못 걸림 | `caret_asked`(nexa-ui 194차) — 캐럿을 그린 창만 틱 재페인트 · 목록 회전 링은 연결 중 행 있을 때만 질의 | 유휴 프레임 0(아래 §4) |

## 2. 계측 seam(이번에 신설 · 영구)

| seam | 용도 | 비용(꺼짐) |
| --- | --- | --- |
| `NEXA_PAINT_TRACE=1` | 프레임마다 `[paint] 역할 WxH paint=ms present=ms`(stderr) | 0(OnceLock) |
| `NEXA_SCRIPT` `wheel=<dy>` · `move=<x>,<y>` · `click=` · `rclick=` | 포커스 창에 **실제 `route` 경로**로 입력 주입(물리 px) — macOS는 밖에서 입력을 넣기 어렵다 | 0 |
| `NEXA_SCRIPT` `license` · `about` | 창 열기 단계 추가 | 0 |
| `NEXA_MAC_PRESENT=softbuffer` | mac present 종전 경로 강제(A/B · 회귀 비교) | 0 |
| `footprint <pid>` | mac phys_footprint(Apple 공식 · [18](18-build-and-test.md)) | — |

시나리오(재현 스크립트 = **[`tools/perf-scenario.sh`](../tools/perf-scenario.sh)**(10-10 Win 1차 `032c267` — mac 1차는 세션 scratchpad `perf-run.sh`였다 · 3-OS 분기·집계 awk) · 절차 요약 [26 §3-9](26-run-and-manual-test.md)): 3신원 사본 A가 B와 대화 열기 → 30줄 전송(120ms 간격) → 휠 20회(80ms) → 이동 10회 → 우클릭 → 메뉴 안 이동 10회 → quit. 창 = 분리 대화 창 1600×1200 물리(800×600 논리) · Intel mac Retina.

## 3. 실측 경과(분리 대화 창 · 30줄 · ms)

| 단계 | 구간 | paint 평균/최대 | present 평균/최대 | 비고 |
| --- | --- | --- | --- | --- |
| 기준선(ec59317) | 휠 | 12.9 / 15.3 | 16.5 / 22.1 | 프레임 ≈ 29ms · 휠 20회 중 10프레임 — ⚠ 10-10 Win 정정: 프레임 수는 **절반 이상 seam 아티팩트**(아래 주석) |
| | 이동·우클릭 | 11.4~11.6 | 14.5~15.3 | |
| 줄바꿈 캐시(`48d3392`) | 휠 | 7.5 / 8.8 | 16.1 / 17.7 | paint −42% · present 불변 |
| IOSurface(`present.rs`) | 휠 | 7.4 / 8.4 | **0.98 / 1.3** | present −94% · 프레임 ≈ 8.5ms |
| | 메인 목록 창 | 9.9 / 17.2 | 2.1 / 4.7 | 948×1378 |
| IOSurface + 유휴 해제 + 캐럿 틱 정밀화 | 휠 | 7.1 / 8.5 | 0.9 / 1.1 | **프레임 ≈ 8ms** · 유휴 프레임 0(§4) |

> ⚠ **프레임 수는 성능 근거가 아니다**(10-10 Win 1차 정정): `NEXA_SCRIPT` 단계는 이벤트 루프 틱(≈200ms · RL-10 `ControlFlow` 바닥)에 묶여 실행된다 — Win 실측 `[script]` 시각이 ≈200ms 단위로 몰려 80ms 간격 휠 20회가 프레임 1.8ms인데도 **9프레임**이 됐다. 따라서 "휠 20회 → 10프레임 = 입력이 밀린다"는 mac 코얼레싱과 seam 묶음이 섞인 값이다(OS 실입력은 `WindowEvent`로 즉시 깨우므로 실사용과 무관). 근거는 **paint·present ms 자체**로만 둔다.

### 3-1. Windows 실측(10-10 Win 1차 · `tools/perf-scenario.sh` · 분리 대화 창 800×600 물리 100% · softbuffer GDI present)

| 단계 | n | paint 평균/최대 | present 평균/최대 |
| --- | --- | --- | --- |
| 30줄 전송 | 48 | 1.35 / 3.63 | 0.41 / 3.45 |
| 휠 20회 | 9 | 1.76 / 2.84 | 0.23 / 0.35 |
| 이동·우클릭·메뉴 호버 | 8 | 1.1~1.4 / 1.97 | 0.23 / 0.39 |
| 메인 목록 창 474×689 | 6 | 3.3 / 5.7 | 0.36 / 0.52 |

→ **프레임 ≈2ms**(면적이 mac Retina 1/4 · GDI present 0.2~0.4ms = sql 65 실측과 같은 축) — Windows는 문제 없음(PERF-1 Win ✓ · Linux 잔여). 유휴(35s) = WS-Private 6.3MB · 유휴 프레임 3/45s(기동뿐 = RL-9 Win 성립).

메모리(phys_footprint · 메인 창만 유휴 / 메인+대화 창):

| 경로 | 유휴 | 대화 창 열림 | 판정(DR-5 유휴 ≤30MB) |
| --- | --- | --- | --- |
| softbuffer | 20MB | 29MB | ✓ |
| IOSurface(풀 3장 상주) | 36MB | 53MB | ✗ (+16MB = 5.2MB×3) |
| IOSurface + `trim_idle`(캐럿 틱 정밀화 전) | 35MB | 49MB | ✗ — 틱 재도색 때문에 "유휴"가 없었다(§1 마지막 행) |
| IOSurface + `trim_idle` + `caret_asked` | **25MB** | **41MB** | ✓ |

## 4. 최종 실측(IOSurface + `trim_idle` + `caret_asked` · 같은 시나리오)

| 항목 | 값 | 기준선 대비 |
| --- | --- | --- |
| 휠 스크롤 프레임 | paint 7.1 / 8.5 · present 0.9 / 1.1 → **≈8ms** | 29 → 8ms(−72%) |
| 이동·우클릭·메뉴 호버 | paint 7.2~7.6 · present 0.9 | 27 → 8ms |
| 메인 목록 창 프레임 | paint 10.5 · present 2.9 | present 10.6 → 2.9 |
| **유휴 프레임(메인 · 12s)** | **4**(기동 프레임뿐 · 이후 0) | 26 → 0 (캐럿 틱 재도색 제거 · 유휴 CPU ≈2% → 0) |
| **유휴 footprint(메인 창만)** | **25MB** | softbuffer 20 · IOSurface 풀 상주 36 → 25 = DR-5 ≤30MB ✓ |
| 대화 창 열림 footprint | 41MB(피크 46) | softbuffer 29 · 상주 53 → 41(활성 예산 80MB ✓) |
| 산출물 | 3.5MB | ✓ |

판정: **D-34 채택 근거 충족** — 속도는 −72%, 유휴 메모리는 예산 안. 대화 창이 열린 동안의 +12MB는 표면 2장(앞 장 + 그리는 장)의 값이며 창을 닫으면 돌아온다.

## 5. 구조 발견(에이전트 조사 3건 요약 · 파일:행은 조사 시점)

1. **전체 창 재래스터 + 전체 present**: `redraw()`가 매 프레임 `fill` → 역할 paint → `present()`. softbuffer CG 백엔드는 `buffer_mut`마다 `vec![0; w*h]` 새 할당, `present_with_damage`는 damage 무시. **더티 영역 present는 4개 저장소 어디에도 없다**(sql 39 §3-4 T-90f 후보).
2. **대화 1패스 레이아웃이 기록 전체**: `chat_view.rs` 1패스가 전 줄 `wrap_text`(글자당 `to_string()`+측정) — 이번에 캐시. 2패스는 보이는 것만 그린다(컬링 있음).
3. **글리프 래스터 캐시는 있다**(nexa-gfx 8192 · 넘치면 전체 비움 · LRU 아님) · 전진 폭 캐시는 GDI/CoreText 경로만(ab_glyph 경로 없음 — beep은 ab_glyph).
4. **winit mac은 런루프 1회당 창마다 RedrawRequested 1회**로 코얼레싱 — 프레임이 비싸면 입력 큐가 밀려 손가락을 늦게 따라간다(기준선 휠 20회 → 10프레임 — 단 이 프레임 수에는 `NEXA_SCRIPT` 틱 묶음이 섞여 있다 · §3 주석).
5. **우클릭**: 메뉴 생성 자체는 가볍다(5행 · 그림자 없음) · NSPasteboard 읽기 0.06ms · 느림의 정체 = 그 뒤 전체 프레임 1회.
6. 유휴: 캐럿 틱이 포커스 창을 무조건 재도색(RL-9) · `ControlFlow` 200ms 바닥(RL-10 · 미착수).

## 6. 사용자 6축 대조 — 지금 상태와 다음

| 축 | 지금(실측·근거) | 다음 |
| --- | --- | --- |
| ① 체감·UX | 스크롤/호버/메뉴 프레임 29 → ≈8ms(mac) · 휠 노치 DPI 보정 | Windows/Linux 같은 시나리오 실측(Win softbuffer = GDI 경로 · sql 65 실측 1.3ms라 present는 문제 아님 · paint 축만) |
| ② 첫 실행·전환 속도 | 계측 없음(05 B-11 첫 화면 ≤500ms) | `[startup]` 누적 트레이스(sql `NSQL_TRACE_FRAMES` 방식 · 49 INS-3) · 창 열기 = Resized→첫 paint ms |
| ③ 기능 실행 속도 | 전송·격리·해시는 워커(08-13·08-16) · UI 정지 0 설계 | 수신 이벤트 루프 1회 시간 예산(dir3 §75 · 6ms) 계측 |
| ④ UI 갱신·스크롤 | §3 | 더티 영역 present(persistent backbuffer + clip) — 4저장소 공통 과제 · 캐럿만 부분 재도색 |
| ⑤ 메모리·회수 | 유휴 20MB(softbuffer) · IOSurface는 풀 해제로 예산 안(§4) · 09-07 MEM-1~5 | sql 71 C-2 **회수 4점 시험**(대화 창 여닫기·이미지 보기·격리함·1GiB 전송) · 누수 기울기 E |
| ⑥ 파일 송수신·저장/로딩 | 08-16 M4-11 실측 속도 표시 · 32KiB 청크 오버헤드 0.25%(08-21) · 기록 적재 = 열 때 전체(≤2000줄) | 기록 200줄 + 스크롤 시 로드(MEM-⑤) · 저장 세그먼트 적재 ms 계측 |

## 7. 성능 척도 — dir3·sql 대조표(에이전트 조사 · beep에 쓸 16항목)

| # | 항목 | 측정법 | 목표치 | 출처 | beep 상태 |
| --- | --- | --- | --- | --- | --- |
| 1 | 프레임 시간(paint/present 분리) | `NEXA_PAINT_TRACE` | ≤8ms 평상 · ≤16ms 한도 | sql 71 §4 · 39 S-8 · 05 B-12 | ✅ seam · mac 실측 §3 |
| 2 | 대화 스크롤 프레임(2000줄) | 1 + `NEXA_SCRIPT wheel=` | ≤16ms · 줄 수 무관 | 05 B-12 · sql 37 P-1/2 | ✅ 캐시(30줄 실측 · 2000줄 = 다음) |
| 3 | 피어 500 목록 스크롤 | INS-4 합성 데모 + 1 | 60fps | 49 C-2 | ☐ |
| 4 | 입력→present 지연 | tmark(메모리 적재 후 present 뒤 1줄) | ≤16ms | sql 55 §6 · 49 C-1 | ☐(1로 근사) |
| 5 | mac present 비용 | 1의 present · `NEXA_MAC_PRESENT` A/B | ≈1~3ms(IOSurface) | sql 62·65 | ✅ 0.9ms |
| 6 | hover 재그리기 | 같은 행 안 이동 프레임 수 | 0 | sql 71 §8 · 05 FR-U-13 | △(메뉴 = 행 변경 때만 · 목록 = 다음) |
| 7 | 유휴 프레임·CPU | 1의 카운트 + 49 D-6 | 프레임 0(캐럿 제외) · CPU ≤0.2% | 05 B-7 · 49 C-4 | ✅ RL-9 처방(§4) · RL-10 ☐ |
| 8 | 콜드·웜 기동 | `[startup]` 누적 · 5회 중앙값 | ≤500ms(목표 200) | 05 B-11 · sql 71 B · dir3 perf-baseline | ☐ |
| 9 | 유휴·활성 메모리 | 49 §5 OS 지표 | 30 / 80MB | 05 B-1/2 | ✅ 유휴 25MB · 대화 창 41MB(mac IOSurface · §4) |
| 10 | 회수 4점 시험 | 기준→올림→놓음→유휴 | ④−① ≤2MB | sql 71 C-2 · 49 D-5 | ☐ |
| 11 | 누수 주기 기울기 | 뒤 절반 MB/주기 · 핸들·스레드 | →0 | sql 71 E · 39 §6-2 · 05 B-6 | ☐(24h ΔRSS만) |
| 12 | 캐시 상한 원장 | 글리프 8192 · 아바타 · qthumbs · 썸네일 · 기록 · **줄바꿈 캐시(줄 수 = 기록 상한 2000)** | 전 캐시 상한+수치 | sql 39 S-3 · beep 09-07 | △(MEM-1·2 미해소) |
| 13 | 오프스크린 벤치(chat paint 2000줄) | `examples/bench_chat`(sql bench_editor 방식) | 회귀 +20% 이내 | sql 71 F | ☐ |
| 14 | UI 스레드 처리 예산 | 수신·전송 루프 1회 ms | ≤6ms 후 backlog | dir3 §75 | ☐ |
| 15 | 회귀 판정 규칙 | 같은 시각 A/B | 상주 +5%/+1MB · CPU 2배 · 기동 +20% · 벤치 +20% | sql 71 §4 | ☐(문서화만) |
| 16 | 산출물·의존 | CI budget · check-imports | 10MB · 인박스만 | 05 B-3/5 | ✅ 3.5MB |

## 8. 결정·잔여

- **D-34(권고 = 채택)**: macOS present 기본 = **IOSurface**(유휴 풀 해제 포함). 근거 = present −94% · 유휴 footprint 예산 안(§4). 옵트아웃 = `NEXA_MAC_PRESENT=softbuffer`(설정 항목은 실기 뒤 — nexa-sql은 `gfx.mac_present` 설정 · beep은 창이 작아 메모리 부담이 작다).
- 잔여(TODO 등재 = [PERF-1~7](TODO.md)): Windows·Linux 같은 시나리오 실측(PERF-1) → 더티 영역 present(PERF-2 · 공통 과제 · nexa-ui T-90f) → RL-10 유휴 바닥 → 척도 ☐ 항목 계측 신설(PERF-4·5·6) → 2000줄 기록 실측(PERF-3) → `gfx.mac_present` 설정(PERF-7).
