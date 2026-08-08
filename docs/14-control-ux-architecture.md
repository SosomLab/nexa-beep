# 14 · 컨트롤 아키텍처 · UX 설계 표준

> **사용자 확정(2026-08-08)**: 컨트롤은 **한 벌 + OS 동작 어댑터** · 이벤트는 **3단계(캡처→타겟→버블)** · **macOS 위주 UX 경험 유지** · *"모든 모양이 예쁘면서도 직관적이고 빠르게"* — 호버링·클릭·풀다운·선택 시 동작과 **직관적 식별**을 충분히 고려할 것.
> 상위: [ADR-0001 §3](07-adr-0001-stack.md)(자체 CPU 래스터라이저) · [13 코드 설계 표준](13-code-design-standards.md) · [12 차용 자산](12-asset-reuse.md).
> **이 문서가 UI 구현의 SSOT다.** 시각·상호작용을 바꾼 커밋에서 함께 고친다.

---

## 0. 결정 요약

| # | 결정 |
|---|---|
| **① 구현 벌 수** | **컨트롤은 플랫폼 중립 1벌.** OS별로 다른 것은 **동작 관례만** 어댑터로 독자 구현 |
| **② 시각 언어** | **macOS 기준으로 4타깃 통일.** Windows·Linux에서도 같은 화면을 본다 |
| **③ 동작 관례** | **각 OS 네이티브를 따른다** — 수정 키·스크롤·더블클릭·컨텍스트 메뉴·창 동작 |
| **④ 이벤트** | **3단계 전파** — 캡처 → 타겟 → 버블. `stop_propagation` + `prevent_default` |
| **⑤ 공통 전파** | 트레이트 **기본 메서드 + 컴포지션**. 깊은 상속 계층 금지([13 §8](13-code-design-standards.md)) |
| **⑥ 성능** | 입력→반영 **≤16ms** · 상태 변화는 **해당 컨트롤만** 다시 그린다 |

---

## 1. 핵심 원리 — 시각과 동작을 분리한다

"맥 위주 UX"와 "4타깃 동일 UI"와 "각 OS 네이티브"는 그냥 두면 서로 싸운다. **축을 둘로 가르면 전부 성립한다.**

| 축 | 기준 | 이유 |
|---|---|---|
| **시각 언어**(Visual) | **macOS 통일** — 색·형태·간격·타이포·모션 | S-7(스크린샷으로 OS 구분 불가)을 지킨다. 픽셀은 우리가 그린다 |
| **동작 관례**(Behavioral) | **각 OS 네이티브** — 키 바인딩·스크롤·더블클릭·메뉴 트리거 | 여기까지 통일하면 **Windows에서 쓸 수 없는 프로그램**이 된다. 맥에서 `Cmd+C`, 윈도우에서 `Ctrl+C`는 협상 대상이 아니다 |

> **판단 기준 한 줄** — *"보이는 것은 맥, 손에 익는 것은 그 OS."*

`nexa-dir2`의 `ctl`도 이미 이 방향이었다(`NxComboBox` = **macOS 팝업 버튼** 모델). 시각 규약 계승([12 §3-B](12-asset-reuse.md))이 그대로 맞물린다.

---

## 2. 컨트롤 계층 — 공통을 전체에 전파하는 구조

### 2-1. 상속 대신 — 트레이트 기본 메서드 + 공통 상태 컴포지션

Rust에는 상속이 없다. **"공통 속성·동작을 전체에 전파"** 라는 요구는 두 수단의 조합으로 구현한다.

