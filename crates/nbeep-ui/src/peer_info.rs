//! **상대 프로필 보기**(M3-17 · 목록 우클릭 ▸ "프로필 보기") — 읽기 전용 카드.
//!
//! 큰 원형 이니셜 아바타 + 기본(발견) 이름/프로필 이름/연락처/이미지 상태/키 지문.
//! 실제 사진 렌더는 M4-5(imgdec) 후 — 그때까지 "이미지 캐시됨"으로 존재만 알린다.

use crate::controls::Control as _;
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
    /// 프로필 수신 시각 표시 문자열(M3-21 ③ — 카드 신선도 · 빈 = 수신 기록 없음).
    pub received: String,
    /// 아바타 보더 색(08-15 — 상대가 공개한 값 · 검증 통과분). 큰 프리뷰 = 3px.
    pub border: Option<(u8, u8, u8)>,
    /// 안전 번호(M3-6 · SAS 60자리 — 5자리×12그룹 공백 구분). 두 사람 화면에 같은
    /// 값이 나온다(개시자 무관 정렬) — **다른 채널**(전화·대면)로 직접 대조한다.
    pub safety_number: String,
    /// 이미 지문 대조 완료(`FingerprintVerified`) — true면 버튼 대신 완료 표시.
    pub verified: bool,
}

