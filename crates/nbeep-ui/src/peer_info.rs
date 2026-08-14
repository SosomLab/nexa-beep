//! **상대 프로필 보기**(M3-17 · 목록 우클릭 ▸ "프로필 보기") — 읽기 전용 카드.
//!
//! 큰 원형 이니셜 아바타 + 기본(발견) 이름/프로필 이름/연락처/이미지 상태/키 지문.
//! 실제 사진 렌더는 M4-5(imgdec) 후 — 그때까지 "이미지 캐시됨"으로 존재만 알린다.

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 카드에 실을 내용(호스트가 채운다 — 위젯은 출처를 모른다).
#[derive(Debug, Default, Clone)]
pub struct PeerInfo {
    /// 기본(발견) 이름.
    pub name: String,
    /// 프로필에 등록된 표시 이름(없으면 빈 값).
    pub profile_name: String,
    /// 이메일(없으면 빈 값).
    pub email: String,
    /// 전화번호(없으면 빈 값).
    pub phone: String,
    /// 프로필 이미지 캐시 여부.
    pub has_image: bool,
    /// 키 지문(짧은 표기).
    pub fingerprint: String,
    /// 아바타 색 시드(키 지문 바이트).
    pub seed: Vec<u8>,
    /// 프로필 사진(M4-5 imgdec — 원형 마스크 완료본). 없으면 이니셜.
    pub avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 최근 접속 표시 문자열(08-15 — 호스트가 시각을 사람 표기로 변환 · 빈 = 기록 없음).
    pub last_seen: String,
    /// 최근 대화 표시 문자열(08-15 — 빈 = 기록 없음).
    pub last_chat: String,
}

/// 상대 프로필 카드 위젯.
#[derive(Debug)]
pub struct PeerInfoWidget {
    bounds: Rect,
    scale: f32,
    info: PeerInfo,
    closed: bool,
}

impl PeerInfoWidget {
    /// 내용으로 만든다.
    #[must_use]
    pub fn new(info: PeerInfo) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            info,
            closed: false,
        }
    }

    /// 닫기 요청(1회성 · Esc).
    pub fn take_closed(&mut self) -> bool {
        std::mem::take(&mut self.closed)
    }

    /// 배율.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        inv.push(self.bounds);
    }

    fn s(&self, v: i32) -> i32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let r = (v as f32 * self.scale).round() as i32;
        r
    }
}

impl Widget for PeerInfoWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, _inv: &mut Invalidations) {
        if matches!(
            *ev,
            InputEvent::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.closed = true;
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        // 큰 아바타(목록 40의 3배 = 120) — 가운데 상단.
        let d = self.s(120);
        let av = Rect::new(b.x + (b.w - d) / 2, b.y + self.s(20), d, d);
        if let Some(img) = &self.info.avatar {
            ctx.image_scaled(av, img, b);
        } else {
            crate::avatar::draw_avatar(ctx, av, &self.info.name, &self.info.seed, 34.0);
        }
        // 기본 이름(굵게 · 중앙).
        ctx.select_font_sized(FontSlot::Base, true, 3.0);
        let tw = ctx.text_width(&self.info.name);
        ctx.text(
            b.x + (b.w - tw) / 2,
            av.bottom() + self.s(14),
            b,
            &self.info.name,
            theme.text,
        );
        // 프로필 이름(있으면 아래 — 회색).
        let mut y = av.bottom() + self.s(14) + ctx.text_height() + self.s(6);
        if !self.info.profile_name.is_empty() {
            ctx.select_font(FontSlot::Base, false);
            let tw = ctx.text_width(&self.info.profile_name);
            ctx.text(
                b.x + (b.w - tw) / 2,
                y,
                b,
                &self.info.profile_name,
                theme.text_dim,
            );
            y += ctx.text_height() + self.s(12);
        } else {
            y += self.s(8);
        }
        // 상세 행(라벨: 값) — 없는 항목은 "(비공개)"로 명시(없음과 미공개를 숨기지 않는다).
        ctx.select_font(FontSlot::Status, false);
        let rows = [
            ("이메일", &self.info.email),
            ("전화번호", &self.info.phone),
            ("최근 접속", &self.info.last_seen),
            ("최근 대화", &self.info.last_chat),
        ];
        let x = b.x + self.s(28);
        for (label, val) in rows {
            // 연락처의 빈 값 = 미공개, 시각의 빈 값 = 기록 없음(뜻이 다르다).
            let shown = if val.is_empty() {
                if matches!(label, "최근 접속" | "최근 대화") {
                    "(기록 없음)"
                } else {
                    "(비공개)"
                }
            } else {
                val
            };
            ctx.text(x, y, b, &format!("{label}  ·  {shown}"), theme.text);
            y += ctx.text_height() + self.s(8);
        }
        let img = if self.info.has_image {
            "이미지  ·  캐시됨(렌더는 imgdec 후)"
        } else {
            "이미지  ·  (없음/비공개)"
        };
        ctx.text(x, y, b, img, theme.text_dim);
        y += ctx.text_height() + self.s(8);
        ctx.text(
            x,
            y,
            b,
            &format!("키 지문  ·  {}", self.info.fingerprint),
            theme.text_dim,
        );
        // 하단 안내.
        ctx.text(
            x,
            b.bottom() - self.s(26),
            b,
            "Esc = 닫기 · 신원은 이름이 아니라 키 지문입니다",
            theme.text_dim,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_requests_close_once() {
        let mut w = PeerInfoWidget::new(PeerInfo::default());
        let mut inv = Invalidations::default();
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_closed());
        assert!(!w.take_closed(), "1회성");
    }
}