```rust
/// 모든 컨트롤이 공통으로 갖는 상태. 각 컨트롤이 필드로 "포함"한다(컴포지션).
pub struct WidgetBase {
    pub id: WidgetId,
    pub rect: Rect,              // 부모 기준 배치
    pub visible: bool,
    pub enabled: bool,
    pub focusable: bool,
    pub state: VisualState,      // §4 — hover/active/focus/selected 비트
    pub tab_index: Option<u16>,
    pub tooltip: Option<TextKey>,
    pub a11y: A11yRole,          // 접근성 역할(v2 노출, v1부터 채운다)
}

/// 모든 컨트롤이 구현하는 계약.
/// ★ 기본 메서드가 "공통 동작"을 담아 전 컨트롤에 자동 전파된다.
pub trait Widget {
    // ── 필수: 컨트롤 고유 ─────────────────────────────
    fn base(&self) -> &WidgetBase;
    fn base_mut(&mut self) -> &mut WidgetBase;
    fn measure(&mut self, cx: &LayoutCx) -> Size;
    fn paint(&mut self, dc: &mut dyn DrawCtx, cx: &PaintCx);

    // ── 기본 제공: 덮어쓰지 않으면 공통 동작 적용 ──────
    fn children(&self) -> &[WidgetRef] { &[] }
    fn hit_test(&self, p: Point) -> bool { self.base().rect.contains(p) }
    fn on_event(&mut self, _e: &mut EventCtx) -> Handled { Handled::No }

    /// 공통 자동 높이 — ctl 규약 계승: 글꼴 + 상하 4px(컴팩트는 +2px)
    fn auto_height(&self, cx: &LayoutCx) -> i32 { cx.metrics.auto_height() }

    /// 상태 전이 시 무효화 — 전 컨트롤 공통(개별 구현 불필요)
    fn set_state(&mut self, s: VisualState, inv: &mut Invalidator) {
        if self.base().state != s {
            self.base_mut().state = s;
            inv.mark(self.base().rect);       // ★ 해당 컨트롤만 다시 그린다
        }
    }

    /// 접근성 노드 — 기본은 base에서 파생, 필요 시 덮어쓴다
    fn a11y_node(&self) -> A11yNode { A11yNode::from_base(self.base()) }
}
```

| 요구 | 구현 수단 |
|---|---|
| "공통 속성" | **`WidgetBase` 컴포지션** — 모든 컨트롤이 필드로 포함 |
| "공통 동작을 전체에 전파" | **트레이트 기본 메서드** — 새 공통 동작을 기본 구현과 함께 추가하면 **기존 컨트롤이 자동으로 얻는다**(추상 클래스 템플릿 메서드와 같은 이점, [13 §2-1](13-code-design-standards.md)) |
| "추상 레벨" | 컨트롤은 `Widget`만 알면 된다. 컨테이너도 `Widget` |
| 깊은 계층 방지 | **트레이트 상속 2단 이하.** 재사용은 **컴포지션**(예: `Pressable`·`Scrollable` 같은 동작 조각을 필드로) |

### 2-2. 속성 캐스케이드 — 위에서 아래로 흐르는 것

부모가 정하면 자식이 물려받는 값. **컨트롤마다 다시 전달하지 않는다.**

```rust
/// 페인트/레이아웃 시 트리를 따라 내려가는 문맥. 자식은 필요한 것만 덮어쓴다.
pub struct PaintCx<'a> {
    pub style: &'a Style,        // 팔레트 — ctl 규약 계승(색 하드코딩 금지)
    pub metrics: &'a Metrics,    // 폰트·자동 높이·간격 리듬
    pub scale: f32,              // DPI 배율
    pub density: Density,        // Comfortable | Compact
    pub inactive: bool,          // 창 비활성 — 맥은 선택색이 회색으로 바뀐다
    pub lang: LangId,
    pub reduce_motion: bool,     // 접근성 설정
}
```

| 캐스케이드되는 것 | 캐스케이드 안 되는 것 |
|---|---|
| 팔레트·테마 · 폰트·메트릭 · DPI 배율 · 밀도 · 언어 · 창 활성 상태 · 모션 감소 · `enabled`(부모가 끄면 자식도 꺼짐) | 크기·위치(레이아웃이 계산) · 포커스(트리 전역 단일) · 선택(컨트롤 고유) |

> **`enabled`만 부모→자식 전파**다. 그룹을 통째로 끄는 동작이 실제로 자주 필요하고, 컨트롤마다 구현하면 반드시 빠뜨린다.

