//! 설정 초기화 원천 점검(08-15 사용자 요청 "기본값 초기화에서도 정상 초기화").
//!
//! 초기화(do_reset_settings)는 레지스트리 항목의 **기본값 쌍 전체**를 돌린다 —
//! `e.key` 하나만 돌던 시절 값 키가 여럿인 항목(FontSection = family+size)의
//! 짝 키가 새던 회귀를 박제한다. 기본값 정책(사용자 확정)도 여기서 고정한다.

use nbeep_ui::settings::{registry, SettingKind, SettingsState};

/// 초기화가 도는 것과 같은 (키, 기본값) 전개.
fn reset_pairs() -> Vec<(&'static str, String)> {
    registry()
        .iter()
        .filter(|e| !matches!(e.kind, SettingKind::Action { .. }))
        .flat_map(nbeep_ui::settings::Entry::default_values)
        .collect()
}

#[test]
fn reset_covers_every_value_key_of_every_entry() {
    let pairs = reset_pairs();
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
    // ★ 값 키가 여럿인 항목의 짝 키 — e.key만 돌면 여기서 샌다(실제로 샜던 회귀).
    assert!(keys.contains(&"font.base.family"));
    assert!(keys.contains(&"font.base.size"), "FontSection 짝 키(size)");
    // 시드(with_defaults)와 초기화가 같은 원천을 쓴다 — 어긋나면 초기화 ≠ 첫 실행.
    let seeded = SettingsState::with_defaults();
    for (k, d) in &pairs {
        assert_eq!(seeded.get(k), d, "{k}: 시드와 초기화 기본값이 다르다");
    }
}

#[test]
fn confirmed_defaults_hold() {
    let d = SettingsState::with_defaults();
    // M3-2d 사용자 확정(08-15) — 3-OS 공통 기본 꺼짐(mac 차등 아님).
    assert_eq!(d.get("ui.close_to_tray"), "off");
    // 프로필 공개 = 기본 비노출(DR-22 옵트인).
    for k in [
        "profile.share.basic",
        "profile.share.email",
        "profile.share.phone",
    ] {
        assert_eq!(d.get(k), "off", "{k}");
    }
    // 세션 배지 실루엣(M3-19) — 기본 켬.
    assert_eq!(d.get("ui.link_badge_shape"), "on");
    // 목록 정렬 기본(08-15 사용자 확정) — 최근 대화 우선.
    assert_eq!(d.get("ui.list_sort"), "chat");
    // 알림(M3-8) — 표시는 기본 켬 · 본문 미리보기는 기본 끔(화면 공유 안전 — FR-S-42 결).
    assert_eq!(d.get("notify.enabled"), "on");
    assert_eq!(d.get("notify.preview"), "off");
}
