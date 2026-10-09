# 50 · 설정 체계 개편 + nexa-ui/nexa-license 도입 — 적용 범위 · 개발 순서 · 진행 계획

> **상태**: ✅ **P1~P4 완료 · v0.3.0 공개(10-09)** — 잔여 = P5 캡처(사용자 실기 뒤) · 실기 T-1~10 · 대화상자 ①~⑧. (종전: 📐 계획 확정 10-09 → 🚧 P1 착수 → 같은 날 P1-c·P2·P3·P4·릴리스까지 완주 — 협업 세션 분담.)
> **요청 원문**(10-09): 설정 화면·구성·체계 전반 수정(nexa-sql 참고 — 그룹·고급 설정·검색·화면 구성·UI) · 언어·테마 **시스템 설정** 추가 · 파일 저장/읽기 = **새 다이얼로그**(nexa-dlg) 반영 · **nexa-ui·nexa-license로 개발**(라이선스 제품명 `nexa-beep`) · 화면 컨트롤도 nexa-ui로 교체 · 협업 세션과 함께 · 끝나면 테스트 필요 항목 보고.
> **선행 조사**(10-09 · 4축 = nexa-ui API · nexa-license 통합 · nexa-sql 설정 화면(협업 세션) · beep 현행) → [journal/2026-10-09](journal/2026-10-09.md) §2.
> **관련**: [14 §10](14-control-ux-architecture.md)(DR-24 설정 화면) · [28 ADR-0011](28-adr-0011-settings-persistence.md)(영속) · [35 ADR-0014](35-adr-0014-native-file-dialog.md)(⚠ 본 계획으로 **정정**) · nexa-ui `docs/20`(FilePicker · D-9) · nexa-sql `docs/24·78·94·23·25`.

## 0. 한 줄 결론

**beep의 자체 UI 크레이트 3종(nbeep-gfx·nbeep-ctl·nexa-conf 사본)은 nexa-ui의 직계 조상**이라 교체는 "되돌아가기"에 가깝다(접점 = `nbeep-ui/src/lib.rs:144` 재노출 한 줄 + 시그니처 차이 6건). 그 위에 **nexa-sql의 설정 화면 구조**(그룹 트리 · 고급 토글 · 카드 · 초기화 · 창 기억)를 beep의 Entry 레지스트리(90키)에 얹고, **파일 다이얼로그는 nexa-dlg FilePicker**(3-OS 자체 · ADR-0014 정정)로, **라이선스는 얇은 어댑터 크레이트**(nexa-dir3 선례 · Feature 0개 시작 · 게이트 기본 끔)로 붙인다.

## 1. 조사 결과 요약 (근거 = journal 10-09 §2 · 경로:행 생략)