### 2-3. 컨트롤 목록 (v1)

`nexa-dir2` `ctl`의 **계약**을 계승하되([12 §3-B](12-asset-reuse.md)) 메신저에 필요한 것만.

| 컨트롤 | 용도 |
|---|---|
| `Label` `Button` `IconButton` | 기본 |
| `TextField` `SearchField` | 입력 · 검색(내장 ✕) |
| `MessageComposer` | 여러 줄 입력 + IME + 붙여넣기 + 드롭 |
| `ListView` | **피어 목록 · 대화 목록** — 가상화 필수 |
| `ConversationView` | 말풍선 스트림 · 이미지 인라인 |
| `PopUpButton` | **맥 팝업 버튼**(현재 선택을 버튼에 표시, ✓ 표시) |
| `PullDownMenu` `ContextMenu` | 액션 메뉴 |
| `Checkbox` `Segmented` `Toggle` | 설정 |
| `GroupCard` | 설정·정보 묶음 |
| `Sheet` `Alert` | 모달 — **맥 시트 스타일** |
| `Toast` `Badge` | 알림 · 미읽음 · **신뢰 상태 배지** |
| `ProgressBar` | 전송 진행 |
| `Scroller` | **오버레이 스크롤바**(맥 방식) |
| `Avatar` | 피어 식별 |

---

## 3. 이벤트 전파 모델 — 3단계

### 3-1. 흐름

```
        루트
         │ ① 캡처(Capture) — 위 → 아래
         ▼      모달 차단 · 드래그 가로채기 · 전역 단축키
      컨테이너
         │
         ▼
      [ 타겟 ] ② 타겟(Target) — 컨트롤 자신이 처리
         │
         ▲ ③ 버블(Bubble) — 아래 → 위
      컨테이너      미처리분을 상위가 받는다(스크롤·기본 동작)
         ▲
        루트
```

### 3-2. 계약

```rust
pub struct EventCtx<'a> {
    pub event: &'a Event,
    pub phase: Phase,              // Capture | Target | Bubble
    pub target: WidgetId,
    pub modifiers: Modifiers,      // ★ 이미 OS 어댑터가 정규화한 값(§6)
    pub inv: &'a mut Invalidator,
    stopped: bool,
    default_prevented: bool,
}

impl EventCtx<'_> {
    /// 더 이상 전파하지 않는다(다른 컨트롤이 못 본다).
    pub fn stop_propagation(&mut self) { self.stopped = true; }
    /// 전파는 계속하되 기본 동작은 막는다.
    pub fn prevent_default(&mut self) { self.default_prevented = true; }
}
```

| 상황 | 어느 단계에서 |
|---|---|
| **모달·시트 열림** → 뒤쪽 입력 차단 | **캡처** |
| **전역 단축키**(Cmd/Ctrl+F 검색 등) | **캡처** — 텍스트 필드가 먹기 전에 |
| **드래그 중** 포인터 가로채기 | **캡처**(포인터 캡처 소유자) |
| 버튼 클릭·체크박스 토글 | **타겟** |
| 목록에서 처리 안 한 휠 → 상위 스크롤 | **버블** |
| ESC로 팝업 닫기 | **버블**(타겟이 안 먹으면 상위 팝업이) |

### 3-3. 포인터

