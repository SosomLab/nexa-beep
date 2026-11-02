//! ★ 설정 내보내기/가져오기/JSON 편집용 **평면 JSON 객체** 코덱(11-02 · nexa-dir3 Export/Import/Edit JSON 이식 ·
//! 외부 crate 0 — DR-12 의존 최소). 다루는 꼴은 `{"key": "value", …}` 하나뿐이다: 값은 문자열(숫자·불리언은 그
//! 글자 그대로 받는다 · 중첩 객체/배열은 오류). 키 순서 = 입력 순서(인코더는 호출자가 준 순서).

/// `(키, 값)` 목록 → 보기 좋게 들여쓴 JSON 객체(끝 개행 포함 · 비어 있으면 `{}`).
#[must_use]
pub fn encode(pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return "{}\n".to_string();
    }
    let mut out = String::from("{\n");
    for (i, (k, v)) in pairs.iter().enumerate() {
        out.push_str("  ");
        push_str_lit(&mut out, k);
        out.push_str(": ");
        push_str_lit(&mut out, v);
        if i + 1 < pairs.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn push_str_lit(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// JSON 객체 → `(키, 값)` 목록. 문자열·숫자·불리언·null 값만(숫자·불리언·null은 원문 글자 그대로) ·
/// 중첩·배열·문법 오류 = `Err(이유)`. 같은 키가 겹치면 뒤의 것이 이긴다(호출자가 정리).
///
/// # Errors
/// 객체가 아니거나 문법이 틀리면 사람이 읽을 이유 문자열.
pub fn decode(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.ws();
    if !p.eat(b'{') {
        return Err("객체가 아닙니다(`{`로 시작해야 합니다)".into());
    }
    let mut out = Vec::new();
    p.ws();
    if p.eat(b'}') {
        p.ws();
        return p.end(out);
    }
    loop {
        p.ws();
        let key = p.string().map_err(|e| format!("키: {e}"))?;
        p.ws();
        if !p.eat(b':') {
            return Err(format!("`{key}` 뒤에 `:`가 없습니다"));
        }
        p.ws();
        let val = p.scalar().map_err(|e| format!("`{key}` 값: {e}"))?;
        out.push((key, val));
        p.ws();
        if p.eat(b',') {
            continue;
        }
        if p.eat(b'}') {
            p.ws();
            return p.end(out);
        }
        return Err(format!("{}번째 글자: `,` 또는 `}}`가 와야 합니다", p.i + 1));
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn eat(&mut self, b: u8) -> bool {
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn end(&self, out: Vec<(String, String)>) -> Result<Vec<(String, String)>, String> {
        if self.i == self.s.len() {
            Ok(out)
        } else {
            Err(format!("{}번째 글자 뒤에 군더더기가 있습니다", self.i + 1))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if !self.eat(b'"') {
            return Err("문자열(`\"`)이 와야 합니다".into());
        }
        let mut out = Vec::new();
        loop {
            let Some(&b) = self.s.get(self.i) else {
                return Err("문자열이 닫히지 않았습니다".into());
            };
            self.i += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return Err("이스케이프가 끝나지 않았습니다".into());
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'u' => {
                            let hex = self
                                .s
                                .get(self.i..self.i + 4)
                                .and_then(|h| std::str::from_utf8(h).ok())
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .ok_or_else(|| "\\u 뒤 16진수 4자리가 없습니다".to_string())?;
                            self.i += 4;
                            let ch = char::from_u32(hex).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => return Err(format!("모르는 이스케이프 `\\{}`", other as char)),
                    }
                }
                b => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| "UTF-8이 아닙니다".to_string())
    }
    fn scalar(&mut self) -> Result<String, String> {
        match self.s.get(self.i) {
            Some(b'"') => self.string(),
            Some(b'{') | Some(b'[') => {
                Err("중첩 객체·배열은 지원하지 않습니다(값은 문자열)".into())
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len()
                    && !matches!(self.s[self.i], b',' | b'}' | b' ' | b'\t' | b'\n' | b'\r')
                {
                    self.i += 1;
                }
                let lit =
                    std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "UTF-8이 아닙니다")?;
                if lit.is_empty() {
                    return Err("값이 비었습니다".into());
                }
                Ok(lit.to_string())
            }
            None => Err("값이 없습니다".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_escapes_and_order() {
        let pairs = [
            ("ui.theme", "dark"),
            ("profile.display_name", "홍길동 \"짱\"\n둘째 줄\\"),
            ("empty", ""),
        ];
        let text = encode(&pairs);
        assert!(text.starts_with("{\n  \"ui.theme\": \"dark\",\n"));
        let back = decode(&text).unwrap();
        let back: Vec<(&str, &str)> = back.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(back, pairs);
        assert_eq!(encode(&[]), "{}\n");
        assert_eq!(decode("{}").unwrap(), vec![]);
    }

    #[test]
    fn accepts_bare_literals_and_rejects_nesting() {
        let v = decode(" { \"a\" : 12 , \"b\":true, \"c\": null }\n").unwrap();
        assert_eq!(
            v,
            vec![
                ("a".to_string(), "12".to_string()),
                ("b".to_string(), "true".to_string()),
                ("c".to_string(), "null".to_string())
            ]
        );
        assert!(decode("{\"a\": {\"b\": 1}}").is_err());
        assert!(decode("{\"a\": [1]}").is_err());
        assert!(decode("[1,2]").is_err());
        assert!(decode("{\"a\": \"x\"} junk").is_err());
        assert!(decode("{\"a\" \"x\"}").is_err());
        assert_eq!(decode("{\"u\": \"\\u00e9\\ud55c\"}").unwrap()[0].1, "é한");
    }
}
