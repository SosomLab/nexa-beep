//! 피어 목록 뷰 — M1-6 최소 경로(FR-D-2 · 미검증 배지).
//!
//! [`nbeep_core`]의 목록([`PeerEntry`])과 신뢰([`TrustLevel`])를 읽어 [`nbeep_gfx`]로 그린다.
//! **신뢰 배지 3종은 항상 붙는다**(미검증/핀 고정/대조 완료 — [docs/08] "미검증으로 표시한다").
//! 색·수치는 임시 팔레트다 — macOS 시각 언어 수치표(M3-1c)가 확정되면 `theme`으로 이관한다.

use nbeep_core::peers::PeerEntry;
use nbeep_core::TrustLevel;
use nbeep_gfx::{Color, Font, Surface};

/// 목록 한 행 — 목록 항목 + 신뢰 상태(둘의 출처가 다르므로 조립 지점에서 합친다:
/// 목록은 발견, 신뢰는 `TrustStore`).
#[derive(Clone, Debug)]
pub struct PeerRow {
    /// 발견 목록 항목.
    pub entry: PeerEntry,
    /// 신뢰 상태(배지 결정).
    pub trust: TrustLevel,
}

/// 행 높이(px) — 임시. M3-1c 수치표에서 확정.
pub const ROW_H: u32 = 36;

/// 신뢰 배지 라벨·색(임시 팔레트).
#[must_use]
pub fn badge(trust: TrustLevel) -> (&'static str, Color) {
    match trust {
        TrustLevel::Unverified => ("미검증", Color(0x0080_5020)), // 주황 계열
        TrustLevel::Pinned => ("핀 고정", Color(0x0030_5580)),    // 파랑 계열
        TrustLevel::FingerprintVerified => ("대조 완료", Color(0x002E_6B3A)), // 초록 계열
    }
}

/// 피어 목록을 표면에 그린다(스크롤 없는 최소 경로 — 넘치는 행은 표면이 클립).
pub fn render(surface: &mut Surface<'_>, font: &Font, rows: &[PeerRow]) {
    surface.fill(Color(0x0020_2124)); // 배경(다크 임시)
    let size = 15.0;
    let width = u32::try_from(surface.width()).unwrap_or(u32::MAX);

    for (i, row) in rows.iter().enumerate() {
        let top = i32::try_from(i as u64 * u64::from(ROW_H)).unwrap_or(i32::MAX);
        // 행 구분선.
        surface.fill_rect(
            0,
            top + i32::try_from(ROW_H).unwrap_or(0) - 1,
            width,
            1,
            Color(0x002A_2B2F),
        );
        let baseline = top as f32 + f32::from(u16::try_from(ROW_H).unwrap_or(36)) * 0.65;

        // 이름(왼쪽) — 이미 무해화된 DisplayName만 이 타입에 담긴다.
        font.draw_text(
            surface,
            12.0,
            baseline,
            size,
            Color(0x00E8_E8E8),
            row.entry.name.as_str(),
        );

        // 다중 경로 표시(진단) — 경로 2개 이상이면 "×N".
        if row.entry.paths > 1 {
            let label = format!("×{}", row.entry.paths);
            let name_w = font.measure(row.entry.name.as_str(), size);
            font.draw_text(
                surface,
                16.0 + name_w,
                baseline,
                11.0,
                Color(0x0088_8888),
                &label,
            );
        }

        // 신뢰 배지(오른쪽 정렬) — 항상 표시.
        let (label, bg) = badge(row.trust);
        let text_w = font.measure(label, 12.0);
        let pad = 8.0;
        let chip_w = text_w + pad * 2.0;
        let chip_x = surface.width() as f32 - chip_w - 10.0;
        surface.fill_rect(chip_x as i32, top + 8, chip_w as u32, ROW_H - 16, bg);
        font.draw_text(
            surface,
            chip_x + pad,
            baseline - 1.0,
            12.0,
            Color(0x00F0_F0F0),
            label,
        );
    }
}