/// 상대 프로필 카드 위젯.
#[derive(Debug)]
pub struct PeerInfoWidget {
    bounds: Rect,
    scale: f32,
    info: PeerInfo,
    closed: bool,
    /// "대조 완료로 표시" 버튼(M3-6) — 이미 검증됐으면 그리지 않는다.
    verify: crate::controls::Button,
    /// 대조 완료 요청(1회성 — 호스트가 신뢰 저장소에 승격 반영).
    verify_req: bool,
    /// "인증 취소" 버튼 — **검증 완료 상태에서만** 그린다(`/unverify`와 같은 동작).
    unverify: crate::controls::Button,
    /// 인증 취소 요청(1회성 — 호스트가 `unverify(peer)` + 영속 + 배지 강등).
    unverify_req: bool,
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
            verify: crate::controls::Button::new("일치 확인 — 대조 완료로 표시"),
            verify_req: false,
            unverify: crate::controls::Button::new("인증 취소"),
            unverify_req: false,
        }
    }

    /// 대조 완료 요청(1회성) — 호스트가 `verify(peer)` + 영속 + 배지 갱신을 맡는다.
    pub fn take_verify(&mut self) -> bool {
        std::mem::take(&mut self.verify_req)
    }

    /// 인증 취소 요청(1회성) — 호스트가 `unverify(peer)` + 영속 + 배지 강등을 맡는다.
    pub fn take_unverify(&mut self) -> bool {
        std::mem::take(&mut self.unverify_req)
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
        // 대조 버튼(M3-6) — 하단 안내 위 · 중앙(결정적 순간의 단일 표적).
        let (bw, bh) = (self.s(230), self.s(30));
        let slot_y = bounds.bottom() - self.s(72);
        self.verify.set_scale(self.scale);
        self.verify.set_bounds(
            Rect::new(bounds.x + (bounds.w - bw) / 2, slot_y, bw, bh),
            inv,
        );
        // 인증 취소 버튼 — 검증 완료 상태 전용. 좁게(140) 중앙, 같은 하단 슬롯
        // (verify와 상호 배타라 자리를 공유한다).
        let uw = self.s(140);
        self.unverify.set_scale(self.scale);
        self.unverify.set_bounds(
            Rect::new(bounds.x + (bounds.w - uw) / 2, slot_y, uw, bh),
            inv,
        );
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        if matches!(
            *ev,
            InputEvent::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.closed = true;
        }
        // 대조 버튼(M3-6) — 검증 전엔 "대조 완료", 검증 후엔 "인증 취소"(상호 배타).
        if self.info.verified {
            self.unverify.on_event(ev, inv);
            if self.unverify.take_clicked() {
                self.unverify_req = true;
            }
        } else {
            self.verify.on_event(ev, inv);
            if self.verify.take_clicked() {
                self.verify_req = true;
            }
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        // 큰 아바타(목록 40의 3배 = 120) — 가운데 상단.
        let d = self.s(120);
        let av = Rect::new(b.x + (b.w - d) / 2, b.y + self.s(20), d, d);
        if let Some(img) = &self.info.avatar {
            // 목록 행과 같은 문법(08-14) — 원 배경을 깔고 얹는다(내장 투명 배경 대비).
            ctx.fill_ellipse(av, crate::avatar::avatar_color(&self.info.seed));
            ctx.image_scaled(av, img, b);
        } else {
            crate::avatar::draw_avatar(ctx, av, &self.info.name, &self.info.seed, 34.0);
        }
        // 아바타 보더(08-15 사용자 실기 — 카드에서만 빠져 있었다) — 큰 프리뷰 3px
        // (프로필 화면과 같은 확정 규약 · 목록 소형은 2px).
        if let Some((br, bg, bb)) = self.info.border {
            let c =
                crate::theme::Color((u32::from(br) << 16) | (u32::from(bg) << 8) | u32::from(bb));
            ctx.stroke_ellipse(av, c, self.s(3).max(3) as f32);
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
        // 신선도(M3-21 ③) — 이 카드 내용이 언제 받은 것인지. 캐시가 낡았을 수
        // 있음을 사용자가 안다(카드 열기 pull이 곧 갱신하지만, 비연결이면 이대로).
        let img = match (self.info.has_image, self.info.received.is_empty()) {
            (true, false) => format!("이미지  ·  캐시됨  ·  {} 수신", self.info.received),
            (true, true) => "이미지  ·  캐시됨".into(),
            (false, false) => format!("프로필  ·  {} 수신", self.info.received),
            (false, true) => "이미지  ·  (없음/비공개)".into(),
        };
        ctx.text(x, y, b, &img, theme.text_dim);
        y += ctx.text_height() + self.s(8);
        ctx.text(
            x,
            y,
            b,
            &format!(
                "{}  ·  {}",
                nbeep_core::t(nbeep_core::Msg::FingerprintLabel),
                self.info.fingerprint
            ),
            theme.text_dim,
        );
        y += ctx.text_height() + self.s(14);
        // ── 안전 번호(M3-6 · SAS) — 두 화면에 같은 60자리가 뜬다. 고정폭으로
        //    6그룹×2줄(줄이 흔들리면 눈 대조가 어긋난다 — Mono 슬롯이 그 자리).
        if !self.info.safety_number.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            ctx.text(x, y, b, "안전 번호  ·  전화·대면으로 직접 대조", theme.text);
            y += ctx.text_height() + self.s(6);
            ctx.select_font(FontSlot::Mono, false);
            let groups: Vec<&str> = self.info.safety_number.split(' ').collect();
            for line in groups.chunks(6) {
                let t = line.join("  ");
                let tw = ctx.text_width(&t);
                ctx.text(b.x + (b.w - tw) / 2, y, b, &t, theme.text);
                y += ctx.text_height() + self.s(4);
            }
            ctx.select_font(FontSlot::Status, false);
            if self.info.verified {
                // 완료 문구는 흐름 y에, "인증 취소" 버튼은 하단 슬롯에(겹치지 않게
                // 버튼 위에 고정 배치 — 흐름 y가 슬롯을 침범하면 위로 끌어올린다).
                let t = "✓ 지문 대조 완료 — 이 키는 사람이 확인했습니다";
                let tw = ctx.text_width(t);
                let btn_top = self.unverify.bounds().y;
                let ty = (y + self.s(6)).min(btn_top - ctx.text_height() - self.s(10));
                ctx.text(b.x + (b.w - tw) / 2, ty, b, t, theme.ok);
                self.unverify.paint(ctx, theme);
            } else {
                self.verify.paint(ctx, theme);
            }
        }
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

    /// SAS 대조 버튼(M3-6) — 클릭 = 1회성 요청 · 이미 검증됐으면 표적이 없다.
    #[test]
    fn verify_button_requests_once_and_only_when_unverified() {
        let mut w = PeerInfoWidget::new(PeerInfo {
            safety_number: "12345 67890".into(),
            ..PeerInfo::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 520), &mut inv);
        let b = self_btn_bounds(&w);
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: cx,
                y: cy,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::MouseUp { x: cx, y: cy }, &mut inv);
        assert!(w.take_verify(), "클릭 = 대조 완료 요청");
        assert!(!w.take_verify(), "1회성");
        // 검증 완료 상태 — 버튼 이벤트가 무시된다.
        let mut v = PeerInfoWidget::new(PeerInfo {
            safety_number: "12345 67890".into(),
            verified: true,
            ..PeerInfo::default()
        });
        v.set_bounds(Rect::new(0, 0, 360, 520), &mut inv);
        let b2 = self_btn_bounds(&v);
        let (cx2, cy2) = (b2.x + b2.w / 2, b2.y + b2.h / 2);
        v.on_event(
            &InputEvent::MouseDown {
                x: cx2,
                y: cy2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        v.on_event(&InputEvent::MouseUp { x: cx2, y: cy2 }, &mut inv);
        assert!(!v.take_verify(), "검증 후엔 요청 없음");
    }

    fn self_btn_bounds(w: &PeerInfoWidget) -> Rect {
        w.verify.bounds()
    }

    /// 인증 취소 버튼 — **검증 완료 상태에서만** 산다 · 클릭 = 1회성 요청.
    #[test]
    fn unverify_button_requests_once_and_only_when_verified() {
        let mut w = PeerInfoWidget::new(PeerInfo {
            safety_number: "12345 67890".into(),
            verified: true,
            ..PeerInfo::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 520), &mut inv);
        let b = w.unverify.bounds();
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: cx,
                y: cy,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::MouseUp { x: cx, y: cy }, &mut inv);
        assert!(w.take_unverify(), "클릭 = 인증 취소 요청");
        assert!(!w.take_unverify(), "1회성");
        // 미검증 상태 — 인증 취소 표적이 없다(대조 버튼만 산다).
        let mut u = PeerInfoWidget::new(PeerInfo {
            safety_number: "12345 67890".into(),
            ..PeerInfo::default()
        });
        u.set_bounds(Rect::new(0, 0, 360, 520), &mut inv);
        let b2 = u.unverify.bounds();
        let (cx2, cy2) = (b2.x + b2.w / 2, b2.y + b2.h / 2);
        u.on_event(
            &InputEvent::MouseDown {
                x: cx2,
                y: cy2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        u.on_event(&InputEvent::MouseUp { x: cx2, y: cy2 }, &mut inv);
        assert!(!u.take_unverify(), "미검증 상태엔 인증 취소 요청 없음");
    }

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