| 축 | 확인한 사실 | 계획에 미치는 영향 |
|---|---|---|
| **nexa-ui** | 계보 `nexa-dir2 → nbeep-{gfx,ctl} → nclip → nexa-ui` · `Widget`/`Control`/`ControlBase`/`DrawCtx`/`CtlMsg` 시그니처 동일 · nexa-ctl = 컨트롤 30종 45k LOC(beep 17종 11k의 상위집합) · 깨지는 차이 6건(`RasterCtx::new(surface,font,scale)` · `set_secret→set_masked`+`take_secret_text` · `TextBox::text()→Cow` · `TreeModel::flatten()→Rc<Vec>` · `Button::set_tone` 반환 없음 · `Button::press/is_focused` 없음) · hangul·typeahead가 nexa-ctl 모듈 · **nexa-conf는 beep 사본이 최신**(`tighten` 0600 조이기가 ui에 없음) · **시스템 테마 추종·locale 감지 없음**(nexa-sys는 배터리·원격·reduce_motion·CPU·netwatch·입력 소스) · nexa-dlg = `FilePicker{Open,Save,Folder}` 하나(MessageBox·Prompt 미구현 · 네이티브 0) · nexa-font = `ui_font/mono_font → Loaded{font,chain}` | P1 기계적 교체 가능 · `tighten`은 ui에 먼저 올린다 · 테마 판정은 beep `nbeep-plat::theme` 유지(포털 실시간 추종 = sql보다 앞섬) · locale 감지는 **nexa-sys에 신설**(sql `syslang.rs` 이식 = 계열 공용) |
| **nexa-license** | 라이브러리는 앱을 모른다 — 제품 열거형·제품별 키 없음 · `Product{id,build_date,version}` 상수 1개 · 루트 키 `root-v1` 공통 · `verify_license → Verdict{Licensed,Outdated,Expired,Invalid}` · `License.allows(feature)`(`*` = 전부) · `Store{locate,read,install_text,remove}` · 요청 코드 `NEXAREQ1.<기기>.<메타 app=제품/버전>` · 발급기 `issue --product` 또는 요청 코드 메타로 제품 추론 · 프리셋 trial/pro/org 전부 `features=*` · ID 접두 자동 `NBL` · 선례 = nexa-sql `nsql-license`(824줄 · Feature 21 · 별도 라이선스 창 · CLI) · nexa-dir3 `ndir-license`(Feature 0) · **무료(비상업) = 파일 없이 Free · 게이트 기본 끔(D-145)** · PolyForm NC가 상업=유료를 정하고 앱은 감지하지 않는다(전화홈 0) | P4 = `nbeep-license` 어댑터 크레이트 + 도움말▸라이선스 창 + CLI · 발급기 `presets.rs` 안내 분기 1곳(nexa-license 저장소) · CI 체크아웃 추가 |
| **nexa-sql 설정 화면** | `REGISTRY` 단일 원천(Entry 6필드) · `CATEGORY_TREE` 그룹→카테고리 · 보조 표 `HIDDEN/ADVANCED/DEPENDS/INFO_KEYS/OS_DEFAULTS` · 고급 토글 `ui.prefs_advanced`(끄면 "N개 숨김" 배너 · 켜면 키 이름 강조) · 좌 트리 230+스플리터+우 카드(라벨/키+복사/설명/컨트롤/Default/[초기화]) · 창 920×640 기억 · 하단 [고급][settings.json][닫기] · 검색 = 부분 문자열 1개 + **자모 분해** + 이력 · 즉시 적용·즉시 저장 · 검증 실패 = 카드 오류만 · `ui.lang` en/ko(기본 = OS 언어 `default_of`) · `ui.theme` system/light/dark | P2의 본 |
| **beep 현행** | `nbeep-ui/settings.rs` 3821줄 · Entry 90(+숨김 18) · 카테고리 12·하위 6 · Kind 9종 · 검색 = 토큰 AND × 4개 국어 · 고급 토글 없음 · 즉시 적용(DR-24 · 적용/취소는 프로필 M3-18) · `apply_settings`(app.rs ≈470줄) 키별 반영 · 4개 국어 `Msg` 791 · `ui.language` 기본 en(OS 미추종) · `ui.theme` system 기본 + 포털 실시간 · 자체 피커 = app.rs `Role::Picker`(용도 8 · TreeView 재사용) · `FileDialogPort` 미구현(ADR-0014 "후속 슬라이스") · 파일 전송 진입 = DnD만 | — |

## 2. 결정 (D-33 묶음 — 권고안으로 진행 · 사용자 이의 시 되돌린다)

