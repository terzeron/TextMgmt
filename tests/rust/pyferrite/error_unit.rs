use super::*;

#[test]
fn every_error_variant_has_a_stable_display_message() {
    let errors = [
        (Error::format("bad"), "malformed file: bad".to_owned()),
        (Error::unsupported("feature"), "unsupported feature: feature".to_owned()),
        (Error::cast("lossy"), "conversion error: lossy".to_owned()),
        (Error::invalid("option"), "invalid argument: option".to_owned()),
        (
            Error::Shape { expected: vec![2, 3], got: 5 },
            "shape [2, 3] does not match 5 elements".to_owned(),
        ),
        (Error::pickle("opcode"), "pickle error: opcode".to_owned()),
        (Error::Compression("zlib".into()), "compression error: zlib".to_owned()),
        (Error::Exhausted, "stream already exhausted".to_owned()),
        (Error::Io(std::io::Error::other("closed")), "io error: closed".to_owned()),
    ];
    for (error, expected) in errors {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn error_sources_and_standard_error_conversions_are_preserved() {
    let io_error = std::io::Error::other("disk");
    let wrapped = Error::from(io_error);
    assert!(std::error::Error::source(&wrapped).is_some());
    assert!(std::error::Error::source(&Error::format("bad")).is_none());

    let utf8_error = String::from_utf8(vec![0xFF]).unwrap_err();
    let converted = Error::from(utf8_error);
    assert!(matches!(converted, Error::Format(message) if message.starts_with("invalid utf-8:")));
}
