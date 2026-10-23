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