| # | 쟁점 | 결정 | 근거 |
|---|---|---|---|
| D-33-1 | 검색 | **beep 토큰 AND 유지 + sql 자모 분해 추가**(조합 중 글자도 매치) · 대상 = key+그룹+cat+하위+label+desc **전 언어**(⚠ 구현 중 "현재+영어"로 좁혔다가 영어 UI의 한글 검색이 깨져 **전 언어로 복원** — 시험이 잡음) · 이력 = `prefs.search`(↑/↓) | DR-24 명문(AND) · sql의 장점만 흡수 |
| D-33-2 | 적용 방식 | **즉시 적용 유지**(DR-24 §10 4) + **카드별 [초기화]**(기본값과 다를 때만) + 검증 실패 = 직전값 원복(14 §14) | sql과 동일 · M3-18 보류 의미론은 프로필에만 |
| D-33-3 | 파일 다이얼로그 | **ADR-0014(D-30) 정정 → nexa-dlg `FilePicker` 3-OS 자체**(nexa-ui D-9 수용) · 호스팅 = sql `file_win.rs` 패턴(별도 winit+softbuffer 모달 창 · 메인 소유) · 용도 8 전부 이관 · `PickerLabels` 4개 국어 주입 | 사용자 지시 "새 다이얼로그 반영" · DR-6(전 렌더링 자체) 원복 · Linux 자체 피커는 이미 전제 |
| D-33-4 | 언어 시스템 설정 | `ui.language` = **`system`(기본)/en/ko/zh/ja** — `system` = 부팅마다 OS 언어 추종(`nexa-sys::locale` 신설 · 미지원 언어 = en) · 변경 즉시 반영(재시작 없음) | `ui.theme`과 같은 어휘 · sql은 "system 값 없이 기본값만 OS"라 **명시 추종이 더 투명** |
| D-33-5 | 테마 시스템 설정 | `ui.theme` 현행 유지(system 기본 · 포털 실시간) · 설정 화면에서 **"일반 › 시스템" 그룹 카드**로 언어·테마·크기 배율을 한자리에 | 이미 구현 · 자리만 옮김 |
| D-33-6 | 라이선스 범위 | `Feature` **0개로 시작**(nexa-dir3 선례) · 등급 표시 Free/Trial/Pro/Org · 게이트 스위치 `license.gates` 기본 **off**(D-145 계승) · 상업 전용 기능 잠금은 **후속 결정**(후보 = 릴레이 서버 Managed · 그룹 N명 초과 · 컨텐츠 모드) · 무료(비상업) = 파일 없이 Free | PolyForm NC = 상업 사용 유료를 **라이선스가** 정한다 · 앱은 전화홈 0 |
| D-33-7 | 라이선스 자리 | 파일 = `data_dir()/license/nexa-beep.license`(포터블 = 실행 파일 옆 · DR-4) + 기기 공용(Win `%ProgramData%\nexa\nexa-beep` · mac `/Library/Application Support/nexa/nexa-beep` · Linux `/etc/nexa/nexa-beep`) · UI = **도움말 ▸ 라이선스…** 창(상태표 · 요청 코드 복사 · 파일 열기(nexa-dlg) · 제거) + 설정 "정보" 그룹 읽기 전용 카드 · CLI `--license status\|request\|install\|remove` · 빌드일 `NBEEP_BUILD_DATE` | sql·dir3 동일 패턴 |
| D-33-8 | nexa-conf | beep 사본의 `tighten`을 **nexa-ui에 올린 뒤** beep `crates/nexa-conf` 삭제 → path 의존 | "사본 본문 동일 유지" 규약 · 최신본 = beep |
| D-33-9 | 키 재편 | sql `docs/94` 규칙(2레벨 `<접두>.<이름>` · 접두 = 카테고리 1:1) 적용 · 바뀌는 키는 `RENAMED` 표로 읽을 때 이관 · **`ime.*` 11키 = 고급(ADVANCED)** · `theme.*` 8키 = 모양 › 색 | 사용자 눈에 보이는 키는 줄이고 영속 호환은 지킨다 |

## 3. 적용 범위

### 들어가는 것

