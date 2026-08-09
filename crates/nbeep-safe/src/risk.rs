//! **위험 등급 판정** — 확장자·매직바이트를 모두 보고 **상위 등급을 택한다**(fail-closed).
//!
//! [docs/11 §3] ADR-0004. 판정은 **플랫폼 무관 합집합**이다 — 내 OS에서 안 도는 실행형도
//! 전달(포워드)되면 다른 OS에서 돈다. `.lnk`·`.url`·`.scf`·`.desktop`처럼 **매직이 없는**
//! 형식은 확장자 판정이 유일한 방어선이고, 반대로 이름을 속인 실행 파일은 매직이 잡는다.
//! 둘이 어긋나면 [`Verdict::mismatch`]로 보고한다(승인 화면 경고 — "PDF라더니 실행 파일").
//!
//! `svg`는 이미지가 아니라 **능동 문서** 취급(스크립트·외부 참조 — FR-S-12 미지원).

use nbeep_core::RiskLevel;

/// 매직바이트로 판정한 형식(승인 화면 표시·불일치 경고 근거).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DetectedKind {
    /// Windows PE(`MZ`).
    Pe,
    /// ELF.
    Elf,
    /// Mach-O(32/64·FAT).
    MachO,
    /// 스크립트(셔뱅 `#!`).
    Script,
    /// PDF.
    Pdf,
    /// OLE 복합 문서(구형 오피스 — 매크로 가능).
    Ole,
    /// ZIP 계열(`PK` — zip·jar·docx·xlsx 등 컨테이너).
    Zip,
    /// 7-Zip.
    SevenZ,
    /// RAR.
    Rar,
    /// gzip.
    Gzip,
    /// bzip2.
    Bzip2,
    /// xz.
    Xz,
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF.
    Gif,
    /// SVG(문서 취급).
    Svg,
    /// ISO 9660 디스크 이미지(MotW 우회 경로).
    Iso,
    /// 판정 불가/미지.
    Unknown,
}

impl DetectedKind {
    /// 표시용 이름(메타 `detected_kind` 문자열).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pe => "pe",
            Self::Elf => "elf",
            Self::MachO => "mach-o",
            Self::Script => "script",
            Self::Pdf => "pdf",
            Self::Ole => "ole",
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::Rar => "rar",
            Self::Gzip => "gzip",
            Self::Bzip2 => "bzip2",
            Self::Xz => "xz",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Svg => "svg",
            Self::Iso => "iso",
            Self::Unknown => "unknown",
        }
    }

    /// 이 형식 자체의 위험 등급.
    fn risk(self) -> RiskLevel {
        match self {
            Self::Pe | Self::Elf | Self::MachO | Self::Script => RiskLevel::Executable,
            // OLE(매크로)·PDF·SVG·ISO = 능동 콘텐츠 문서.
            Self::Pdf | Self::Ole | Self::Svg | Self::Iso => RiskLevel::ActiveDocument,
            Self::Zip | Self::SevenZ | Self::Rar | Self::Gzip | Self::Bzip2 | Self::Xz => {
                RiskLevel::Archive
            }
            Self::Png | Self::Jpeg | Self::Gif | Self::Unknown => RiskLevel::Data,
        }
    }
}

/// 판정 결과 — 최종 등급(fail-closed 최상위) + 근거.
#[derive(Clone, Copy, Debug)]
pub struct Verdict {
    /// 최종 등급 = max(확장자, 매직).
    pub risk: RiskLevel,
    /// 확장자 판정.
    pub by_ext: RiskLevel,
    /// 매직 판정.
    pub by_magic: RiskLevel,
    /// 매직으로 본 형식.
    pub detected: DetectedKind,
    /// 확장자 주장과 매직 실체의 **등급 불일치**(승인 화면 경고 근거).
    pub mismatch: bool,
}

/// 🔴 실행형 — 전 플랫폼 합집합([docs/11 §3] · fail-closed).
const EXEC_EXT: &[&str] = &[
    // Windows
    "exe",
    "com",
    "scr",
    "pif",
    "bat",
    "cmd",
    "msi",
    "msp",
    "msc",
    "cpl",
    "lnk",
    "hta",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "ps1",
    "psm1",
    "ps1xml",
    "psc1",
    "reg",
    "inf",
    "sct",
    "scf",
    "url",
    "library-ms",
    "search-ms",
    "appx",
    "appxbundle",
    "msix",
    // macOS
    "app",
    "pkg",
    "mpkg",
    "dmg",
    "command",
    "workflow",
    "terminal",
    // Linux
    "sh",
    "bash",
    "run",
    "deb",
    "rpm",
    "appimage",
    "snap",
    "flatpakref",
    "desktop",
    // 공통
    "jar",
    "class",
    "py",
    "rb",
    "pl",
    "php",
];

/// 🟠 능동 콘텐츠 문서(`svg` 포함 — 문서 취급 · `iso` 계열 = MotW 우회 경로).
const ACTIVE_EXT: &[&str] = &[
    "docm", "xlsm", "pptm", "dotm", "xltm", "potm", "xlam", "ppam", "pdf", "rtf", "chm", "iso",
    "img", "vhd", "vhdx", "svg",
];

