//! 규칙 텍스트 파서. 재귀 하강이고 중첩은 깊이 카운터로 제한한다(적대적 규칙 파일이 스택을 넘기지 못하게).

use scan_common::Severity;

use super::{Cmp, Cond, Pattern, Quant, Rule, RuleError, RuleSet, RuleString};

/// 규칙 원문의 최대 바이트(1MiB).
pub const MAX_SOURCE: usize = 1024 * 1024;
/// 규칙 수의 상한.
pub const MAX_RULES: usize = 1000;
/// 규칙당 문자열 수의 상한.
pub const MAX_STRINGS: usize = 256;
/// 문자열 하나의 최대 바이트.
pub const MAX_STRING_BYTES: usize = 4096;
/// 조건식 중첩(괄호, `not`)의 최대 깊이.
const MAX_DEPTH: usize = 32;
/// 지원하지 않는 YARA 문법의 키워드. 만나면 "unsupported syntax"다.
const UNSUPPORTED: &[&str] = &["import", "include", "private", "global", "for", "filesize", "entrypoint", "uint8", "uint16", "uint32", "int8", "int16", "int32", "matches", "contains", "defined", "in", "xor", "base64"];

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl<'a> Parser<'a> {
    fn line(&self) -> usize {
        1 + self.src[..self.pos.min(self.src.len())].iter().filter(|&&b| b == b'\n').count()
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, RuleError> {
        Err(RuleError { line: self.line(), message: message.into() })
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    /// 공백과 주석(`//`, `/* */`)을 건너뛴다. 닫히지 않은 블록 주석은 오류다.
    fn skip(&mut self) -> Result<(), RuleError> {
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() => self.pos += 1,
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'/') => {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            None => {
                                self.pos = start;
                                return self.fail("unterminated block comment");
                            }
                            Some(b'*') if self.src.get(self.pos + 1) == Some(&b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(_) => self.pos += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// 식별자를 읽는다(위치는 식별자 뒤). 식별자가 아니면 `None`이고 위치는 그대로다.
    fn ident(&mut self) -> Option<String> {
        let start = self.pos;
        if !self.peek().is_some_and(is_ident_start) {
            return None;
        }
        while self.peek().is_some_and(is_ident_byte) {
            self.pos += 1;
        }
        Some(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    /// 다음 식별자를 읽지 않고 본다.
    fn peek_ident(&mut self) -> Option<String> {
        let at = self.pos;
        let id = self.ident();
        self.pos = at;
        id
    }

    fn expect(&mut self, c: u8) -> Result<(), RuleError> {
        self.skip()?;
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            self.fail(format!("expected '{}'", c as char))
        }
    }

    fn unsupported<T>(&self, what: &str) -> Result<T, RuleError> {
        self.fail(format!("unsupported syntax: {what}"))
    }

    /// 따옴표 문자열 하나(이스케이프 `\\ \" \n \r \t \xNN`)를 바이트로 읽는다.
    fn string_literal(&mut self) -> Result<Vec<u8>, RuleError> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => return self.fail("unterminated string"),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'\\') => out.push(b'\\'),
                        Some(b'"') => out.push(b'"'),
                        Some(b'n') => out.push(b'\n'),
                        Some(b'r') => out.push(b'\r'),
                        Some(b't') => out.push(b'\t'),
                        Some(b'x') => {
                            let hex = self.src.get(self.pos + 1..self.pos + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                            let Some(byte) = hex else { return self.fail("invalid \\x escape") };
                            out.push(byte);
                            self.pos += 2;
                        }
                        _ => return self.fail("unknown escape sequence"),
                    }
                    self.pos += 1;
                }
                Some(b) => {
                    out.push(b);
                    self.pos += 1;
                }
            }
            if out.len() > MAX_STRING_BYTES {
                return self.fail(format!("string longer than {MAX_STRING_BYTES} bytes"));
            }
        }
    }

    /// `{ 4D 5A ?? 00 }`. 점프·대안·반 바이트 와일드카드는 지원하지 않는다.
    fn hex_block(&mut self) -> Result<Vec<Option<u8>>, RuleError> {
        debug_assert_eq!(self.peek(), Some(b'{'));
        self.pos += 1;
        let mut out: Vec<Option<u8>> = Vec::new();
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.pos += 1;
            }
            match self.peek() {
                None => return self.fail("unterminated hex string"),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b'[') => return self.unsupported("hex jump [n-m]"),
                Some(b'(') | Some(b'|') => return self.unsupported("hex alternatives"),
                Some(b'?') => {
                    if self.src.get(self.pos + 1) == Some(&b'?') {
                        out.push(None);
                        self.pos += 2;
                    } else {
                        return self.unsupported("nibble wildcard (use ??)");
                    }
                }
                Some(b) if b.is_ascii_hexdigit() => {
                    match self.src.get(self.pos + 1) {
                        Some(n) if n.is_ascii_hexdigit() => {
                            out.push(Some(u8::from_str_radix(std::str::from_utf8(&self.src[self.pos..self.pos + 2]).unwrap_or("00"), 16).unwrap_or(0)));
                            self.pos += 2;
                        }
                        Some(b'?') => return self.unsupported("nibble wildcard (use ??)"),
                        _ => return self.fail("invalid hex string: bytes need two hex digits"),
                    }
                }
                Some(_) => return self.fail("invalid hex string"),
            }
            if out.len() > MAX_STRING_BYTES {
                return self.fail(format!("hex string longer than {MAX_STRING_BYTES} bytes"));
            }
        }
        if out.len() < 2 {
            return self.fail("hex string must be at least 2 bytes");
        }
        if out[0].is_none() {
            return self.fail("hex string: the first byte must not be a wildcard");
        }
        Ok(out)
    }

    fn integer(&mut self) -> Result<u64, RuleError> {
        let start = self.pos;
        let hex = self.src[self.pos..].starts_with(b"0x");
        if hex {
            self.pos += 2;
        }
        let digits_start = self.pos;
        while self.peek().is_some_and(|b| if hex { b.is_ascii_hexdigit() } else { b.is_ascii_digit() }) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.src[digits_start..self.pos]).unwrap_or("");
        let value = if hex { u64::from_str_radix(text, 16) } else { text.parse() };
        match value {
            Ok(v) if !text.is_empty() => Ok(v),
            _ => {
                self.pos = start;
                self.fail("expected an integer")
            }
        }
    }

    // ---- 조건식 ----

    fn cond_or(&mut self, depth: usize) -> Result<Cond, RuleError> {
        let mut left = self.cond_and(depth)?;
        loop {
            self.skip()?;
            if self.peek_ident().as_deref() == Some("or") {
                self.ident();
                let right = self.cond_and(depth)?;
                left = Cond::Or(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn cond_and(&mut self, depth: usize) -> Result<Cond, RuleError> {
        let mut left = self.cond_not(depth)?;
        loop {
            self.skip()?;
            if self.peek_ident().as_deref() == Some("and") {
                self.ident();
                let right = self.cond_not(depth)?;
                left = Cond::And(Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn cond_not(&mut self, depth: usize) -> Result<Cond, RuleError> {
        self.skip()?;
        if self.peek_ident().as_deref() == Some("not") {
            if depth + 1 > MAX_DEPTH {
                return self.fail(format!("condition nesting is deeper than {MAX_DEPTH}"));
            }
            self.ident();
            return Ok(Cond::Not(Box::new(self.cond_not(depth + 1)?)));
        }
        self.cond_primary(depth)
    }

    fn cond_primary(&mut self, depth: usize) -> Result<Cond, RuleError> {
        self.skip()?;
        match self.peek() {
            Some(b'(') => {
                if depth + 1 > MAX_DEPTH {
                    return self.fail(format!("condition nesting is deeper than {MAX_DEPTH}"));
                }
                self.pos += 1;
                let inner = self.cond_or(depth + 1)?;
                self.expect(b')')?;
                Ok(inner)
            }
            Some(b'$') => {
                self.pos += 1;
                let Some(id) = self.ident() else { return self.fail("expected a string name after '$'") };
                self.skip()?;
                if self.peek_ident().as_deref() == Some("at") {
                    self.ident();
                    self.skip()?;
                    return Ok(Cond::At { id, offset: self.integer()? });
                }
                Ok(Cond::Str(id))
            }
            Some(b'#') => {
                self.pos += 1;
                let Some(id) = self.ident() else { return self.fail("expected a string name after '#'") };
                self.skip()?;
                let op = self.comparison()?;
                self.skip()?;
                Ok(Cond::Count { id, op, n: self.integer()? })
            }
            Some(b) if b.is_ascii_digit() => {
                let n = self.integer()?;
                self.skip()?;
                self.of_them(Quant::N(u32::try_from(n).unwrap_or(u32::MAX)))
            }
            Some(b) if is_ident_start(b) => {
                let word = self.peek_ident().unwrap_or_default();
                match word.as_str() {
                    "any" | "all" => {
                        self.ident();
                        self.skip()?;
                        self.of_them(if word == "any" { Quant::Any } else { Quant::All })
                    }
                    other => self.unsupported(other),
                }
            }
            _ => self.fail("expected a condition"),
        }
    }

    fn of_them(&mut self, quant: Quant) -> Result<Cond, RuleError> {
        if self.ident().as_deref() != Some("of") {
            return self.fail("expected 'of them'");
        }
        self.skip()?;
        match self.ident().as_deref() {
            Some("them") => Ok(Cond::Of { quant }),
            _ => self.unsupported("of (string set); only 'of them' is supported"),
        }
    }

    fn comparison(&mut self) -> Result<Cmp, RuleError> {
        let rest = &self.src[self.pos..];
        for (text, op) in [("==", Cmp::Eq), ("!=", Cmp::Ne), ("<=", Cmp::Le), (">=", Cmp::Ge), ("<", Cmp::Lt), (">", Cmp::Gt)] {
            if rest.starts_with(text.as_bytes()) {
                self.pos += text.len();
                return Ok(op);
            }
        }
        self.fail("expected a comparison operator")
    }

    // ---- 규칙 ----

    fn meta(&mut self, rule: &mut Rule) -> Result<(), RuleError> {
        loop {
            self.skip()?;
            match self.peek_ident().as_deref() {
                Some("strings") | Some("condition") | None => return Ok(()),
                Some(_) => {}
            }
            let key = self.ident().unwrap_or_default();
            self.expect(b'=')?;
            self.skip()?;
            let value = match self.peek() {
                Some(b'"') => Some(String::from_utf8_lossy(&self.string_literal()?).into_owned()),
                Some(b) if b.is_ascii_digit() => {
                    self.integer()?;
                    None
                }
                _ => match self.ident().as_deref() {
                    Some("true") | Some("false") => None,
                    _ => return self.fail("expected a meta value (string, integer, true or false)"),
                },
            };
            match (key.as_str(), value) {
                ("severity", Some(v)) => {
                    rule.severity = match v.to_ascii_lowercase().as_str() {
                        "low" => Severity::Low,
                        "medium" => Severity::Medium,
                        "high" => Severity::High,
                        "critical" => Severity::Critical,
                        _ => return self.fail(format!("invalid severity \"{v}\" (low, medium, high or critical)")),
                    };
                }
                ("severity", None) => return self.fail("severity must be a string"),
                ("description", Some(v)) => rule.description = v,
                _ => {}
            }
        }
    }

    fn strings(&mut self, rule: &mut Rule) -> Result<(), RuleError> {
        loop {
            self.skip()?;
            if self.peek() != Some(b'$') {
                return Ok(());
            }
            self.pos += 1;
            let Some(id) = self.ident() else { return self.fail("expected a string name after '$'") };
            if rule.strings.iter().any(|s| s.id == id) {
                return self.fail(format!("duplicate string ${id}"));
            }
            if rule.strings.len() >= MAX_STRINGS {
                return self.fail(format!("more than {MAX_STRINGS} strings in one rule"));
            }
            self.expect(b'=')?;
            self.skip()?;
            let pattern = match self.peek() {
                Some(b'"') => {
                    let bytes = self.string_literal()?;
                    if bytes.len() < 2 {
                        return self.fail("string must be at least 2 bytes");
                    }
                    let (mut nocase, mut ascii, mut wide) = (false, false, false);
                    loop {
                        self.skip()?;
                        match self.peek_ident().as_deref() {
                            Some("nocase") => nocase = true,
                            Some("wide") => wide = true,
                            Some("ascii") => ascii = true,
                            Some("condition") | None => break,
                            Some(other) => return self.fail(format!("unknown string modifier: {other}")),
                        }
                        self.ident();
                    }
                    Pattern::Text { bytes, nocase, ascii: ascii || !wide, wide }
                }
                Some(b'{') => Pattern::Hex(self.hex_block()?),
                Some(b'/') => return self.unsupported("regular expressions"),
                _ => return self.fail("expected a string (\"…\") or a hex string ({ … })"),
            };
            rule.strings.push(RuleString { id, pattern });
        }
    }

    fn rule(&mut self, names: &mut Vec<String>) -> Result<Rule, RuleError> {
        self.skip()?;
        let name_pos = self.pos;
        let Some(name) = self.ident() else { return self.fail("expected a rule name") };
        if UNSUPPORTED.contains(&name.as_str()) {
            self.pos = name_pos;
            return self.unsupported(&name);
        }
        if names.contains(&name) {
            return self.fail(format!("duplicate rule name {name}"));
        }
        self.expect(b'{')?;
        let mut rule = Rule { name: name.clone(), severity: Severity::Medium, description: String::new(), strings: Vec::new(), condition: Cond::Of { quant: Quant::Any } };
        let mut condition: Option<Cond> = None;
        loop {
            self.skip()?;
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let section_pos = self.pos;
            let Some(section) = self.ident() else { return self.fail("expected meta:, strings:, condition: or '}'") };
            self.expect(b':')?;
            match section.as_str() {
                "meta" => self.meta(&mut rule)?,
                "strings" => self.strings(&mut rule)?,
                "condition" => {
                    condition = Some(self.cond_or(0)?);
                }
                other => {
                    self.pos = section_pos;
                    return self.fail(format!("unknown section: {other}"));
                }
            }
        }
        let Some(cond) = condition else { return self.fail(format!("rule {name} has no condition")) };
        self.check_references(&rule, &cond)?;
        rule.condition = cond;
        names.push(name);
        Ok(rule)
    }

    /// 조건이 쓰는 문자열이 모두 정의되어 있고 `N of them`의 N이 문자열 수를 넘지 않는지 본다.
    fn check_references(&self, rule: &Rule, cond: &Cond) -> Result<(), RuleError> {
        // 깊이가 `MAX_DEPTH`로 제한되므로 재귀가 얕다.
        match cond {
            Cond::Str(id) | Cond::Count { id, .. } | Cond::At { id, .. } => {
                if !rule.strings.iter().any(|s| &s.id == id) {
                    return self.fail(format!("undefined string ${id} in rule {}", rule.name));
                }
                Ok(())
            }
            Cond::Of { quant } => {
                if rule.strings.is_empty() {
                    return self.fail(format!("rule {} uses 'of them' but has no strings", rule.name));
                }
                if let Quant::N(n) = quant
                    && (*n == 0 || *n as usize > rule.strings.len())
                {
                    return self.fail(format!("{n} of them but rule {} has {} string(s)", rule.name, rule.strings.len()));
                }
                Ok(())
            }
            Cond::And(a, b) | Cond::Or(a, b) => {
                self.check_references(rule, a)?;
                self.check_references(rule, b)
            }
            Cond::Not(a) => self.check_references(rule, a),
        }
    }
}

/// 규칙 텍스트를 읽는다. 오류에는 줄 번호가 들어 있다.
pub fn parse(source: &str) -> Result<RuleSet, RuleError> {
    if source.len() > MAX_SOURCE {
        return Err(RuleError { line: 1, message: format!("rules source is too large (max 1 MiB, got {} bytes)", source.len()) });
    }
    let mut p = Parser { src: source.as_bytes(), pos: 0 };
    let mut set = RuleSet::default();
    let mut names: Vec<String> = Vec::new();
    loop {
        p.skip()?;
        if p.peek().is_none() {
            return Ok(set);
        }
        let at = p.pos;
        match p.ident().as_deref() {
            Some("rule") => {
                if set.rules.len() >= MAX_RULES {
                    return p.fail(format!("more than {MAX_RULES} rules"));
                }
                set.rules.push(p.rule(&mut names)?);
            }
            Some(other) if UNSUPPORTED.contains(&other) => {
                p.pos = at;
                return p.unsupported(other);
            }
            _ => return p.fail("expected 'rule'"),
        }
    }
}