| 영역 | 내용 |
|---|---|
| **의존 전환** | `nbeep-gfx`→`nexa-gfx` · `nbeep-ctl`→`nexa-ctl` · `crates/nexa-conf`→`nexa-ui/nexa-conf` · `nbeep-plat/font.rs`→`nexa-font` · `nbeep-ui/{hangul,typeahead}`→`nexa-ctl::{hangul,typeahead}` · 신규 `nexa-dlg`·`nexa-sys`(locale) · `nexa-license` — 전부 **path 의존**(`../nexa-ui` · `../nexa-license`) · 삭제 크레이트 3(nbeep-gfx·nbeep-ctl·nexa-conf) → 워크스페이스 14→11+1(nbeep-license) |
| **설정 모델** | Entry에 그룹 층(`CATEGORY_TREE`) · `ADVANCED`/`HIDDEN`/`DEPENDS`/`INFO_KEYS` 보조 표 · `display_order` · `default_of`(OS별 기본) · `RENAMED` 이관 · 카드별 초기화 |
| **설정 화면** | 좌 트리(그룹→카테고리 · 항상 펼침 · 검색 중 "(N)") + 스플리터 + 우 카드(라벨 / 키 이름+복사 / 설명 / 컨트롤 / Default / [초기화]) · 하단 [고급 스위치][settings.cfg 열기][닫기] · "고급 N개 숨김" 배너 · DEPENDS 잠금+흐림 · 창 크기·위치 기억(`ui.prefs_*`) · 검색 = AND+자모+이력 · Kind→컨트롤: Radio→`Combo` · Toggle→`Switch` · PositionGrid→`PositionDropdown`(nexa-ctl 신규 — 64px) · Color→`ColorPicker` · Text(secret)→`set_masked`+`take_secret_text` · Action→`Button` |
| **시스템 설정** | "일반 › 시스템" = `ui.language`(system 추가) · `ui.theme` · `ui.control_size` · `nexa-sys::locale`(신설 · Win `GetUserDefaultUILanguage` · mac `CFLocaleCopyPreferredLanguages` · Linux `LANGUAGE→LC_ALL→LC_MESSAGES→LANG`) |
| **파일 다이얼로그** | app.rs 자체 피커(`Role::Picker`·`PickerPurpose` 8·`open_picker`·이벤트 ≈400줄) **삭제** → `nbeep-ui/picker_win.rs`(nexa-dlg 호스팅 · sql `file_win.rs` 이식) · 용도 8 = BackupDir·RestoreKey·SettingsBackupDir·SettingsRestoreFile·ProfileImage·HistoryBackupDir·HistoryRestoreDir·GallerySample → `PickerMode{Folder,Open,Save}` 매핑 · 필터(키 파일·이미지·세그먼트) · 라이선스 파일 열기(P4) · **(선택 P3-b)** 대화창 "파일 보내기…" 메뉴 = Open 다중 선택 → 기존 Offer 경로(DnD와 동일 합류점) |
| **라이선스** | `crates/nbeep-license`(Product · build.rs · Feature(0) · Tier · `Licensing{open_default,check,refresh,install,remove,request_code}` · 폴더 2축) · `nbeep-ui/license_win.rs` · 도움말 메뉴 · 설정 "정보" 카드(INFO_KEYS) · CLI · `license.gates`(HIDDEN) · nexa-license 저장소 = `presets.rs` nexa-beep 안내 분기 + CONSUMER-CHANGES 행 |
| **CI·배포** | 6 워크플로 = 형제 체크아웃(`SosomLab/nexa-ui`·`SosomLab/nexa-license`) + `defaults.run.working-directory: nexa-beep` + rust-cache `workspaces` · `packaging/lib.sh`는 `BASH_SOURCE` 기준이라 무변경 · 예산 게이트(≤10MB · RSS ≤30MB) **재실측** |
| **문서** | 14 §10 개정 · 35 ADR-0014 정정 표기 · 10 DR-30(가칭 · 본 묶음) · 18 빌드 절차(형제 clone 필수) · 45 매뉴얼 설정 캡처 · README/위키 Install("형제 저장소" 안내) · TODO/STATUS/MILESTONES |

### 들어가지 않는 것(명시 제외)

