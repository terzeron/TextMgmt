//! `common` 최상위 함수 테스트.

#[test]
fn panic_message_reads_str_and_string_payloads_only() {
    let text: Box<dyn std::any::Any + Send> = Box::new("boom");
    let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("owned"));
    let other: Box<dyn std::any::Any + Send> = Box::new(5u8);
    assert_eq!(common::panic_message(text.as_ref()), Some("boom"));
    assert_eq!(common::panic_message(owned.as_ref()), Some("owned"));
    assert_eq!(common::panic_message(other.as_ref()), None);
}
