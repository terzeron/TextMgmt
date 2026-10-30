//! 자체 규칙 엔진의 파서 테스트.

use bookdoctor::rules::{Cmp, Cond, Pattern, Quant, parse};
use scan_common::Severity;

fn ok(src: &str) -> bookdoctor::rules::RuleSet {
    parse(src).unwrap_or_else(|e| panic!("line {}: {}\n{src}", e.line, e.message))
}

fn err(src: &str) -> bookdoctor::rules::RuleError {
    parse(src).expect_err(src)
}

fn s(id: &str) -> Box<Cond> {
    Box::new(Cond::Str(id.into()))
}

#[test]
fn parses_a_full_rule() {
    let set = ok(r#"
        rule Sample_One {
          meta:
            severity = "high"
            description = "시험 규칙"
            author = "x"
          strings:
            $a = "text" nocase wide
            $b = { 4D 5A ?? 00 }
            $c = "plain"
          condition:
            any of them
        }
    "#);
    assert_eq!(set.rules.len(), 1);
    let r = &set.rules[0];
    assert_eq!((r.name.as_str(), r.severity, r.description.as_str()), ("Sample_One", Severity::High, "시험 규칙"));
    assert_eq!(r.strings.len(), 3);
    assert_eq!(r.strings[0].pattern, Pattern::Text { bytes: b"text".to_vec(), nocase: true, ascii: false, wide: true });
    assert_eq!(r.strings[1].pattern, Pattern::Hex(vec![Some(0x4D), Some(0x5A), None, Some(0x00)]));
    assert_eq!(r.strings[2].pattern, Pattern::Text { bytes: b"plain".to_vec(), nocase: false, ascii: true, wide: false });
    assert_eq!(r.condition, Cond::Of { quant: Quant::Any });
    // 기본 심각도는 medium, 설명은 빈 문자열이다.
    let d = ok("rule D { strings: $a = \"ab\" condition: $a }");
    assert_eq!((d.rules[0].severity, d.rules[0].description.as_str()), (Severity::Medium, ""));
    // ascii와 wide를 함께 쓰면 둘 다 본다.
    let both = ok("rule B { strings: $a = \"ab\" ascii wide condition: $a }");
    assert_eq!(both.rules[0].strings[0].pattern, Pattern::Text { bytes: b"ab".to_vec(), nocase: false, ascii: true, wide: true });
}

#[test]
fn comments_and_escapes() {
    let set = ok("// line comment\n/* block\n comment */ rule C { strings: $a = \"a\\\\b\\\"c\\n\\r\\t\\x41\" // trailing\n condition: $a }");
    assert_eq!(set.rules[0].strings[0].pattern, Pattern::Text { bytes: b"a\\b\"c\n\r\tA".to_vec(), nocase: false, ascii: true, wide: false });
    assert!(err("rule X { strings: $a = \"bad\\q\" condition: $a }").message.contains("escape"));
}

#[test]
fn hex_with_wildcards_and_a_leading_wildcard_is_rejected() {
    assert!(err("rule X { strings: $a = { ?? 4D 5A } condition: $a }").message.contains("first byte"));
    assert!(err("rule X { strings: $a = { 4D ?5 } condition: $a }").message.contains("nibble") || err("rule X { strings: $a = { 4D ?5 } condition: $a }").message.contains("unsupported"));
    assert!(err("rule X { strings: $a = { 4D 5 } condition: $a }").message.contains("hex"));
    assert!(err("rule X { strings: $a = { 4D [2-4] 5A } condition: $a }").message.contains("unsupported"));
    assert!(err("rule X { strings: $a = { 4D ( 5A | 5B ) } condition: $a }").message.contains("unsupported"));
    assert!(err("rule X { strings: $a = { 4D } condition: $a }").message.contains("at least 2"));
}

#[test]
fn unknown_modifier_and_unsupported_syntax_are_errors_with_line_numbers() {
    let e = err("rule X {\n strings:\n  $a = \"ab\" fullword\n condition: $a }");
    assert_eq!(e.line, 3, "{}", e.message);
    assert!(e.message.contains("fullword"));
    for (src, needle, line) in [
        ("import \"pe\"\nrule X { strings: $a = \"ab\" condition: $a }", "import", 1),
        ("rule X {\n strings:\n  $a = /ab+c/\n condition: $a }", "regular", 3),
        ("rule X { strings: $a = \"ab\" condition: filesize < 10 }", "filesize", 1),
        ("rule X {\n strings: $a = \"ab\"\n condition:\n  for any i in (1..3) : ($a) }", "for", 4),
        ("private rule X { strings: $a = \"ab\" condition: $a }", "private", 1),
        ("rule X { strings: $a = \"ab\" condition: uint8(0) == 0x4d }", "uint8", 1),
    ] {
        let e = err(src);
        assert!(e.message.to_lowercase().contains(needle), "{src}: {}", e.message);
        assert_eq!(e.line, line, "{src}: {}", e.message);
    }
}

#[test]
fn condition_references_must_exist_and_names_must_be_unique() {
    assert!(err("rule X { strings: $a = \"ab\" condition: $b }").message.contains("$b"));
    assert!(err("rule X { strings: $a = \"ab\" condition: #b > 1 }").message.contains("$b"));
    assert!(err("rule X { strings: $a = \"ab\" $a = \"cd\" condition: $a }").message.contains("duplicate"));
    assert!(err("rule X { strings: $a = \"ab\" condition: $a } rule X { strings: $a = \"ab\" condition: $a }").message.contains("duplicate"));
    assert!(err("rule X { condition: any of them }").message.contains("no strings"));
    assert!(err("rule X { strings: $a = \"ab\" condition: 3 of them }").message.contains("3"), "N must not exceed the string count");
    assert!(err("rule X { strings: $a = \"ab\" }").message.contains("condition"));
    assert!(err("rule X { strings: $a = \"\" condition: $a }").message.contains("empty") || err("rule X { strings: $a = \"\" condition: $a }").message.contains("at least 2"));
    assert!(err("rule X { meta: severity = \"fatal\" strings: $a = \"ab\" condition: $a }").message.contains("severity"));
}

#[test]
fn precedence_not_and_or_and_parentheses() {
    let cond = |c: &str| ok(&format!("rule P {{ strings: $a = \"aa\" $b = \"bb\" $c = \"cc\" condition: {c} }}")).rules[0].condition.clone();
    assert_eq!(cond("$a or $b and $c"), Cond::Or(s("a"), Box::new(Cond::And(s("b"), s("c")))));
    assert_eq!(cond("not $a and $b"), Cond::And(Box::new(Cond::Not(s("a"))), s("b")));
    assert_eq!(cond("( $a or $b ) and $c"), Cond::And(Box::new(Cond::Or(s("a"), s("b"))), s("c")));
    assert_eq!(cond("$a and $b and $c"), Cond::And(Box::new(Cond::And(s("a"), s("b"))), s("c")));
    assert_eq!(cond("#a >= 3"), Cond::Count { id: "a".into(), op: Cmp::Ge, n: 3 });
    assert_eq!(cond("$b at 0"), Cond::At { id: "b".into(), offset: 0 });
    assert_eq!(cond("2 of them"), Cond::Of { quant: Quant::N(2) });
    assert_eq!(cond("all of them"), Cond::Of { quant: Quant::All });
    assert_eq!(cond("not not $a"), Cond::Not(Box::new(Cond::Not(s("a")))));
}

#[test]
fn limits_are_errors() {
    let rule = |i: usize| format!("rule R{i} {{ strings: $a = \"ab\" condition: $a }}\n");
    let many: String = (0..1001).map(rule).collect();
    assert!(err(&many).message.contains("1000"));
    assert_eq!(ok(&(0..1000).map(rule).collect::<String>()).rules.len(), 1000);
    let strings: String = (0..257).map(|i| format!("$s{i} = \"ab\" ")).collect();
    assert!(err(&format!("rule X {{ strings: {strings} condition: any of them }}")).message.contains("256"));
    let long = "a".repeat(5000);
    assert!(err(&format!("rule X {{ strings: $a = \"{long}\" condition: $a }}")).message.contains("4096"));
    let deep = format!("{}$a{}", "( ".repeat(33), " )".repeat(33));
    assert!(err(&format!("rule X {{ strings: $a = \"ab\" condition: {deep} }}")).message.contains("nesting"));
    let ok_deep = format!("{}$a{}", "( ".repeat(32), " )".repeat(32));
    ok(&format!("rule X {{ strings: $a = \"ab\" condition: {ok_deep} }}"));
    let big = format!("// {}\nrule X {{ strings: $a = \"ab\" condition: $a }}", "x".repeat(1024 * 1024));
    assert!(err(&big).message.contains("1 MiB") || err(&big).message.contains("too large"));
}

#[test]
fn hostile_nesting_does_not_overflow_the_stack() {
    let started = std::time::Instant::now();
    let parens = format!("rule X {{ strings: $a = \"ab\" condition: {} }}", "(".repeat(900_000));
    assert!(err(&parens).message.contains("nesting"));
    let nots = format!("rule X {{ strings: $a = \"ab\" condition: {} $a }}", "not ".repeat(200_000));
    assert!(err(&nots).message.contains("nesting"));
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
}

#[test]
fn empty_source_is_an_empty_set() {
    assert!(ok("").rules.is_empty());
    assert!(ok("  // only a comment\n/* and a block */\n").rules.is_empty());
}

#[test]
fn the_number_of_condition_terms_per_rule_is_limited() {
    let terms = |n: usize| (0..n).map(|_| "$a").collect::<Vec<_>>().join(" or ");
    ok(&format!("rule X {{ strings: $a = \"ab\" condition: {} }}", terms(1000)));
    let e = err(&format!("rule X {{ strings: $a = \"ab\" condition: {} }}", terms(1001)));
    assert!(e.message.contains("terms") && e.message.contains("1000"), "{}", e.message);
    // `at`·`#`·`of them` 항도 항으로 센다.
    let at: String = (0..1001).map(|i| format!("$a at {i}")).collect::<Vec<_>>().join(" or ");
    assert!(err(&format!("rule X {{ strings: $a = \"ab\" condition: {at} }}")).message.contains("terms"));
}
