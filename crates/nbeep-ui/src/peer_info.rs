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

/// 문자열을 `maxw`(px) 안에 들도록 말줄임(…)한다(08-17 — 소개글 1줄 고정).
/// 들어가면 원문 그대로. 문자 경계 누적 폭으로 한 번에 자른다.
fn fit_ellipsis(ctx: &mut dyn DrawCtx, s: &str, maxw: i32) -> String {
    if ctx.text_width(s) <= maxw {
        return s.to_string();
    }
    let mut w = Vec::new();
    ctx.text_prefix_widths(s, &mut w);
    let ell = ctx.text_width("…");
    let chars: Vec<char> = s.chars().collect();
    let mut n = 0;
    for i in 0..chars.len() {
        if w.get(i + 1).copied().unwrap_or(i32::MAX) + ell > maxw {
            break;
        }
        n = i + 1;
    }
    let mut out: String = chars[..n].iter().collect();
    out.push('…');
    out
}

/// 카드에 실을 내용(호스트가 채운다 — 위젯은 출처를 모른다).
#[derive(Debug, Default, Clone)]
pub struct PeerInfo {
    /// 기본(발견) 이름.
    pub name: String,
    /// 프로필에 등록된 표시 이름(없으면 빈 값). 08-17 — 큰 이름은 이게 있으면 이걸.
    pub profile_name: String,
    /// 소개글(08-17 · 큰 이름 아래 회색 줄 — 목록과 같은 자리 · 줄바꿈은 접는다).
    pub bio: String,
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
            // 08-17 사용자 확정 — 대조 완료 = 안전(초록) · 인증 취소 = 되돌림(붉은
            // 벽돌). 흰 글씨. 카드 본문 폰트(Status)에 맞춘다.
            verify: crate::controls::Button::new(nbeep_core::t(nbeep_core::Msg::CardVerifyBtn))
                .with_tone(crate::controls::ButtonTone::Safe)
                .with_font(crate::draw::FontSlot::Status),
            verify_req: false,
            unverify: crate::controls::Button::new(nbeep_core::t(nbeep_core::Msg::CardUnverifyBtn))
                .with_tone(crate::controls::ButtonTone::Danger)
                .with_font(crate::draw::FontSlot::Status),
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
        // 대조/인증 취소 버튼(M3-6) — 하단 안내 위 · 중앙. 08-17: 카드 본문
        // 폰트(Status)에 맞춰 **작게**(높이 26 · 폭도 축소).
        let bh = self.s(26);
        let slot_y = bounds.bottom() - self.s(64);
        let bw = self.s(190);
        self.verify.set_scale(self.scale);
        self.verify.set_bounds(
            Rect::new(bounds.x + (bounds.w - bw) / 2, slot_y, bw, bh),
            inv,
        );
        // 인증 취소 — 좁게(110) 중앙, 같은 슬롯(verify와 상호 배타).
        let uw = self.s(110);
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
        use nbeep_core::{t, Msg};
        // 큰 이름(굵게 · 중앙) = 표시 이름(프로필名 우선·없으면 발견名 · 08-17
        // — 목록과 같은 규약. 신원은 여전히 아래 키 지문).
        let big = if self.info.profile_name.is_empty() {
            &self.info.name
        } else {
            &self.info.profile_name
        };
        ctx.select_font_sized(FontSlot::Base, true, 3.0);
        let tw = ctx.text_width(big);
        ctx.text(
            b.x + (b.w - tw) / 2,
            av.bottom() + self.s(14),
            b,
            big,
            theme.text,
        );
        // 소개글(있으면 아래 — 회색·작은 폰트·줄바꿈 접음 · 08-17 사용자 요청:
        // 종전엔 이름이 두 번 나왔다). 목록 2번째 줄과 같은 자리·규약.
        let mut y = av.bottom() + self.s(14) + ctx.text_height() + self.s(6);
        let bio_one: String = self
            .info
            .bio
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !bio_one.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            // ★ 항상 1줄(08-17 사용자 확정 — 워드랩 없음): 카드 폭을 넘으면 말줄임
            //   후 좌측 정렬, 들어가면 가운데. 여러 줄·긴 소개도 한 줄로만.
            let avail = b.w - self.s(28) * 2;
            let fitted = fit_ellipsis(ctx, &bio_one, avail);
            let tw = ctx.text_width(&fitted);
            let bx = if tw < avail {
                b.x + (b.w - tw) / 2
            } else {
                b.x + self.s(28)
            };
            ctx.text(bx, y, b, &fitted, theme.text_dim);
            y += ctx.text_height() + self.s(12);
        } else {
            y += self.s(8);
        }
        // 상세 행(라벨 · 값) — 빈 값은 (비공개)/(기록 없음)으로 명시. 08-17 i18n.
        ctx.select_font(FontSlot::Status, false);
        let rows: [(&str, &str, bool); 4] = [
            (t(Msg::FieldEmail), &self.info.email, false),
            (t(Msg::FieldPhone), &self.info.phone, false),
            (t(Msg::CardLastSeen), &self.info.last_seen, true),
            (t(Msg::CardLastChat), &self.info.last_chat, true),
        ];
        let x = b.x + self.s(28);
        for (label, val, is_time) in rows {
            // 연락처의 빈 값 = 미공개, 시각의 빈 값 = 기록 없음(뜻이 다르다).
            let shown = if val.is_empty() {
                if is_time {
                    t(Msg::CardNoRecord)
                } else {
                    t(Msg::CardPrivate)
                }
            } else {
                val
            };
            ctx.text(x, y, b, &format!("{label}  ·  {shown}"), theme.text);
            y += ctx.text_height() + self.s(8);
        }
        // 신선도(M3-21 ③) — 이 카드 내용이 언제 받은 것인지(08-17 i18n).
        let img = match (self.info.has_image, self.info.received.is_empty()) {
            (true, false) => nbeep_core::tf(Msg::CardImageCachedAt, &[&self.info.received]),
            (true, true) => t(Msg::CardImageCached).to_string(),
            (false, false) => nbeep_core::tf(Msg::CardProfileAt, &[&self.info.received]),
            (false, true) => t(Msg::CardImageNone).to_string(),
        };
        ctx.text(x, y, b, &img, theme.text_dim);
        y += ctx.text_height() + self.s(8);
        ctx.text(
            x,
            y,
            b,
            &format!("{}  ·  {}", t(Msg::FingerprintLabel), self.info.fingerprint),
            theme.text_dim,
        );
        y += ctx.text_height() + self.s(14);
        // ── 지문 대조(M3-6) — 08-17 사용자 확정: 60자리 SAS는 숨기고 위 키 지문만
        //    다른 채널로 대조. safety_number는 "대조 가능한 상대" 게이트로만 쓴다.
        if !self.info.safety_number.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            ctx.text(x, y, b, t(Msg::CardVerifyPrompt), theme.text);
            y += ctx.text_height() + self.s(6);
            if self.info.verified {
                // 완료 문구는 흐름 y, 버튼은 하단 슬롯(겹치면 문구를 위로 끌어올림).
                let vt = t(Msg::CardVerified);
                let tw = ctx.text_width(vt);
                let btn_top = self.unverify.bounds().y;
                let ty = (y + self.s(6)).min(btn_top - ctx.text_height() - self.s(10));
                ctx.text(b.x + (b.w - tw) / 2, ty, b, vt, theme.ok);
                self.unverify.paint(ctx, theme);
            } else {
                self.verify.paint(ctx, theme);
            }
        }
        // 하단 안내 — 08-17: 이메일 행과 같은 작은 폰트(Status) · i18n.
        ctx.select_font(FontSlot::Status, false);
        ctx.text(
            x,
            b.bottom() - self.s(24),
            b,
            t(Msg::CardFooter),
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
