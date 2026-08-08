//! 플랫폼 중립 입력 이벤트 — `nbeep-plat`이 OS 이벤트를 이 타입으로 번역해 위젯에 라우팅.
//!
//! `nexa-dir2/crates/nexa-gui/src/event.rs` 이식([docs/12 §A]). `now_ms` 주입(테스트 가능성)과
//! 분수 노치 휠 누적기가 검증 자산의 핵심이다. 3단계 전파(캡처→타겟→버블 — [docs/14 §3])는
//! M3-1d에서 이 타입 위에 얹는다.

/// 휠 1노치의 delta 단위(플랫폼 어댑터가 이 단위로 정규화한다).
pub const WHEEL_DELTA: i32 = 120;

/// 네비게이션 키(키보드 우선 FR-U-4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// ↑.
    Up,
    /// ↓.
    Down,
    /// PgUp.
    PageUp,
    /// PgDn.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// →.
    Right,
    /// ←.
    Left,
    /// 스페이스(선택 토글 — 타입어헤드에서 제외).
    Space,
    /// Enter(대화 열기·확정).
    Enter,
    /// Esc(취소·닫기).
    Escape,
}

/// 위젯이 받는 입력 이벤트.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputEvent {
    /// 세로 휠 원시 delta([`WHEEL_DELTA`] 단위 — 트랙패드는 분수 노치). 양수 = 위로.
    Wheel {
        /// delta.
        delta: i32,
    },
    /// 가로 휠(Shift+휠 포함). 양수 = 오른쪽.
    HWheel {
        /// delta.
        delta: i32,
    },
    /// 키 입력 + 수식키. `shift` = 범위 선택, `primary` = OS 주 수식키(mac ⌘/기타 Ctrl —
    /// 번역은 `PlatformConventions`([docs/14 §6]) 몫).
    Key {
        /// 키.
        key: Key,
        /// 범위 선택.
        shift: bool,
        /// 주 수식키.
        primary: bool,
    },
    /// 인쇄 가능 문자(타입어헤드). `'\u{8}'` = Backspace(접두사 축소).
    /// `now_ms` = 단조 시각(밀리초) **주입** — 버퍼 타임아웃 판정이 테스트 가능해진다.
    Char {
        /// 문자.
        c: char,
        /// 단조 시각(ms).
        now_ms: u64,
    },
    /// 전체 선택(⌘/Ctrl+A).
    SelectAll,
    /// 좌클릭(클라이언트 좌표). `shift` = 범위, `primary` = 비연속 토글.
    MouseDown {
        /// x.
        x: i32,
        /// y.
        y: i32,
        /// 범위 선택.
        shift: bool,
        /// 주 수식키.
        primary: bool,
    },
    /// 우클릭(컨텍스트 메뉴).
    RightDown {
        /// x.
        x: i32,
        /// y.
        y: i32,
    },
    /// 이동(버튼 상태 무관 — 위젯이 드래그 상태를 보유).
    MouseMove {
        /// x.
        x: i32,
        /// y.
        y: i32,
    },
    /// 버튼 해제.
    MouseUp {
        /// x.
        x: i32,
        /// y.
        y: i32,
    },
}

/// 분수 노치 휠 누적기 — 트랙패드 분수 delta를 잔여 누적(이월 손실 없음).
#[derive(Clone, Copy, Default, Debug)]
pub struct WheelAccum {
    accum: i32,
}

impl WheelAccum {
    /// delta를 누적하고 이번에 스크롤할 행 수를 반환(양수 = 위로). 잔여는 다음 호출로 이월.
    pub fn add(&mut self, delta: i32, lines_per_notch: i32) -> i32 {
        self.accum += delta;
        let lines = self.accum * lines_per_notch / WHEEL_DELTA;
        if lines != 0 {
            self.accum -= lines * WHEEL_DELTA / lines_per_notch;
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_notch_scrolls_lines_per_notch() {
        let mut w = WheelAccum::default();
        assert_eq!(w.add(WHEEL_DELTA, 3), 3);
        assert_eq!(w.add(-WHEEL_DELTA, 3), -3);
    }

    #[test]
    fn fractional_notches_accumulate() {
        let mut w = WheelAccum::default();
        assert_eq!(w.add(40, 3), 1);
        assert_eq!(w.add(40, 3), 1);
        assert_eq!(w.add(40, 3), 1);
    }

    #[test]
    fn remainder_carries_over_without_loss() {
        let mut w = WheelAccum::default();
        let mut total = 0;
        for _ in 0..12 {
            total += w.add(30, 3);
        }
        assert_eq!(total, 9);
    }
}
