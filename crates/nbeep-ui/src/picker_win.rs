//! **파일 선택 대화상자**(docs/50 P3 · ADR-0014 정정) — 본체는 nexa-ui `nexa-dlg` [`FilePicker`]
//! (3-OS 동일 자체 그리기 · 네이티브 대화상자 0 · 장소 사이드바 · 경로 상자 · 정렬 · 필터 · 다중 선택 · 저장 덮어쓰기 2단 확인).
//! 출처: nexa-sql `crates/nexa-sql/src/file_win.rs`(라벨·필터 구성).
//!
//! 이 모듈은 **앱 문자열 → 라벨**([`picker_labels`])과 **용도별 필터**([`PickFilter`])만 맡는다. `FilePicker`는 그 자체로
//! `Widget`이라 호스트(app.rs `Role::Picker`)가 다른 위젯처럼 `set_bounds`/`on_event`/`paint`를 부르고, 결과는
//! [`FilePicker::take_action`] 1회성으로 받는다. 매 프레임 `tick(now_ms)` · 애니메이션 중이면 빠른 깨우기(`animating`).

use nbeep_core::{t, Msg};
pub use nexa_dlg::{FileFilter, FilePicker, PickerAction, PickerLabels, PickerMode};

/// 앱 문자열 → 선택기 라벨(i18n — 리터럴은 여기 없다 · P3 블록).
#[must_use]
pub fn picker_labels() -> PickerLabels {
    PickerLabels {
        file_name: t(Msg::PkFileName).into(),
        file_type: t(Msg::PkFileType).into(),
        ok_open: t(Msg::PkOkOpen).into(),
        ok_save: t(Msg::PkOkSave).into(),
        ok_folder: t(Msg::PkOkFolder).into(),
        folder_name: t(Msg::PkFolderName).into(),
        cancel: t(Msg::PkCancel).into(),
        new_folder: t(Msg::PkNewFolder).into(),
        new_folder_name: t(Msg::PkNewFolderName).into(),
        show_hidden: t(Msg::PkShowHidden).into(),
        show_dot: t(Msg::PkShowDot).into(),
        col_name: t(Msg::PkColName).into(),
        col_modified: t(Msg::PkColModified).into(),
        col_size: t(Msg::PkColSize).into(),
        col_kind: t(Msg::PkColKind).into(),
        kind_folder: t(Msg::PkKindFolder).into(),
        kind_file: t(Msg::PkKindFile).into(),
        place_home: t(Msg::PkPlaceHome).into(),
        place_desktop: t(Msg::PkPlaceDesktop).into(),
        place_documents: t(Msg::PkPlaceDocuments).into(),
        place_downloads: t(Msg::PkPlaceDownloads).into(),
        place_drives: t(Msg::PkPlaceDrives).into(),
        kind_drive: t(Msg::PkKindDrive).into(),
        place_recent: t(Msg::PkPlaceRecent).into(),
        path_hint: t(Msg::PkPathHint).into(),
        err_not_found: t(Msg::PkErrNotFound).into(),
        err_exists: t(Msg::PkErrExists).into(),
        overwrite_ask: t(Msg::PkOverwriteAsk).into(),
        overwrite_yes: t(Msg::PkOverwriteYes).into(),
        err_bad_name: t(Msg::PkErrBadName).into(),
        err_list: t(Msg::PkErrList).into(),
        err_mkdir: t(Msg::PkErrMkdir).into(),
        menu_open: t(Msg::PkMenuOpen).into(),
        menu_copy_path: t(Msg::PkMenuCopyPath).into(),
        menu_copy_name: t(Msg::PkMenuCopyName).into(),
        menu_refresh: t(Msg::PkMenuRefresh).into(),
        multi_selected: t(Msg::PkMultiSelected).into(),
    }
}

/// 용도별 확장자 필터(첫 줄 = 기본 · 끝 줄 = 모든 파일 — 다른 이름으로 저장된 파일도 고를 수 있게).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickFilter {
    /// 모든 파일만(갤러리 실증).
    All,
    /// 폴더 고르기(목록에 폴더만 — 필터 뜻 없음 → "폴더" 한 줄).
    Folders,
    /// 신원 키(`.key`).
    IdentityKey,
    /// 설정 백업(`.cfg`).
    Settings,
    /// 이미지(프로필 사진 — imgdec가 읽는 형식).
    Image,
    /// 대화 기록 세그먼트(`.seg`).
    History,
    /// 라이선스(`.license`).
    License,
}