/// 🟡 아카이브 — 자동 해제 금지.
const ARCHIVE_EXT: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "bz2", "xz", "tgz", "cab", "lzh", "ace", "arj",
];

/// fail-closed 비교용 심각도(클수록 위험).
fn severity(r: RiskLevel) -> u8 {
    match r {
        RiskLevel::Executable => 3,
        RiskLevel::ActiveDocument => 2,
        RiskLevel::Archive => 1,
        RiskLevel::Data => 0,
    }
}

/// 파일명에서 확장자(소문자) — 마지막 점 뒤. 점이 없으면 빈 문자열.
#[must_use]
pub fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 확장자 → 등급.
#[must_use]
pub fn classify_ext(ext: &str) -> RiskLevel {
    let e = ext.to_ascii_lowercase();
    if EXEC_EXT.contains(&e.as_str()) {
        RiskLevel::Executable
    } else if ACTIVE_EXT.contains(&e.as_str()) {
        RiskLevel::ActiveDocument
    } else if ARCHIVE_EXT.contains(&e.as_str()) {
        RiskLevel::Archive
    } else {
        RiskLevel::Data
    }
}

/// 매직바이트(파일 선두) → 형식. 판정 불가 = [`DetectedKind::Unknown`].
#[must_use]
pub fn detect_magic(head: &[u8]) -> DetectedKind {
    // ISO 9660: 오프셋 0x8001에 "CD001"(볼륨 서술자) — 선두가 아니라 별도 확인.
    if head.len() > 0x8005 && &head[0x8001..0x8006] == b"CD001" {
        return DetectedKind::Iso;
    }
    match head {
        [0x4D, 0x5A, ..] => DetectedKind::Pe,
        [0x7F, b'E', b'L', b'F', ..] => DetectedKind::Elf,
        // Mach-O: 32/64 · 양엔디언 · FAT.
        [0xFE, 0xED, 0xFA, 0xCE, ..]
        | [0xFE, 0xED, 0xFA, 0xCF, ..]
        | [0xCE, 0xFA, 0xED, 0xFE, ..]
        | [0xCF, 0xFA, 0xED, 0xFE, ..]
        | [0xCA, 0xFE, 0xBA, 0xBE, ..] => DetectedKind::MachO,
        [b'#', b'!', ..] => DetectedKind::Script,
        [b'%', b'P', b'D', b'F', ..] => DetectedKind::Pdf,
        [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, ..] => DetectedKind::Ole,
        [b'P', b'K', 0x03 | 0x05 | 0x07, ..] => DetectedKind::Zip,
        [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, ..] => DetectedKind::SevenZ,
        [b'R', b'a', b'r', b'!', 0x1A, 0x07, ..] => DetectedKind::Rar,
        [0x1F, 0x8B, ..] => DetectedKind::Gzip,
        [b'B', b'Z', b'h', ..] => DetectedKind::Bzip2,
        [0xFD, b'7', b'z', b'X', b'Z', 0x00, ..] => DetectedKind::Xz,
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => DetectedKind::Png,
        [0xFF, 0xD8, 0xFF, ..] => DetectedKind::Jpeg,
        [b'G', b'I', b'F', b'8', ..] => DetectedKind::Gif,
        _ => {
            // SVG: 텍스트 선두(공백/BOM/XML 선언 허용)에 "<svg" — 선두 512B만 본다.
            let n = head.len().min(512);
            if let Ok(txt) = core::str::from_utf8(&head[..n]) {
                if txt.contains("<svg") {
                    return DetectedKind::Svg;
                }
            }
            DetectedKind::Unknown
        }
    }
}