- 채팅·목록·프로필 **화면 자체의 재설계** — 컨트롤 교체는 기능 불변(시각 동일) · 화면 구성 변경은 설정 화면만.
- nexa-ui에 **없는** 다이얼로그(MessageBox·Prompt) 신설 — beep `alert.rs`·`prompt.rs` 유지.
- 상업 전용 기능 **게이팅 실물**(Feature ≥1) — D-33-6 후속.
- nexa-grid·nexa-explorer 도입(사용처 없음).
- Linux arm64 등 배포 확장(M5-4c 잔여).

## 4. 개발 순서 · 분담 (협업 = 이 세션 `nexa-beep-11` · 좌측 `nexa-beep-78`)

> 원칙: **한 작업 트리에서 app.rs를 동시에 만지지 않는다.** 저장소가 다른 작업(nexa-ui · nexa-license)은 병렬, beep 안은 단계별 직렬 + 파일 경계로 분할. 각 단계 = 브랜치 1개 → ff 병합 → push(nexa-ui·nexa-license가 beep보다 **먼저** push — CI가 형제 main을 체크아웃한다).

| 단계 | 내용 | 담당 | 선행 | 게이트 |
|---|---|---|---|---|
| **P0** 결정·계획 | 본 문서 · docs/README 50 · journal | 11 | — | ✅ 10-09 |
| **P1-a** nexa-ui 선행 | ① `nexa-conf::tighten` 이식(+테스트 `open_tightens_legacy_mode`) ② `nexa-sys::locale`(sql `syslang.rs` 이식 · `Lang` 코드 반환 `Option<&str>` "ko"/"en"/"zh"/"ja" · OnceLock 아님 — 부팅마다) ③ CONSUMER-CHANGES 행 · nexa-ui check-3os | **78**(저장소 분리 = 병렬) | — | nexa-ui 3-OS clippy · 테스트 |
| **P1-b** nexa-license 선행 | `nexa-license-tool presets.rs` nexa-beep 안내 분기(`apply_steps`) · CONSUMER-CHANGES | **78** | — | tool 테스트 |
| **P1-c** beep 의존 전환(기능 불변) | Cargo path 의존 6종 · `nbeep-ui/lib.rs:144` → `nexa_ctl` · 시그니처 6건 · gfx/font 교체(`nbeep-plat/font.rs` → nexa-font 어댑터) · hangul/typeahead 전환(ime_gate) · 크레이트 3 삭제 · CI 형제 체크아웃 6 워크플로 · 예산 재실측 | **11** | P1-a ① | 4타깃 check/clippy · **807+ 테스트 green** · ime 재생 17종 · 크기 ≤10MB · 3신원 실기 발견 |
| **P2** 설정 체계 개편 | 2-a 모델(그룹·보조 표·display_order·default_of·RENAMED) · 2-b 화면(트리/카드/하단/배너/창 기억/검색 자모·이력) · 2-c 시스템 그룹(`ui.language` system · nexa-sys locale 배선 · 테마 카드) · 2-d `apply_settings` 정리 | **11**(`settings.rs`·app.rs `apply_settings`·`open_settings` 구간) | P1-c · P1-a ② | settings 테스트 24+ · settings_reset · 실기 Win/mac/Linux 캡처 |
| **P3** 파일 다이얼로그 | 3-a `nbeep-ui/picker_win.rs`(nexa-dlg 호스팅) · 용도 8 매핑 · app.rs 자체 피커 삭제 · `PickerLabels` i18n · 35 정정 · (3-b 선택) "파일 보내기…" | **78**(새 파일 + app.rs **피커 구간만** — 편집 전 통지) | P1-c | 용도 8 실기 3-OS · 키 파일 복원 왕복 |
| **P4** 라이선스 | `crates/nbeep-license` · `license_win.rs` · 도움말 메뉴 · 설정 "정보" 카드(P2 모델의 INFO_KEYS) · CLI · 요청 코드 → 발급 → 설치 왕복 실측(발급기 로컬 키) | **78**(새 크레이트·새 파일) → 메뉴/CLI 배선은 11과 조율 | P1-b · P2-a | 검증 4판정 테스트 · 왕복 실기 |
| **P5** 마감 | 문서 일습 · 위키 · 매뉴얼 캡처 · TODO/STATUS/MILESTONES · 릴리스 후보(v0.3.0 — 의존 구조 변경 = 마이너 상승) | 11+78 | 전부 | 전 게이트 + 실기 표 §6 |

