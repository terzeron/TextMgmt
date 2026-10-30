//! 자체 규칙 엔진의 매처 테스트.

use std::time::{Duration, Instant};

use bookdoctor::rules::{RuleSet, parse};

fn set(src: &str) -> RuleSet {
    parse(src).unwrap_or_else(|e| panic!("line {}: {}\n{src}", e.line, e.message))
}

/// 일치한 규칙 이름들.
fn hits(src: &str, data: &[u8]) -> Vec<String> {
    let set = set(src);
    set.scan(data).matches.iter().map(|m| set.rules[m.rule].name.clone()).collect()
}

fn matched(src: &str, data: &[u8]) -> bool {
    !hits(src, data).is_empty()
}

#[test]
fn text_nocase_and_wide() {
    let plain = "rule R { strings: $a = \"abc\" condition: $a }";
    assert!(matched(plain, b"xxabcxx"));
    assert!(!matched(plain, b"xxABCxx"));
    let nocase = "rule R { strings: $a = \"ABC\" nocase condition: $a }";
    assert!(matched(nocase, b"xAbCx") && matched(nocase, b"abc") && !matched(nocase, b"abd"));
    let wide = "rule R { strings: $a = \"abc\" wide condition: $a }";
    assert!(matched(wide, b"x\0a\0b\0c\0y") && !matched(wide, b"xabcy"), "wide only looks at the UTF-16LE form");
    let both = "rule R { strings: $a = \"abc\" ascii wide condition: $a }";
    assert!(matched(both, b"abc") && matched(both, b"a\0b\0c\0"));
    let wide_nocase = "rule R { strings: $a = \"AbC\" wide nocase condition: $a }";
    assert!(matched(wide_nocase, b"a\0B\0c\0") && !matched(wide_nocase, b"a\0B\0d\0"));
    // 비ASCII(UTF-8) 텍스트는 바이트 그대로 일치한다.
    assert!(matched("rule R { strings: $a = \"한글\" condition: $a }", "앞 한글 뒤".as_bytes()));
}

#[test]
fn hex_with_wildcards() {
    let r = "rule R { strings: $a = { 4D ?? 90 00 } condition: $a }";
    assert!(matched(r, &[1, 0x4D, 0x5A, 0x90, 0x00, 2]) && matched(r, &[0x4D, 0xFF, 0x90, 0x00]));
    assert!(!matched(r, &[0x4D, 0x5A, 0x91, 0x00]));
    assert!(!matched(r, &[0x4D, 0x5A, 0x90]), "a pattern that runs past the end of the data must not match or panic");
    assert!(!matched(r, &[0x4D]));
}

#[test]
fn any_all_and_n_of_them() {
    let strings = "strings: $a = \"aa\" $b = \"bb\" $c = \"cc\"";
    let rule = |c: &str| format!("rule R {{ {strings} condition: {c} }}");
    assert!(matched(&rule("any of them"), b"..bb..") && !matched(&rule("any of them"), b"zzzz"));
    assert!(matched(&rule("all of them"), b"aa bb cc") && !matched(&rule("all of them"), b"aa bb"));
    assert!(matched(&rule("2 of them"), b"aa cc") && !matched(&rule("2 of them"), b"aa"));
}

#[test]
fn counts_and_at_offsets() {
    let count = |c: &str| format!("rule R {{ strings: $a = \"ab\" condition: {c} }}");
    assert!(matched(&count("#a >= 3"), b"ab ab ab") && !matched(&count("#a >= 3"), b"ab ab"));
    assert!(matched(&count("#a == 2"), b"ab ab") && matched(&count("#a != 2"), b"ab") && matched(&count("#a < 2"), b"ab") && matched(&count("#a <= 1"), b"ab") && matched(&count("#a > 1"), b"abab"));
    assert!(matched(&count("$a at 0"), b"abxx") && !matched(&count("$a at 0"), b"xabx"));
    assert!(matched(&count("$a at 1"), b"xabx"));
    // 일치가 아주 많아도 `at`은 그 위치에서 직접 확인한다(기록한 오프셋 수에 가려지지 않는다).
    let many: Vec<u8> = b"ab".repeat(5000);
    assert!(matched(&count("$a at 9998"), &many), "the last occurrence must still be anchorable");
    assert!(!matched(&count("$a at 9999"), &many));
    // 개수는 1,000에서 멈춘다.
    assert!(matched(&count("#a == 1000"), &many) && !matched(&count("#a > 1000"), &many));
}

#[test]
fn not_and_or_precedence_at_evaluation() {
    let rule = |c: &str| format!("rule R {{ strings: $a = \"aa\" $b = \"bb\" $c = \"cc\" condition: {c} }}");
    assert!(matched(&rule("$a or $b and $c"), b"aa"), "and binds tighter than or");
    assert!(!matched(&rule("( $a or $b ) and $c"), b"aa"));
    assert!(matched(&rule("not $a and $b"), b"bb") && !matched(&rule("not $a and $b"), b"aa bb"));
    assert!(matched(&rule("not not $a"), b"aa"));
    // 빈 입력에서 `not $a`는 참이다.
    assert!(matched(&rule("not $a"), b"") && !matched(&rule("any of them"), b""));
}