/// **최종 판정** — 확장자·매직 중 상위 등급(fail-closed) + 불일치 경고.
///
/// `head`는 파일 선두 바이트(ISO 판정까지 하려면 32KiB+5, 아니면 있는 만큼).
#[must_use]
pub fn classify(name: &str, head: &[u8]) -> Verdict {
    let ext = extension_of(name);
    let by_ext = classify_ext(&ext);
    let detected = detect_magic(head);
    let by_magic = detected.risk();
    let risk = if severity(by_magic) > severity(by_ext) {
        by_magic
    } else {
        by_ext
    };
    // 불일치 = 매직이 실체를 알아봤는데(Unknown 아님) 등급이 서로 다를 때.
    // (Unknown은 "매직 없는 형식"이 흔해서 — .lnk·.txt 등 — 경고가 소음이 된다.)
    let mismatch = detected != DetectedKind::Unknown && by_magic != by_ext;
    Verdict {
        risk,
        by_ext,
        by_magic,
        detected,
        mismatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_only_defenses_lnk_url_desktop() {
        // 매직이 없는 형식 — 확장자 판정이 유일한 방어선([docs/11 §3]).
        for name in ["autorun.lnk", "phish.URL", "x.scf", "run.desktop"] {
            let v = classify(name, b"whatever");
            assert_eq!(v.risk, RiskLevel::Executable, "{name}");
            assert!(!v.mismatch, "Unknown 매직은 경고 소음 아님: {name}");
        }
    }

    #[test]
    fn magic_catches_renamed_executable() {
        // "PDF라더니 실행 파일" — MZ 매직이 잡고 불일치 경고.
        let v = classify("report.pdf", b"MZ\x90\x00rest");
        assert_eq!(v.risk, RiskLevel::Executable);
        assert_eq!(v.by_ext, RiskLevel::ActiveDocument);
        assert_eq!(v.detected, DetectedKind::Pe);
        assert!(v.mismatch, "승인 화면 경고 근거");
    }

    #[test]
    fn levels_by_extension_union_of_platforms() {
        assert_eq!(
            classify_ext("sh"),
            RiskLevel::Executable,
            "리눅스 실행형도 상시"
        );
        assert_eq!(classify_ext("DMG"), RiskLevel::Executable, "대소문자 무시");
        assert_eq!(classify_ext("docm"), RiskLevel::ActiveDocument);
        assert_eq!(
            classify_ext("iso"),
            RiskLevel::ActiveDocument,
            "MotW 우회 경로"
        );
        assert_eq!(
            classify_ext("svg"),
            RiskLevel::ActiveDocument,
            "이미지 아님 — 문서"
        );
        assert_eq!(classify_ext("zip"), RiskLevel::Archive);
        assert_eq!(classify_ext("png"), RiskLevel::Data);
        assert_eq!(classify_ext(""), RiskLevel::Data, "확장자 없음");
    }

    #[test]
    fn magic_detection_table() {
        let cases: &[(&[u8], DetectedKind)] = &[
            (b"\x7FELF\x02\x01", DetectedKind::Elf),
            (b"\xFE\xED\xFA\xCF...", DetectedKind::MachO),
            (b"#!/bin/sh\n", DetectedKind::Script),
            (b"%PDF-1.7", DetectedKind::Pdf),
            (b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1..", DetectedKind::Ole),
            (b"PK\x03\x04....", DetectedKind::Zip),
            (b"7z\xBC\xAF\x27\x1C", DetectedKind::SevenZ),
            (b"Rar!\x1A\x07\x00", DetectedKind::Rar),
            (b"\x1F\x8B\x08", DetectedKind::Gzip),
            (b"BZh91AY", DetectedKind::Bzip2),
            (b"\xFD7zXZ\x00", DetectedKind::Xz),
            (b"\x89PNG\x0D\x0A\x1A\x0A", DetectedKind::Png),
            (b"\xFF\xD8\xFF\xE0", DetectedKind::Jpeg),
            (b"GIF89a", DetectedKind::Gif),
            (b"<?xml version=\"1.0\"?>\n<svg xmlns=", DetectedKind::Svg),
            (b"hello plain text", DetectedKind::Unknown),
        ];
        for (head, want) in cases {
            assert_eq!(detect_magic(head), *want, "{head:?}");
        }
    }

    #[test]
    fn iso_magic_at_offset() {
        let mut img = vec![0u8; 0x8010];
        img[0x8001..0x8006].copy_from_slice(b"CD001");
        assert_eq!(detect_magic(&img), DetectedKind::Iso);
        let v = classify("disk.bin", &img);
        assert_eq!(v.risk, RiskLevel::ActiveDocument, "bin(Data) < iso 매직");
        assert!(v.mismatch);
    }

    #[test]
    fn fail_closed_takes_higher_of_the_two() {
        // 확장자가 상위: exe 이름 + zip 실체 → Executable 유지(이름 방어선).
        let v = classify("tool.exe", b"PK\x03\x04");
        assert_eq!(v.risk, RiskLevel::Executable);
        assert!(v.mismatch);
        // 매직이 상위: txt 이름 + 셔뱅 → Executable.
        let v = classify("notes.txt", b"#!/usr/bin/env bash");
        assert_eq!(v.risk, RiskLevel::Executable);
        // 둘 다 같은 등급(zip+zip) = 불일치 아님.
        let v = classify("a.zip", b"PK\x03\x04");
        assert!(!v.mismatch);
        assert_eq!(v.risk, RiskLevel::Archive);
    }

    #[test]
    fn office_zip_container_is_archive_by_magic_but_macro_ext_wins() {
        // docm(매크로 문서)은 zip 컨테이너 — 확장자 등급(ActiveDocument)이 이긴다.
        let v = classify("invoice.docm", b"PK\x03\x04");
        assert_eq!(v.risk, RiskLevel::ActiveDocument);
        assert_eq!(v.detected, DetectedKind::Zip);
    }

    #[test]
    fn multi_dot_and_no_ext_names() {
        assert_eq!(extension_of("a.tar.gz"), "gz");
        assert_eq!(extension_of("README"), "");
        let v = classify("README", b"plain");
        assert_eq!(v.risk, RiskLevel::Data);
    }
}