예상 규모(난이도): P1-c 중(1~2일 · 기계적이나 전수) · P2 대(3일) · P3 중(1~2일) · P4 중(1~2일) · P5 소. **총 1.5~2주**(두 세션 병렬 시 1주 안팎).

## 5. 위험과 대비

| 위험 | 대비 |
|---|---|
| nexa-ctl TextBox(7,743줄)로 바뀌며 **IME 경합 회귀**(H-1~27) | ime_gate 트레이스 재생 17종이 게이트 · 3-OS 실기(`NEXA_IME_TRACE`) · nexa-ctl에는 beep 입력 수정이 이미 이식돼 있음(hangul diff 17줄) |
| 예산 게이트 초과(nexa-ctl 45k LOC · nexa-gfx 5.8k) | 쓰지 않는 모듈은 링크 시 제거(LTO fat) — P1-c 끝에 산출물 크기·RSS 실측 · 초과 시 feature 분리 요청(nexa-ui) |
| 형제 저장소 path 의존 = **clone 절차 변경**(세 저장소 나란히) | 18 §1·README에 명시 · CI 체크아웃 · `install-local`·`relaunch` 스크립트 무변경(ROOT 기준) |
| app.rs 동시 편집 충돌 | 파일 경계 분담 + 편집 전 통지 + 작은 커밋·잦은 rebase(16 §3) |
| ADR-0014 정정의 기록 규칙 | 35 상단 상태 → "⚠ 정정 10-09 (D-33-3 · 본 문서)" · DR 표 갱신 · 새 ADR 번호는 쓰지 않는다(정정은 journal+본 문서) |
| nexa-license 발급기 루트 키 = 발급 PC에만 | 개발 왕복은 **로컬 임시 키**(`keygen` → 앱 `ROOT_KEYS` 교체 없이 검증하려면 `root-v1` 필요) → 실키 발급은 사용자 PC · 테스트는 라이브러리 `issuer` feature로 임시 키 쌍을 만들어 `verify_license(roots=임시)` 단위 테스트 |

## 6. 테스트 필요 항목(사람 실기 · 완료 시 §7 표로 전환)

| # | 항목 | OS | 비고 |
|---|---|---|---|
| T-1 | 설정 창: 그룹 트리 · 카드 · [초기화] · 고급 스위치(배너/강조) · 창 크기·위치 기억 | 3-OS | 캡처 → 45 매뉴얼 |
| T-2 | 검색: 한글 조합 중 매치 · 토큰 AND · 이력 ↑/↓ | 3-OS | IME |
| T-3 | `ui.language=system`: OS 언어 변경 → 재부팅 시 추종 · 명시 선택 우선 | 3-OS | Linux = `LANG=ja_JP.UTF-8 nexa-beep` |
| T-4 | `ui.theme=system` 실시간(기존) + 창 장식 동기 | 3-OS | 회귀 |
| T-5 | 파일 다이얼로그 용도 8(백업/복원·설정 백업/복원·프로필 사진·대화함 백업/복원·갤러리) + 라이선스 파일 | 3-OS | 키 파일 복원 왕복 = 신원 유지 확인 |
| T-6 | IME 3대 이슈 회귀(H-25~27) · 한/영 직접 토글(Win) | Win·mac | nexa-ctl TextBox |
| T-7 | 라이선스: 요청 코드 복사 → 발급 → 설치 → 상태 Licensed · 제거 → Free · 다른 제품 파일 = Invalid | 1-OS | 발급기 = 사용자 PC |
| T-8 | 예산: 산출물 ≤10MB · 유휴 RSS ≤30MB · 콜드 스타트 체감 | 3-OS | 49 점검 체계 C축 |
| T-9 | 3신원 상호 발견·메시지·파일(기능 불변 확인) | 1-OS | `relaunch` |
| T-10 | 설치본 갱신(`install-local`) · 포터블 `data/` 옆 `license/` | Linux·Win | DR-4 |

