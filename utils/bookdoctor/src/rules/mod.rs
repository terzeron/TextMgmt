//! 자체 규칙 엔진. YARA 문법의 작은 부분집합으로 쓴 규칙(문자열·16진 패턴과 간단한 조건)을 파일의 원시 바이트에 적용한다.
//! 문법과 YARA와의 차이는 `docs/superpowers/plans/2026-10-30-bookdoctor-p5.md`에 있다. 외부 의존성은 없다.

mod matcher;
mod parse;

pub use matcher::{Compiled, DEFAULT_WORK_BUDGET, RuleMatch, ScanOutcome, StringMatch};
pub use parse::{MAX_RULES, MAX_SOURCE, MAX_STRINGS, MAX_STRING_BYTES, parse};

use scan_common::Severity;

/// 파싱한 규칙 모음.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

/// 규칙 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub severity: Severity,
    pub description: String,
    pub strings: Vec<RuleString>,
    pub condition: Cond,
}

/// 이름(`$a`에서 `a`)이 붙은 문자열 패턴.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleString {
    pub id: String,
    pub pattern: Pattern,
}

/// 문자열 패턴. 텍스트는 `ascii`(기본)와 `wide`(UTF-16LE로 본다)를 고를 수 있고, 16진은 `None`이 와일드카드(`??`)다.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Text { bytes: Vec<u8>, nocase: bool, ascii: bool, wide: bool },
    Hex(Vec<Option<u8>>),
}

/// `N of them`의 수량.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quant {
    Any,
    All,
    N(u32),
}

/// `#id`와 정수의 비교.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// 조건식.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    /// `$id`: 문자열이 한 번이라도 일치한다.
    Str(String),
    /// `#id op n`: 일치 횟수를 비교한다.
    Count { id: String, op: Cmp, n: u64 },
    /// `$id at n`: 오프셋 `n`에서 일치한다.
    At { id: String, offset: u64 },
    /// `any|all|N of them`.
    Of { quant: Quant },
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
}

/// 규칙 파싱 오류. `line`은 1부터 센다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub line: usize,
    pub message: String,
}
