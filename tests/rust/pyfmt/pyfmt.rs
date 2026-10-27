//! Python 호환 서식 테스트.

use pyfmt::*;
use serde_json::json;

#[test]
fn python_repr_of_values() {
    assert_eq!(py_repr(&json!({"a": 1, "b": [true, null, 1.5, "x'y"], "c": "한글"})), "{'a': 1, 'b': [True, None, 1.5, \"x'y\"], 'c': '한글'}");
    assert_eq!(py_repr(&json!("a\"b'c")), "'a\"b\\'c'");
    assert_eq!(py_repr(&json!(2.0)), "2.0");
    assert_eq!(py_repr(&json!("tab\there\\")), "'tab\\there\\\\'");
}

#[test]
fn python_repr_escapes_control_characters_and_formats_floats_like_python() {
    assert_eq!(py_repr(&json!("a\rb")), "'a\\rb'");
    assert_eq!(py_repr(&json!("a\u{1b}b")), "'a\\x1bb'");
    assert_eq!(py_repr(&json!("a\u{200b}b")), "'a\\u200bb'");
    assert_eq!(py_repr(&json!("a\u{e0001}b")), "'a\\U000e0001b'");
    assert_eq!(py_repr(&json!(1e-7)), "1e-07");
    assert_eq!(py_repr(&json!(1.5e20)), "1.5e+20");
    assert_eq!(py_repr(&json!(0.0)), "0.0");
    assert_eq!(py_repr(&json!(-3)), "-3");
}

#[test]
fn py_float_matches_python_repr() {
    assert_eq!(py_float(100.0), "100.0");
    assert_eq!(py_float(66.7), "66.7");
    assert_eq!(py_float(0.0), "0.0");
}

#[test]
fn py_round_matches_python_round_half_even_on_the_binary_value() {
    // Python round(0.03125, 4) == 0.0312 (정확히 표현되는 동점은 짝수 쪽), round(0.35, 1) == 0.3
    assert_eq!(py_round(0.03125, 4), 0.0312);
    assert_eq!(py_round(0.35, 1), 0.3);
    assert_eq!(py_round(2.675, 2), 2.67);
    assert_eq!(py_round(1.0, 4), 1.0);
    assert_eq!(py_round(-0.5, 0), -0.0);
}

#[test]
fn py_isalnum_treats_combining_vowel_signs_as_non_alphanumeric() {
    use pyfmt::py_isalnum;
    assert!(py_isalnum('a') && py_isalnum('가') && py_isalnum('7') && py_isalnum('Ⅻ') && py_isalnum('一') && py_isalnum('இ'));
    assert!(!py_isalnum('\u{bbe}'), "타밀 모음 기호(Mc)");
    assert!(!py_isalnum('\u{bc1}'), "타밀 모음 기호(Mn)");
    assert!(!py_isalnum('_') && !py_isalnum(' ') && !py_isalnum('!') && !py_isalnum('Ⓐ'));
}

#[test]
fn py_isalnum_matches_python_313_unicode_151_for_all_codepoints() {
    // Range oracle generated from `chr(cp).isalnum()` on Python 3.13.7 / Unicode 15.1.0.
    let ranges: Vec<(u32, u32)> = include_str!("../../fixtures/python_alnum_ranges_15_1.txt")
        .lines()
        .map(|line| {
            let (start, end) = line.split_once('-').unwrap();
            (u32::from_str_radix(start, 16).unwrap(), u32::from_str_radix(end, 16).unwrap())
        })
        .collect();
    let mut range_index = 0;
    for codepoint in 0..=0x10ffff {
        while range_index < ranges.len() && codepoint > ranges[range_index].1 {
            range_index += 1;
        }
        let expected = ranges.get(range_index).is_some_and(|(start, end)| codepoint >= *start && codepoint <= *end);
        let actual = char::from_u32(codepoint).is_some_and(pyfmt::py_isalnum);
        assert_eq!(actual, expected, "U+{codepoint:04X}");
    }
}

#[test]
fn py_decimal_digit_matches_unicode_decimal_digits() {
    assert_eq!(pyfmt::py_decimal_digit('３'), Some(3));
    assert_eq!(pyfmt::py_decimal_digit('٧'), Some(7));
    assert_eq!(pyfmt::py_decimal_digit('𝟡'), Some(9));
    assert_eq!(pyfmt::py_decimal_digit('²'), None);
}

#[test]
fn py_decimal_digit_matches_python_313_unicode_151_for_all_codepoints() {
    // Range oracle generated from `chr(cp).isdecimal()` on Python 3.13.7 / Unicode 15.1.0.
    let ranges: Vec<(u32, u32)> = include_str!("../../fixtures/python_decimal_ranges_15_1.txt")
        .lines()
        .map(|line| {
            let (start, end) = line.split_once('-').unwrap();
            (u32::from_str_radix(start, 16).unwrap(), u32::from_str_radix(end, 16).unwrap())
        })
        .collect();
    let mut range_index = 0;
    for codepoint in 0..=0x10ffff {
        while range_index < ranges.len() && codepoint > ranges[range_index].1 {
            range_index += 1;
        }
        let expected = ranges.get(range_index).and_then(|(start, end)| if codepoint >= *start && codepoint <= *end { Some(((codepoint - start) % 10) as u8) } else { None });
        let actual = char::from_u32(codepoint).and_then(pyfmt::py_decimal_digit);
        assert_eq!(actual, expected, "U+{codepoint:04X}");
    }
}