/// 프로필 사진으로 받는 확장자(종전 자체 피커의 목록 그대로).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"];

impl PickFilter {
    /// 필터 목록.
    #[must_use]
    pub fn filters(self) -> Vec<FileFilter> {
        let all = || FileFilter::new(t(Msg::PkFilterAll), &[]);
        let one = |m: Msg, exts: &[&str]| vec![FileFilter::new(t(m), exts), all()];
        match self {
            PickFilter::All => vec![all()],
            PickFilter::Folders => vec![FileFilter::new(t(Msg::PkFilterFolders), &[])],
            PickFilter::IdentityKey => one(Msg::PkFilterKey, &["key"]),
            PickFilter::Settings => one(Msg::PkFilterSettings, &["cfg"]),
            PickFilter::Image => one(Msg::PkFilterImage, IMAGE_EXTS),
            PickFilter::History => one(Msg::PkFilterHistory, &["seg"]),
            PickFilter::License => one(Msg::PkFilterLicense, &["license"]),
        }
    }
}

/// 선택기를 만든다 — 모드 · 시작 폴더 · 필터 · 저장 기본 이름 · 다중 선택(열기만).
/// 덮어쓰기는 두 번 눌러야 저장(무장 5초 — nexa-sql 기본과 같다).
#[must_use]
pub fn new_picker(
    mode: PickerMode,
    start: Option<&std::path::Path>,
    filter: PickFilter,
    default_name: &str,
    multi: bool,
) -> FilePicker {
    let filters = if mode == PickerMode::Folder {
        PickFilter::Folders.filters()
    } else {
        filter.filters()
    };
    let mut p = FilePicker::new(mode, start, filters, picker_labels());
    p.set_multi(multi && mode == PickerMode::Open);
    p.set_overwrite_confirm_ms(5000);
    if !default_name.is_empty() {
        p.set_default_name(default_name);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_all_filled_in_every_language() {
        for lang in nbeep_core::Lang::ALL {
            nbeep_core::set_lang(lang);
            let l = picker_labels();
            for (name, v) in [
                ("file_name", &l.file_name),
                ("ok_open", &l.ok_open),
                ("ok_save", &l.ok_save),
                ("ok_folder", &l.ok_folder),
                ("cancel", &l.cancel),
                ("col_name", &l.col_name),
                ("place_home", &l.place_home),
                ("overwrite_ask", &l.overwrite_ask),
                ("multi_selected", &l.multi_selected),
                ("menu_refresh", &l.menu_refresh),
            ] {
                assert!(!v.is_empty(), "{lang:?}/{name}");
            }
            assert!(l.overwrite_ask.contains("{0}"), "{lang:?} 파일 이름 자리");
            assert!(
                l.multi_selected.contains("{0}") && l.multi_selected.contains("{1}"),
                "{lang:?} 수·크기 자리"
            );
        }
        nbeep_core::set_lang(nbeep_core::Lang::En);
    }

    #[test]
    fn filters_default_first_then_all() {
        let f = PickFilter::License.filters();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].exts, ["license"]);
        assert!(f[1].exts.is_empty(), "끝 = 모든 파일");
        assert_eq!(PickFilter::Image.filters()[0].exts.len(), IMAGE_EXTS.len());
        assert_eq!(PickFilter::History.filters()[0].exts, ["seg"]);
        assert_eq!(PickFilter::Settings.filters()[0].exts, ["cfg"]);
        assert_eq!(PickFilter::IdentityKey.filters()[0].exts, ["key"]);
        assert_eq!(PickFilter::All.filters().len(), 1);
        assert_eq!(PickFilter::Folders.filters().len(), 1);
    }

    #[test]
    fn new_picker_starts_in_given_dir_and_yields_no_action() {
        let dir = std::env::temp_dir();
        let mut p = new_picker(
            PickerMode::Save,
            Some(&dir),
            PickFilter::Settings,
            "x.cfg",
            true,
        );
        assert_eq!(p.current_dir(), dir.as_path());
        assert_eq!(p.take_action(), PickerAction::None);
    }
}
