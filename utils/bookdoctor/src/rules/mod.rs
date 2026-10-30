//! 자체 규칙 엔진. YARA 문법의 작은 부분집합으로 쓴 규칙(문자열·16진 패턴과 간단한 조건)을 파일의 원시 바이트에 적용한다.
//! 문법과 YARA와의 차이는 `docs/superpowers/plans/2026-10-30-bookdoctor-p5.md`에 있다. 외부 의존성은 없다.

pub mod builtin;
mod matcher;
mod parse;

pub use matcher::{Compiled, DEFAULT_WORK_BUDGET, RuleMatch, ScanOutcome, StringMatch};
pub use parse::{MAX_RULES, MAX_SOURCE, MAX_STRINGS, MAX_STRING_BYTES, MAX_TERMS, parse};

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

// ---- 규칙 읽기와 파일에 적용 ----

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport};

/// 규칙을 적용하려고 읽는 파일 앞부분의 최대 바이트.
pub const MAX_SCAN_BYTES: u64 = 16 * 1024 * 1024;
/// 디렉터리에서 읽는 `.rules` 파일 수의 상한.
const MAX_RULE_FILES: usize = 100;

/// `--rules` 경로(파일이나 디렉터리의 `*.rules`)와 내장 규칙을 읽어 한 모음으로 합치고 컴파일한다. 규칙이 하나도 없으면 `None`이다.
/// 오류는 `파일:줄: 메시지` 꼴이다. 이름이 겹치는 규칙, 규칙 1,000개 초과, 1MiB 초과 파일은 오류다.
pub fn load(files: &[PathBuf], include_builtin: bool) -> Result<Option<Compiled>, String> {
    let mut sources: Vec<(String, String)> = Vec::new();
    if include_builtin {
        sources.push(("builtin".to_owned(), builtin::BUILTIN.to_owned()));
    }
    for path in files {
        let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut paths = vec![path.clone()];
        if meta.is_dir() {
            let mut found: Vec<PathBuf> = fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rules"))).collect();
            found.sort();
            if found.len() > MAX_RULE_FILES {
                return Err(format!("{}: more than {MAX_RULE_FILES} .rules files", path.display()));
            }
            paths = found;
        }
        for p in paths {
            let mut text = String::new();
            let size = fs::metadata(&p).map(|m| m.len()).map_err(|e| format!("{}: {e}", p.display()))?;
            if size > MAX_SOURCE as u64 {
                return Err(format!("{}: rules file is too large (max 1 MiB)", p.display()));
            }
            fs::File::open(&p).and_then(|mut f| f.read_to_string(&mut text)).map_err(|e| format!("{}: {e}", p.display()))?;
            sources.push((p.display().to_string(), text));
        }
    }
    let mut all = RuleSet::default();
    let mut origin: Vec<String> = Vec::new();
    for (name, text) in sources {
        let set = parse(&text).map_err(|e| format!("{name}:{}: {}", e.line, e.message))?;
        for rule in set.rules {
            if let Some(i) = all.rules.iter().position(|r| r.name == rule.name) {
                return Err(format!("{name}: duplicate rule name {} (already defined in {})", rule.name, origin[i]));
            }
            if all.rules.len() >= MAX_RULES {
                return Err(format!("{name}: more than {MAX_RULES} rules in total"));
            }
            origin.push(name.clone());
            all.rules.push(rule);
        }
    }
    Ok((!all.rules.is_empty()).then(|| all.compile()))
}

/// 일치한 문자열을 `$id ×횟수 @오프셋,…` 꼴로 줄인다.
fn describe(m: &RuleMatch) -> String {
    m.strings.iter().map(|s| format!("${} ×{} @{}", s.id, s.count, s.first.iter().map(usize::to_string).collect::<Vec<_>>().join(","))).collect::<Vec<_>>().join(", ")
}

/// 파일의 앞 16MiB에 규칙을 적용해 일치마다 `RULE_MATCH`(Security, 규칙의 심각도, 위치는 규칙 이름)를 더한다.
pub fn scan_report(path: &Path, set: &Compiled, report: &mut FileReport) {
    scan_report_with_budget(path, set, report, DEFAULT_WORK_BUDGET);
}

/// `scan_report`와 같되 작업량 상한을 정한다(테스트용). 파일을 열 수 없으면 아무것도 하지 않는다(분석기가 이미 알렸다).
pub fn scan_report_with_budget(path: &Path, set: &Compiled, report: &mut FileReport, budget: u64) {
    let Ok(file) = fs::File::open(path) else { return };
    let mut bytes = Vec::new();
    if file.take(MAX_SCAN_BYTES + 1).read_to_end(&mut bytes).is_err() {
        report.mark_incomplete("rules: file could not be read");
        return;
    }
    if bytes.len() as u64 > MAX_SCAN_BYTES {
        bytes.truncate(MAX_SCAN_BYTES as usize);
        report.mark_incomplete(&format!("rules scanned only the first {} MiB", MAX_SCAN_BYTES / (1024 * 1024)));
    }
    let outcome = set.scan_with_budget(&bytes, budget);
    if outcome.work_limit_hit {
        report.mark_incomplete("rule scan stopped at the work limit; matches may be missing");
    }
    for m in &outcome.matches {
        let rule = &set.set().rules[m.rule];
        let detail = if rule.description.is_empty() { describe(m) } else { format!("{}; {}", rule.description, describe(m)) };
        report.push(Category::Security, rule.severity, "RULE_MATCH", &sanitize_display(&rule.name, 120, Some('…')), &sanitize_display(&detail, 300, Some('…')));
    }
}
