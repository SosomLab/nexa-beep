//! 호스트명 취득 — 표시 이름 기본값의 **원료**(M1-10 · FR-S-50).
//!
//! ⚠️ 여기서 얻은 문자열은 실명을 품을 수 있다("{실명}의 MacBook" 등 — R-19).
//! **그대로 쓰지 말 것** — 반드시 `nbeep_core::default_display_name`(정제·폴백)을
//! 거친다. 이 모듈은 원시 문자열만 돌려주고 판단하지 않는다(플랫폼 경계).

/// OS 호스트명(원시). 취득 실패 시 `None` — 호출자는 지문 라벨로 폴백한다.
#[must_use]
pub fn hostname() -> Option<String> {
    imp::hostname().filter(|s| !s.trim().is_empty())
}

#[cfg(windows)]
mod imp {
    /// Windows — `COMPUTERNAME` 환경 변수(모든 프로세스에 주입되는 NetBIOS 이름).
    /// 별도 Win32 호출 없이 충분하다(기본값 "DESKTOP-XXXXXX" 형태).
    pub(super) fn hostname() -> Option<String> {
        std::env::var("COMPUTERNAME").ok()
    }
}

#[cfg(unix)]
mod imp {
    /// Unix — `gethostname(2)`. 실패 시 `HOSTNAME` 환경 변수 폴백.
    pub(super) fn hostname() -> Option<String> {
        let mut buf = [0u8; 256];
        // SAFETY: buf는 유효한 가변 버퍼이고 길이를 함께 넘긴다. gethostname은
        // 최대 len-1 바이트를 쓰고 0을 돌려준다(실패 시 -1).
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            if let Ok(s) = std::str::from_utf8(&buf[..end]) {
                return Some(s.to_string());
            }
        }
        std::env::var("HOSTNAME").ok()
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub(super) fn hostname() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    /// 취득 성공 시 공백만인 문자열은 나오지 않는다(필터 계약).
    #[test]
    fn hostname_is_nonblank_when_present() {
        if let Some(h) = super::hostname() {
            assert!(!h.trim().is_empty());
        }
    }
}
