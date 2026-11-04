
mod common;
use classifier::py_json_writer::{dumps, float_repr};
use serde_json::{json};
use crate::common::shared_fixtures::{parity};

#[test]
fn float_repr_and_json_dumps_match_python() {
    let p = parity();
    for case in p["float_repr"].as_array().unwrap() {
        assert_eq!(float_repr(case["value"].as_f64().unwrap()), case["repr"].as_str().unwrap(), "{}", case["value"]);
    }
    let dumped = &p["json_dumps"];
    assert_eq!(format!("{}\n", dumps(&dumped["value"], false)), dumped["line"].as_str().unwrap());
    assert_eq!(dumps(&json!({"b": [1, 2.5, {"é": null}], "a": true, "10": 1e-7}), true), dumped["sorted"].as_str().unwrap());
}
