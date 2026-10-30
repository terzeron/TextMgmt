//! 규칙을 바이트에 적용한다. 한 번의 훑기로 모든 패턴을 찾고(첫 바이트 표로 후보만 확인) 그 결과로 조건을 평가한다.
//! 적대적 입력(같은 바이트 반복 × 비슷한 접두 패턴 다수)에 대비해 비교 작업량에 상한이 있고, 넘으면 중단한다.

use std::collections::HashMap;

use super::{Cmp, Cond, Pattern, Quant, RuleSet};

/// 한 번의 검사에서 비교하는 작업량(위치 수 + 비교한 바이트 수)의 기본 상한.
pub const DEFAULT_WORK_BUDGET: u64 = 500_000_000;
/// 문자열 하나의 일치 횟수는 여기서 멈춘다(`#id`는 이 값 이상을 구별하지 못한다).
const COUNT_CAP: usize = 1000;
/// 보고에 담는 문자열당 처음 오프셋 수.
const FIRST_OFFSETS: usize = 5;

/// 일치한 문자열 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringMatch {
    pub id: String,
    /// 일치 횟수(1,000에서 멈춘다).
    pub count: usize,
    /// 처음 일치한 오프셋(최대 5개).
    pub first: Vec<usize>,
}

/// 조건이 참인 규칙 하나. `strings`는 일치한 문자열만 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatch {
    pub rule: usize,
    pub strings: Vec<StringMatch>,
}

/// 검사 결과. `work_limit_hit`이면 훑기를 중간에 멈췄으므로 일치하지 않았다는 결과는 믿을 수 없다(일치한 규칙은 유효하다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOutcome {
    pub matches: Vec<RuleMatch>,
    pub work_limit_hit: bool,
}

/// 비교용으로 펼친 패턴 하나(`wide`는 UTF-16LE로 펼친 것). `None`은 와일드카드다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Variant {
    /// `Compiled`의 평평한 문자열 번호.
    string: usize,
    pat: Vec<Option<u8>>,
    /// 대소문자를 무시한다(`pat`은 소문자로 저장한다).
    nocase: bool,
}

/// 첫 바이트 표를 만들어 둔 규칙 모음. 파일마다 새로 만들지 않도록 한 번 컴파일해서 재사용한다.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    set: RuleSet,
    variants: Vec<Variant>,
    /// 첫 바이트 → 그 바이트로 시작할 수 있는 `variants` 번호.
    table: Vec<Vec<u32>>,
    /// 규칙마다 첫 문자열의 평평한 번호.
    rule_first: Vec<usize>,
    /// 평평한 문자열 번호 → 그 문자열의 `variants` 번호(`at` 검사가 다른 문자열의 변형을 훑지 않게 한다).
    string_variants: Vec<Vec<u32>>,
    /// 규칙마다 문자열 이름 → 규칙 안의 번호(조건의 이름 찾기를 선형으로 하지 않는다).
    id_index: Vec<HashMap<String, usize>>,
    strings: usize,
}