#[test]
fn overlapping_matches_are_all_found_and_reported_with_offsets() {
    let src = "rule R { strings: $a = \"aa\" condition: #a == 3 }";
    let set = set(src);
    let out = set.scan(b"aaaa");
    assert_eq!(out.matches.len(), 1);
    let s = &out.matches[0].strings[0];
    assert_eq!((s.id.as_str(), s.count, s.first.as_slice()), ("a", 3, &[0usize, 1, 2][..]));
    // 처음 오프셋은 5개까지만 담는다.
    let out = set.scan(&b"a".repeat(100));
    assert_eq!(out.matches.len(), 0, "99 matches is not 3");
    let many = "rule M { strings: $a = \"aa\" condition: $a }";
    let s2 = &self::set(many).scan(&b"a".repeat(100)).matches[0].strings[0];
    assert_eq!((s2.count, s2.first.len()), (99, 5));
}

#[test]
fn several_rules_and_unmatched_strings_are_not_listed() {
    let src = "rule One { strings: $a = \"alpha\" $b = \"beta\" condition: any of them }\nrule Two { strings: $c = \"gamma\" condition: $c }";
    let set = set(src);
    let out = set.scan(b"..alpha..gamma..");
    let names: Vec<_> = out.matches.iter().map(|m| set.rules[m.rule].name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    let one = &out.matches[0].strings;
    assert_eq!(one.len(), 1, "only the string that matched is listed: {one:?}");
    assert_eq!(one[0].id, "a");
    assert!(!out.work_limit_hit);
}

#[test]
fn hostile_repeated_prefix_input_hits_the_work_limit_fast() {
    // 같은 바이트가 반복되는 입력과 `aaaa…b` 꼴 패턴 수백 개: 위치마다 후보 수백 개가 길게 일치하다 어긋난다.
    let rules: String = (0..300).map(|i| format!("rule R{i} {{ strings: $a = \"{}b{}\" condition: $a }}\n", "a".repeat(60), i % 10)).collect();
    let set = set(&rules);
    let compiled = set.compile();
    let data = vec![b'a'; 4 * 1024 * 1024];
    let started = Instant::now();
    let out = compiled.scan_with_budget(&data, 20_000_000);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(out.work_limit_hit, "the work budget must stop the scan");
    assert!(out.matches.is_empty());
    // 예산이 충분하면 같은 입력도 끝까지 본다.
    let small = compiled.scan_with_budget(&data[..2000], 1_000_000_000);
    assert!(!small.work_limit_hit);
}

#[test]
fn a_thousand_rules_over_a_megabyte_is_fast() {
    let mut seed = 7u32;
    let mut word = || {
        (0..12).map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            char::from(b'a' + ((seed >> 24) % 26) as u8)
        }).collect::<String>()
    };
    let src: String = (0..1000).map(|i| format!("rule R{i} {{ strings: $a = \"{}\" condition: $a }}\n", word())).collect();
    let set = set(&src);
    let compiled = set.compile();
    let text: Vec<u8> = (0..1_000_000u32).map(|i| b'a' + ((i.wrapping_mul(2_654_435_761) >> 24) % 26) as u8).collect();
    let started = Instant::now();
    let out = compiled.scan(&text);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(!out.work_limit_hit);
}

#[test]
fn condition_evaluation_is_charged_to_the_work_budget() {
    // 데이터는 작아 훑기는 싸지만, 조건이 4KiB 패턴에 대한 `at` 검사를 1,000번 한다. 평가도 예산을 써야 한다.
    let pattern = "a".repeat(4000);
    let terms: String = (0..1000).map(|i| format!("$a at {i}")).collect::<Vec<_>>().join(" or ");
    let set = set(&format!("rule R {{ strings: $a = \"{pattern}\" ascii wide condition: {terms} }}"));
    let compiled = set.compile();
    let data = vec![b'x'; 200];
    let out = compiled.scan_with_budget(&data, 1_000_000);
    assert!(out.work_limit_hit, "1000 at-checks over a 4000-byte pattern cost millions of byte comparisons");
    // 예산이 충분하면 끝까지 평가한다.
    let out = compiled.scan_with_budget(&data, 1_000_000_000);
    assert!(!out.work_limit_hit && out.matches.is_empty());
}

#[test]
fn the_at_check_only_looks_at_that_strings_variants() {
    // 규칙 1,000개(변형 1,000개) 곁에서 `at` 하나를 여러 번 평가해도 빠르다.
    let many: String = (0..999).map(|i| format!("rule M{i} {{ strings: $a = \"zz{i:04}zz\" condition: $a }}\n")).collect();
    let src = format!("{many}rule Target {{ strings: $a = \"abcd\" condition: {} }}", (0..500).map(|i| format!("$a at {i}")).collect::<Vec<_>>().join(" or "));
    let compiled = set(&src).compile();
    let started = Instant::now();
    let out = compiled.scan(b"abcd and more");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(out.matches.len(), 1);
}
