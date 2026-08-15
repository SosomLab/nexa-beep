//! 대화창 명령(`/…`) — **입력이 메시지인지 명령인지 가르는 단일 지점**(사용자 요청 08-15).
//!
//! CLI 대화 모드에는 이미 `/quit`·`/help`·`/send`가 있었지만 **GUI에는 없었다**. 같은
//! 문법을 GUI에도 주되, 판정을 여기 한 곳에 모은다 — 부르는 쪽(1:1·그룹·CLI)이 각자
//! 문자열을 뜯으면 **"어디서는 명령이고 어디서는 메시지"** 가 된다.
//!
//! ## 규칙 셋
//!
//! 1. **원본의 첫 글자가 `/`이고, 한 줄이고, 아는 이름이면 명령** — 실행하고
//!    **상대에게 보내지 않는다**. ★ "첫 글자"는 **trim 전 원본 기준**이고(앞에 공백이
//!    있으면 메시지다), **줄바꿈이 있으면 메시지다**(명령은 한 줄 — 그러지 않으면
//!    멀티라인 입력의 뒷줄이 조용히 사라진다).
//! 2. **`//`로 시작하면 escape** — 앞의 `/` 하나를 벗기고 **메시지로 보낸다**.
//!    (이게 없으면 `/`로 시작하는 문장을 영영 못 보낸다.)
//! 3. **모르는 `/이름`은 메시지가 아니라 오류** — 오타(`/verifed`)를 상대에게 그대로
//!    쏘면 사용자는 명령이 실행된 줄 안다. **조용히 다른 일을 하지 않는다**(fail-closed).
//!
//! ## 명령은 로컬이다
//!
//! 여기서 나오는 어떤 것도 **와이어로 나가지 않는다**. 명령은 내 앱에 하는 지시이고,
//! 상대에게 뭔가 보내야 한다면 그건 명령의 *결과*로 호스트가 따로 정하는 일이다.

/// 대화창에서 실행할 수 있는 명령.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    /// 사용 가능한 명령 안내.
    Help,
    /// **지문(SAS) 대조** — 안전 번호 카드를 연다.
    ///
    /// ⚠️ **이 명령이 대조를 완료시키지 않는다.** 카드를 열어 줄 뿐이고, 승격은 사람이
    /// **다른 채널로 숫자를 맞춘 뒤** 카드의 버튼을 눌러야 일어난다 — 인증하려는 그
    /// 통로 안에서 "확인했어?"를 주고받아 승격하면 **중간자가 그 문답까지 대신**할 수
    /// 있어 SAS가 통째로 무의미해진다([docs/26 §9] "교차 대조는 MITM 방어가 아니다").
    Verify,
    /// 이 대화 상대의 신뢰 상태를 한 줄로.
    Trust,
    /// 대화창 닫기.
    Close,
}

impl ChatCommand {
    /// 표준 이름(안내·오류 문구에 쓰는 대표 철자).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Help => "/help",
            Self::Verify => "/verify",
            Self::Trust => "/trust",
            Self::Close => "/close",
        }
    }
}

/// 입력 한 줄의 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// 그대로 보낼 메시지(escape를 이미 벗긴 상태).
    Text(String),
    /// 실행할 명령(상대에게 보내지 않는다).
    Command(ChatCommand),
    /// `/`로 시작하지만 모르는 이름 — **보내지도 실행하지도 않는다**.
    Unknown(String),
    /// 보낼 것이 없다(공백뿐).
    Empty,
}

/// 각 명령의 인식 철자 — 첫 번째가 대표. 오타로 흔한 변형(`/verified`)도 받는다.
const TABLE: &[(&[&str], ChatCommand)] = &[
    (&["help", "?", "명령"], ChatCommand::Help),
    (
        &["verify", "verified", "sas", "지문", "대조"],
        ChatCommand::Verify,
    ),
    (&["trust", "신뢰"], ChatCommand::Trust),
    (&["close", "quit", "exit", "q", "닫기"], ChatCommand::Close),
];

/// 입력 한 줄을 해석한다.
///
/// 앞뒤 공백은 떨어내고 본다. 명령 이름은 **대소문자 무시**.
#[must_use]
pub fn parse(input: &str) -> Parsed {
    if input.trim().is_empty() {
        return Parsed::Empty;
    }
    // ★ ①의 "첫 글자"는 **원본의 첫 글자**다(사용자 확정 08-16 — trim 전).
    //   trim을 먼저 걸면 `" /help"`(앞 공백)도 명령이 된다. 공백으로 시작하는 입력은
    //   사람이 명령을 치려던 것이 아니라 **문장**일 가능성이 크고, 무엇보다 규칙이
    //   "첫 글자"라면 판정도 첫 글자에서 끝나야 읽는 사람이 예측할 수 있다.
    if !input.starts_with('/') {
        return Parsed::Text(input.trim().to_string());
    }
    // ② escape — `//…`는 `/`로 시작하는 **메시지**다(앞 `/` 하나만 벗긴다).
    if let Some(rest) = input.strip_prefix("//") {
        return Parsed::Text(format!("/{}", rest.trim_end()));
    }
    // ★ **명령은 한 줄이다.** 줄바꿈이 있으면 메시지로 본다 — 그러지 않으면
    //   `"/help⏎둘째 줄"`(Shift+Enter 멀티라인)에서 첫 줄만 명령으로 먹고
    //   **나머지가 조용히 사라진다**(08-16 실측으로 잡은 데이터 손실).
    if input.contains('\n') {
        return Parsed::Text(input.trim().to_string());
    }
    let body = &input[1..];
    // 인자는 아직 쓰지 않지만 `/verify 어쩌고`도 명령으로 받는다(뒤는 무시).
    let word = body.split_whitespace().next().unwrap_or("").to_lowercase();
    if word.is_empty() {
        return Parsed::Text(input.trim().to_string()); // `/` 한 글자·`/   `는 메시지
    }
    for (names, cmd) in TABLE {
        if names.contains(&word.as_str()) {
            return Parsed::Command(cmd.clone());
        }
    }
    Parsed::Unknown(word)
}