| 규칙 | 내용 |
|---|---|
| **히트 테스트** | Z 순서 역순(위에서부터) · 부모 클립 영역 밖은 제외 · `visible && enabled`만 |
| **포인터 캡처** | press 시 대상이 캡처를 잡는다. release까지 **모든 포인터 이벤트가 그 컨트롤로** — 밖으로 끌어도 상태가 유지된다 |
| **클릭 취소** | press 후 **컨트롤 밖에서 release** = 취소(눌린 상태 해제, 클릭 미발생). 맥·윈도우 공통 관례 |
| **hover 합성** | 포인터 이동 시 `PointerEnter`/`PointerLeave`를 프레임워크가 **합성**한다. 컨트롤이 직접 추적하지 않는다 |
| **hover 억제** | 드래그 중·팝업 열림 중에는 뒤쪽 hover를 일으키지 않는다 |
| 우클릭 | **OS 어댑터가 정규화** — 맥은 `Ctrl+클릭`도 컨텍스트 메뉴([§6](#6-os-동작-어댑터--플랫폼별로-달라야-하는-것)) |

### 3-4. 키보드 · 포커스

| 규칙 | 내용 |
|---|---|
| 포커스 | 트리 전역 **단 하나**. 포커스 링은 **항상 보인다**(맥 accent 링) |
| Tab 순회 | `tab_index` 순 → 없으면 배치 순. 컨테이너는 진입 가능 |
| 키 라우팅 | **캡처(단축키) → 타겟(포커스 컨트롤) → 버블(기본 동작)** |
| 방향키 | 목록·세그먼트가 **타겟 단계에서 소비**. 소비 안 하면 버블로 포커스 이동 |
| ESC | 팝업 → 시트 → 모달 순으로 **가장 안쪽부터** 닫힌다(버블) |
| Enter | 기본 버튼 실행. **위험 동작에서는 기본 버튼이 "취소"**([ADR-0004 §7](11-adr-0004-quarantine.md)) |
| **IME** | 조합 중 키 이벤트는 **IME가 우선 소비**. `CompositionStart/Update/End`를 별도 이벤트로. 조합 중 단축키 가로채기 금지(FR-U-7) |

---

## 4. 상태 모델 — "직관적 식별"의 실체

```rust
bitflags! {
    pub struct VisualState: u8 {
        const HOVER    = 1 << 0;
        const ACTIVE   = 1 << 1;   // 눌린 중
        const FOCUS    = 1 << 2;   // 키보드 포커스
        const SELECTED = 1 << 3;
        const DISABLED = 1 << 4;
        const INVALID  = 1 << 5;   // 검증 실패
    }
}
```

### 상태별 시각 규약 (전 컨트롤 공통 — `Style`이 값을 제공)

| 상태 | 표현 | 원칙 |
|---|---|---|
| Rest | 기본 | — |
| **Hover** | 배경 미묘한 밝기 변화 | **지연 0** — 즉시 반응해야 "빠르다"고 느낀다 |
| **Active(press)** | 배경 한 단계 더 · 아주 미세한 눌림 | press **즉시**. 애니메이션 대기 금지 |
| **Focus** | **accent 링**(맥 스타일) | 마우스로 눌러도 링은 유지 |
| **Selected** | accent 채움 + 대비 전경색 | 창 **비활성 시 회색**으로(맥 관례 — `PaintCx.inactive`) |
| Disabled | 전경 흐림 · hover 없음 | 클릭 대상에서 제외 |
| Invalid | `danger` 테두리 + 아이콘 | **색만으로 구분하지 않는다** |

> ★ **색만으로 상태를 구분하지 않는다.** 선택은 색 + **체크/굵기/아이콘**, 오류는 색 + **아이콘**. 색각 이상·저대비 환경에서도 식별돼야 한다. `Selected`와 `Focus`는 **동시에 다르게** 보여야 한다(맥에서 흔히 혼동되는 지점).

---

## 5. macOS 기준 시각 언어

> 구체 수치는 **M3에서 시안 대조 후 확정**한다. 여기서는 **규약과 방향**을 고정한다.

| 요소 | 방향 |
|---|---|
| **모서리** | 라운드 기본. 컨트롤 크기별 반경 단계표(작은 컨트롤은 작게 — 맥 관례) |
| **높이 리듬** | `ctl` 계승 — 글꼴 + 상하 4px(컴팩트 +2px). *"배치만 해도 반듯"* |
| **간격** | 4px 그리드. 그룹 간 여백 > 그룹 내 여백 |
| **타이포** | 시스템 폰트([ADR-0001 §5](07-adr-0001-stack.md)) + **우리 텍스트 스택이 자간·행간·크기 단계를 통제** |
| **accent** | 우리 팔레트가 기본. **맥 시스템 강조색 추종은 선택 옵션**(켜면 그 기기만 달라진다 — S-7과의 절충을 설정에 명시) |
| **선택** | accent 채움. **비활성 창 = 회색** |
| **포커스 링** | accent 링, 컨트롤 바깥쪽 |
| **그림자** | 팝업·시트·토스트에만. **평면 UI에 그림자 남발 금지** |
| **스크롤바** | **오버레이** — 평소 숨김, 스크롤 시 등장, 호버 시 굵어짐 |
| **모션** | 짧고 감속(ease-out). 팝업 등장 ~120ms, hover 전이 ~80ms. **`reduce_motion`이면 전부 0** |
| **아이콘** | 벡터 자체 렌더. 굵기·크기 단계 통일 |
| **밀도** | Comfortable(기본) / Compact 두 단계 |

### 팝업/풀다운 규약 (사용자가 특히 지목한 항목)

| 항목 | 규약 |
|---|---|
| **PopUpButton**(선택형) | 맥 방식 — **현재 선택 값이 버튼 면에 표시**되고, 열리면 목록에서 **현재 항목에 ✓**. 팝업이 **버튼 위에 겹쳐** 뜬다(현재 항목이 버튼 자리에 오도록) |
| **PullDownMenu**(액션형) | 버튼 면은 **고정 라벨**. 팝업은 **버튼 아래**로 |
| 열기/닫기 | 열림 ~120ms 감속, 닫힘 즉시. **바깥 클릭·ESC·포커스 이탈**로 닫힘 |
| 키보드 | ↑↓ 이동 · Enter 확정 · ESC 취소 · **타입어헤드 선택** |
| 화면 경계 | 아래 공간이 부족하면 **위로 뒤집는다**. 좌우도 동일 |
| 상태 유지 | 취소하면 **원래 값 복귀**(중간 미리보기 값 채택 금지) |
| 긴 목록 | 스크롤 + 현재 항목으로 초기 스크롤 |

---

## 6. OS 동작 어댑터 — 플랫폼별로 달라야 하는 것

> **이것이 "각 OS에 독자적으로"의 실체다.** 컨트롤은 정규화된 입력만 보고, 아래 차이는 어댑터가 흡수한다.

| 항목 | macOS | Windows | Linux |
|---|---|---|---|
| **주 수정 키** | `Cmd` | `Ctrl` | `Ctrl` |
| 복사/붙여넣기/찾기 | `Cmd+C/V/F` | `Ctrl+C/V/F` | `Ctrl+C/V/F` |
| 설정 열기 | `Cmd+,` | 메뉴/`Ctrl+,` | 메뉴 |
| **컨텍스트 메뉴** | 우클릭 **+ `Ctrl`+클릭** | 우클릭 + 메뉴 키 | 우클릭 + 메뉴 키 |
| **스크롤** | 자연 스크롤 · **관성** | 휠 노치 단위 | 휠 노치 단위 |
| 더블클릭 간격 | 시스템 값 | 시스템 값 | 시스템 값 |
| **창 닫기** | **앱은 계속 실행**(메뉴 막대 상주) | 트레이로 최소화 | 트레이로 최소화 |
| 상주 UI | **메뉴 막대 아이템** | 시스템 트레이 | 트레이(DE별 차이) |
| 텍스트 편집 | `Cmd+←/→` 줄 끝, `Opt+←/→` 단어 | `Home/End`, `Ctrl+←/→` | Windows와 동일 |
| 삭제 키 | `Delete`=백스페이스, `Fn+Delete`=삭제 | `Backspace`/`Delete` | 동일 |
| **IME** | 입력 소스 | IMM/TSF | IBus/Fcitx |
| 접근성 API | NSAccessibility | UIA | AT-SPI |
| 파일 열기 | 시스템 대화상자 | 시스템 대화상자 | 포털/GTK |

**어댑터 계약**

```rust
/// 플랫폼 관례를 정규화해 컨트롤에 넘긴다. 컨트롤은 OS를 모른다.
pub trait PlatformConventions {
    fn primary_modifier(&self) -> Modifier;              // Cmd | Ctrl
    fn accelerator(&self, a: StdAccel) -> KeyChord;      // Copy/Paste/Find/…
    fn opens_context_menu(&self, e: &PointerEvent) -> bool;
    fn scroll_delta(&self, raw: RawScroll) -> Delta;     // 방향·관성 정규화
    fn double_click_interval(&self) -> Duration;
    fn close_button_behavior(&self) -> CloseBehavior;    // HideApp | ToTray
    fn text_nav(&self, key: Key, m: Modifiers) -> Option<TextMotion>;
}
```

> 단축키는 **하드코딩 금지**. 전부 `StdAccel` 열거로 요청하고 어댑터가 해석한다 — 이러면 새 플랫폼 추가 시 컨트롤 코드를 건드리지 않는다.

---

## 7. 성능 규약 — "빠르게 동작한다"

| 항목 | 규칙 |
|---|---|
| 입력→반영 | **≤16ms**(NFR-B-12). hover·press는 **다음 프레임 안에** |
| **부분 무효화** | 상태 변화는 **그 컨트롤의 사각형만** 무효화. 전체 재그리기 금지 |
| 목록 | **가상화 필수** — 보이는 행만 생성·측정·그리기(`nexa-dir2` `rows.rs`가 100k 행에서 검증) |
| 레이아웃 | 변경 없으면 재계산 금지. `measure` 결과 캐시 |
| 텍스트 | 셰이핑 결과 캐시(문자열+폰트+크기 키). 매 프레임 재셰이핑 금지 |
| 애니메이션 | 진행 중인 것이 없으면 **프레임을 요청하지 않는다**(유휴 CPU ≤0.2% — NFR-B-7) |
| 입력 우선 | 입력 처리 > 렌더. 입력이 밀리면 프레임을 건너뛴다 |
| 이미지 | 디코드는 `imgdec`, UI에는 **재인코딩본**([04 §5](04-safe-transfer.md)). 렌더 스레드에서 디코드 금지 |
| 측정 | hover/press 반응 지연을 **계측 대상**으로([13 §6](13-code-design-standards.md)) |

---

## 8. 감수하는 대가

| 대가 | 판단 |
|---|---|
| **Windows·Linux 사용자에게 이질적** | 시각을 맥으로 통일하면 그 OS 사용자는 "이 프로그램 생김새"로 인식한다. Slack·Spotify 등이 같은 길을 갔다. **동작 관례는 네이티브**라 손에는 익는다 |
| **시스템 테마 미추종** | 우리 팔레트가 정본. 다크/라이트 전환만 따라간다. 맥 시스템 강조색 추종은 **선택 옵션** |
| **접근성** | 자체 렌더링이라 v2(R-9). 단 **`A11yRole`을 v1부터 채워** 나중에 노출만 붙인다 |
| 플랫폼 고유 UI 부재 | 맥 시트·윈도우 스낵바 같은 OS 고유 표현을 그대로 쓰지 않는다 |

---

## 9. 미확정 (M3에서 확정)

| # | 항목 |
|---|---|
| 1 | 라운드 반경·간격·타이포 **수치표** — 시안 대조 후 |
| 2 | 맥 시스템 강조색 추종 기본값(켬/끔) |
| 3 | 말풍선 레이아웃(정렬·꼬리 유무·그룹핑 규칙) |
| 4 | 신뢰 상태 배지 3종(미검증/검증됨/대조 완료)의 시각 언어 |
| 5 | Compact 밀도의 구체 수치 |
| 6 | 오버레이 스크롤바 등장/소멸 타이밍 |

---

> 관련: [ADR-0001](07-adr-0001-stack.md) · [13 코드 설계 표준](13-code-design-standards.md)(`ActionKind`·인터셉터) · [12 차용 자산](12-asset-reuse.md)(`nexa-gui` 이식 · `ctl` 시각 규약) · [05](05-requirements.md) FR-U-*