fn lower(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

impl RuleSet {
    /// 첫 바이트 표를 만든다.
    pub fn compile(&self) -> Compiled {
        let mut variants: Vec<Variant> = Vec::new();
        let mut table: Vec<Vec<u32>> = vec![Vec::new(); 256];
        let mut rule_first = Vec::with_capacity(self.rules.len());
        let mut string_variants: Vec<Vec<u32>> = Vec::new();
        let mut id_index: Vec<HashMap<String, usize>> = Vec::with_capacity(self.rules.len());
        let mut flat = 0usize;
        for rule in &self.rules {
            rule_first.push(flat);
            id_index.push(rule.strings.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect());
            for s in &rule.strings {
                let mut made: Vec<Variant> = Vec::new();
                match &s.pattern {
                    Pattern::Text { bytes, nocase, ascii, wide } => {
                        let norm = |b: u8| if *nocase { lower(b) } else { b };
                        if *ascii {
                            made.push(Variant { string: flat, pat: bytes.iter().map(|&b| Some(norm(b))).collect(), nocase: *nocase });
                        }
                        if *wide {
                            made.push(Variant { string: flat, pat: bytes.iter().flat_map(|&b| [Some(norm(b)), Some(0)]).collect(), nocase: *nocase });
                        }
                    }
                    Pattern::Hex(pat) => made.push(Variant { string: flat, pat: pat.clone(), nocase: false }),
                }
                string_variants.push(Vec::new());
                for v in made {
                    let index = variants.len() as u32;
                    string_variants[flat].push(index);
                    // 파서가 첫 바이트가 와일드카드가 아님을 보장한다.
                    if let Some(first) = v.pat.first().copied().flatten() {
                        table[usize::from(first)].push(index);
                        if v.nocase && first.is_ascii_lowercase() {
                            table[usize::from(first.to_ascii_uppercase())].push(index);
                        }
                    }
                    variants.push(v);
                }
                flat += 1;
            }
        }
        Compiled { set: self.clone(), variants, table, rule_first, string_variants, id_index, strings: flat }
    }

    /// 컴파일해서 한 번 적용한다(여러 파일에 쓸 때는 `compile`한 `Compiled`를 재사용한다).
    pub fn scan(&self, data: &[u8]) -> ScanOutcome {
        self.compile().scan(data)
    }
}

impl Compiled {
    pub fn set(&self) -> &RuleSet {
        &self.set
    }

    pub fn scan(&self, data: &[u8]) -> ScanOutcome {
        self.scan_with_budget(data, DEFAULT_WORK_BUDGET)
    }

    /// 작업량 상한을 정해 적용한다. 상한을 넘으면 거기서 멈추고 `work_limit_hit`이다.
    pub fn scan_with_budget(&self, data: &[u8], budget: u64) -> ScanOutcome {
        let mut counts = vec![0usize; self.strings];
        let mut firsts: Vec<Vec<usize>> = vec![Vec::new(); self.strings];
        let mut work: u64 = 0;
        let mut work_limit_hit = false;
        for pos in 0..data.len() {
            work += 1;
            for &vi in &self.table[usize::from(data[pos])] {
                let v = &self.variants[vi as usize];
                if pos + v.pat.len() > data.len() {
                    work += 1;
                    continue;
                }
                let mut compared = 0u64;
                let mut ok = true;
                for (k, want) in v.pat.iter().enumerate() {
                    compared += 1;
                    if let Some(w) = want {
                        let d = if v.nocase { lower(data[pos + k]) } else { data[pos + k] };
                        if d != *w {
                            ok = false;
                            break;
                        }
                    }
                }
                work += compared;
                if ok {
                    if counts[v.string] < COUNT_CAP {
                        counts[v.string] += 1;
                    }
                    if firsts[v.string].len() < FIRST_OFFSETS {
                        firsts[v.string].push(pos);
                    }
                }
            }
            if work > budget {
                work_limit_hit = true;
                break;
            }
        }
        // 조건 평가도 같은 크기의 예산을 쓴다(별도 계수기: 훑기가 예산을 다 써도 거기까지 찾은 일치는 평가한다).
        let mut eval_work: u64 = 0;
        let mut matches = Vec::new();
        for (r, rule) in self.set.rules.iter().enumerate() {
            if eval_work > budget {
                work_limit_hit = true;
                break;
            }
            if self.eval(r, &rule.condition, &counts, data, &mut eval_work, budget) {
                let first = self.rule_first[r];
                let strings = rule.strings.iter().enumerate().filter(|(i, _)| counts[first + i] > 0).map(|(i, s)| StringMatch { id: s.id.clone(), count: counts[first + i], first: firsts[first + i].clone() }).collect();
                matches.push(RuleMatch { rule: r, strings });
            }
        }
        work_limit_hit |= eval_work > budget;
        ScanOutcome { matches, work_limit_hit }
    }

    fn flat_index(&self, rule: usize, id: &str) -> Option<usize> {
        self.id_index[rule].get(id).map(|i| self.rule_first[rule] + i)
    }

    /// `at`: 기록한 오프셋이 아니라 그 위치에서 직접 패턴을 확인한다(일치가 아주 많아도 가려지지 않는다).
    fn matches_at(&self, flat: usize, data: &[u8], offset: u64) -> bool {
        let Ok(offset) = usize::try_from(offset) else { return false };
        self.string_variants[flat].iter().map(|&vi| &self.variants[vi as usize]).any(|v| {
            let Some(window) = data.get(offset..offset.saturating_add(v.pat.len())).filter(|w| w.len() == v.pat.len()) else { return false };
            v.pat.iter().zip(window).all(|(want, &d)| want.is_none_or(|w| w == if v.nocase { lower(d) } else { d }))
        })
    }

    /// 조건을 평가한다. 노드마다 작업량을 더하고(`at`은 비교할 패턴 길이만큼) 예산을 넘으면 더 평가하지 않고 `false`다.
    fn eval(&self, rule: usize, cond: &Cond, counts: &[usize], data: &[u8], work: &mut u64, budget: u64) -> bool {
        if *work > budget {
            return false;
        }
        *work += 1;
        match cond {
            Cond::Str(id) => self.flat_index(rule, id).is_some_and(|f| counts[f] > 0),
            Cond::Count { id, op, n } => self.flat_index(rule, id).is_some_and(|f| {
                let c = counts[f] as u64;
                match op {
                    Cmp::Eq => c == *n,
                    Cmp::Ne => c != *n,
                    Cmp::Lt => c < *n,
                    Cmp::Le => c <= *n,
                    Cmp::Gt => c > *n,
                    Cmp::Ge => c >= *n,
                }
            }),
            Cond::At { id, offset } => self.flat_index(rule, id).is_some_and(|f| {
                *work += self.string_variants[f].iter().map(|&vi| self.variants[vi as usize].pat.len() as u64).sum::<u64>();
                self.matches_at(f, data, *offset)
            }),
            Cond::Of { quant } => {
                let first = self.rule_first[rule];
                let total = self.set.rules[rule].strings.len();
                *work += total as u64;
                let found = (0..total).filter(|i| counts[first + i] > 0).count();
                match quant {
                    Quant::Any => found >= 1,
                    Quant::All => found == total,
                    Quant::N(n) => found >= *n as usize,
                }
            }
            Cond::And(a, b) => self.eval(rule, a, counts, data, work, budget) && self.eval(rule, b, counts, data, work, budget),
            Cond::Or(a, b) => self.eval(rule, a, counts, data, work, budget) || self.eval(rule, b, counts, data, work, budget),
            Cond::Not(a) => !self.eval(rule, a, counts, data, work, budget),
        }
    }
}