## 7. 진행 기록

| 일자 | 단계 | 내용 |
|---|---|---|
| 10-09 | P0 | 조사 4축 · 본 문서 · 분담 합의 |
| 10-09 | P1-a·b ✅ | 협업: nexa-ui `tighten`·`locale::ui_language`·toolbar dim/ring(4커밋 main) · nexa-license 발급기 nexa-beep 분기(1커밋 main) — 미push |
| 10-09 | P1-c ✅ | beep 의존 전환(`refactor(ui)`) — 665 green · 4타깃 0 · 릴리스 6.03MB(+4.2%) · CI 형제 체크아웃 · 발견 = nexa-ctl 전 컨트롤 MouseUp 확정 |
| 10-09 | P2 ✅ | 설정 체계 개편(`feat(settings)`) — 그룹 트리·고급 스위치·카드 초기화/키 복사·DEPENDS·자모 검색+이력·`ui.language=system`·창 기하 · 675 green · 4타깃 0 · 실기 T-1~3 잔여 |
| 10-09 | P4 ①② ✅(협업) | worktree `feat/license-p4` — `crates/nbeep-license` · `license_win.rs` · i18n 37키 · main cherry-pick `6f91364`·`61daef9` |
| 10-09 | P4 ③ ✅ | `8a67200` 배선(메뉴·Role::License·행동 4종·피커 LicenseFile·CLI) · 695 green · 4타깃 0 · CLI 스모크 ✓ · 실기 T-7 잔여 |
| 10-09 | T-9 ✓ | 3신원 재기동 — 지문 불변 · 상호 발견 3/3(이관 뒤 기능 불변) |
| 10-09 | P3 ✅(협업) | `77d863c`·`62974c6` cherry-pick — nexa-dlg 호스팅 · 용도 9 · docs/35 정정 · 동작 변화 4 · Ctrl/⌘+C 복사 배선(nexa-ui 188차) |
| 10-09 | P4 ⑤ ✅ | 설정 › 고급 › 라이선스 정보 카드 4행(Kind::Info) |
| 10-09 | 실기 피드백 | 타입어헤드(표시 이름·↑ 감김·sql 설정) · 설정 preedit · **카드 레이아웃(사용자 확정 sql 모양)** · 위치 드롭다운 · 트레이 배지 M3-2e |
| 10-09 | P5 🚧 | docs/45 초안(협업 `3ba3bf5`) · 14 §10-3 · 위키 User-Guide 로컬 · 캡처 = 사용자 실기 뒤 · 설정 진입점 3가지 · 트레이 설정… · "시스템 (값)" |
| 10-09 | ★**v0.3.1 ✅ 공개** | 패치 — Linux 실기 수정 7건(트레이 설정 창·분리 대화 창 활성화 토큰·한글 타입어헤드·힌트 wrap·테마 라벨) · 내 기기 목록 창 · 툴팁 언어(nexa-ui 190차) · apt 자동 갱신 첫 성공 · choco 제외(0.3.0 검수 중) · CI rustdoc 2회 → `tools/gate.sh` |
| 10-09 | ★**v0.3.0 ✅ 공개** | 태그 `v0.3.0` · release run 37924847844 전 job success · 자산 15(rpm 첫 등장) · brew ✓ · choco 0.3.0 ×2 자동 제출(모더레이션 대기) · winget 제외 · 1차 CI rustdoc 실패 → 18 §1 게이트 행 · 위키 Release-Notes/Home/Install/User-Guide 0.3.0 |