/// 명령 안내 문구(`/help` 출력 · 화면·CLI 공용).
#[must_use]
pub fn help_text() -> String {
    [
        "사용 가능한 명령",
        "  /help            이 안내",
        "  /verify          지문(안전 번호) 대조 카드 열기",
        "  /trust           이 상대의 신뢰 상태 보기",
        "  /close           대화창 닫기",
        "  //로 시작         '/'로 시작하는 메시지를 보낼 때(예: //안녕 → /안녕)",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_not_a_command() {
        assert_eq!(parse("안녕하세요"), Parsed::Text("안녕하세요".into()));
        assert_eq!(parse("  hi  "), Parsed::Text("hi".into()), "앞뒤 공백 제거");
        assert_eq!(parse("   "), Parsed::Empty);
    }

    #[test]
    fn known_commands_parse() {
        assert_eq!(parse("/help"), Parsed::Command(ChatCommand::Help));
        assert_eq!(parse("/verify"), Parsed::Command(ChatCommand::Verify));
        assert_eq!(parse("/close"), Parsed::Command(ChatCommand::Close));
        assert_eq!(parse("/trust"), Parsed::Command(ChatCommand::Trust));
    }

    /// 사용자가 요청한 철자(`/verified`)와 한글 별칭도 받는다.
    #[test]
    fn aliases_and_case_are_accepted() {
        for s in ["/verified", "/VERIFY", "/Sas", "/지문", "/대조"] {
            assert_eq!(parse(s), Parsed::Command(ChatCommand::Verify), "{s}");
        }
        assert_eq!(parse("/Q"), Parsed::Command(ChatCommand::Close));
    }

    /// ★ escape가 없으면 `/`로 시작하는 문장을 영영 못 보낸다.
    #[test]
    fn double_slash_escapes_to_message() {
        assert_eq!(parse("//verify"), Parsed::Text("/verify".into()));
        assert_eq!(parse("//안녕"), Parsed::Text("/안녕".into()));
        assert_eq!(
            parse("/"),
            Parsed::Text("/".into()),
            "슬래시 한 글자는 메시지"
        );
    }

    /// ★ 오타를 **상대에게 보내지 않는다** — 보내면 사용자는 명령이 실행된 줄 안다.
    #[test]
    fn unknown_command_is_neither_sent_nor_run() {
        assert_eq!(parse("/verifed"), Parsed::Unknown("verifed".into()));
        assert_eq!(parse("/zzz 인자"), Parsed::Unknown("zzz".into()));
        // 메시지로 새지 않는다.
        assert!(!matches!(parse("/verifed"), Parsed::Text(_)));
    }

    /// ★ **첫 글자가 `/`일 때만 명령**(사용자 확정 08-16) — trim 전 원본 기준.
    /// 그전에는 `trim()`을 먼저 걸어 `" /help"`도 명령이 됐다.
    #[test]
    fn only_a_literal_leading_slash_is_a_command() {
        for s in [" /help", "\t/help", "\u{a0}/help", " /verify "] {
            assert_eq!(
                parse(s),
                Parsed::Text(s.trim().to_string()),
                "앞에 공백이 있으면 메시지: {s:?}"
            );
        }
        // 진짜 첫 글자면 명령(뒤 공백은 무관).
        assert_eq!(parse("/help  "), Parsed::Command(ChatCommand::Help));
    }

    /// ★ **명령은 한 줄** — 멀티라인(Shift+Enter)의 뒷줄이 사라지면 안 된다.
    /// 08-16 실측으로 잡은 조용한 데이터 손실.
    #[test]
    fn multiline_starting_with_slash_is_a_message() {
        let m = "/help\n두 번째 줄";
        assert_eq!(
            parse(m),
            Parsed::Text(m.to_string()),
            "뒷줄이 보존돼야 한다"
        );
        let e = "//안녕\n둘째";
        assert_eq!(
            parse(e),
            Parsed::Text("/안녕\n둘째".into()),
            "escape + 멀티라인"
        );
        // 한 줄이면 그대로 명령.
        assert_eq!(parse("/help"), Parsed::Command(ChatCommand::Help));
    }

    /// 중간에 나온 `/`는 명령이 아니다(첫 글자 규칙의 반대편).
    #[test]
    fn slash_in_the_middle_is_plain_text() {
        assert_eq!(parse("안녕 /help"), Parsed::Text("안녕 /help".into()));
        assert_eq!(parse("a/help"), Parsed::Text("a/help".into()));
        assert_eq!(
            parse("경로는 /usr/bin"),
            Parsed::Text("경로는 /usr/bin".into())
        );
    }

    /// 인자가 붙어도 명령으로 받는다(뒤는 지금 무시 — 확장 자리).
    #[test]
    fn trailing_args_still_dispatch() {
        assert_eq!(parse("/verify 지금"), Parsed::Command(ChatCommand::Verify));
    }

    /// 안내에 모든 명령의 대표 철자가 들어 있다(문구와 구현이 갈리면 그게 거짓말이다).
    #[test]
    fn help_lists_every_command() {
        let h = help_text();
        for (_, cmd) in TABLE {
            assert!(h.contains(cmd.name()), "{} 누락", cmd.name());
        }
    }
}
