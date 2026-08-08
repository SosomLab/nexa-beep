//! 정수 픽셀 기하 — 창 클라이언트 좌표계(좌상 원점, px).
//!
//! `nexa-dir2/crates/nexa-gui/src/geom.rs` 이식([docs/12 §A] — 플랫폼 참조 0의 검증 자산).

/// 점(px).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    /// x(px).
    pub x: i32,
    /// y(px).
    pub y: i32,
}

/// 크기(px).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Size {
    /// 폭(px).
    pub w: i32,
    /// 높이(px).
    pub h: i32,
}

/// 사각형(px) — `contains`는 반열린 구간(`[x, x+w)`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    /// 왼쪽.
    pub x: i32,
    /// 위.
    pub y: i32,
    /// 폭.
    pub w: i32,
    /// 높이.
    pub h: i32,
}

impl Rect {
    /// 생성.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// 오른쪽 경계(exclusive).
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    /// 아래 경계(exclusive).
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// 크기.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size {
            w: self.w,
            h: self.h,
        }
    }

    /// 넓이 0 여부.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// 점 포함(반열린).
    #[must_use]
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// 교차 여부(변 접촉은 비교차).
    #[must_use]
    pub fn intersects(&self, other: &Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// 두 rect를 덮는 최소 rect. 빈 rect는 항등원.
    #[must_use]
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.contains(Point { x: 0, y: 0 }));
        assert!(r.contains(Point { x: 9, y: 9 }));
        assert!(!r.contains(Point { x: 10, y: 9 }));
    }

    #[test]
    fn union_with_empty_is_identity() {
        let r = Rect::new(5, 5, 10, 10);
        assert_eq!(r.union(&Rect::default()), r);
        assert_eq!(Rect::default().union(&r), r);
    }

    #[test]
    fn union_covers_both() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(20, 5, 10, 10);
        assert_eq!(a.union(&b), Rect::new(0, 0, 30, 15));
    }

    #[test]
    fn intersects_excludes_touching_edges() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.intersects(&Rect::new(9, 9, 5, 5)));
        assert!(!a.intersects(&Rect::new(10, 0, 5, 5)));
        assert!(!a.intersects(&Rect::new(0, 0, 0, 5)));
    }
}
