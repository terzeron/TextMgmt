use pyferrite::num::*;
use std::f32::consts::PI;

#[test]
fn f16_roundtrip() {
    for v in [0.0f32, 1.0, -1.0, 0.5, 65504.0, 1e-8, PI, -2.5] {
        let h = F16::from_f32(v);
        let back = h.to_f32();
        assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {back}");
    }
    assert_eq!(F16::from_f32(1.0).to_bits(), 0x3C00);
    assert_eq!(F16::from_f32(-2.0).to_bits(), 0xC000);
    assert_eq!(F16::from_bits(0x3555).to_f32(), 1365.0 / 4096.0);
}

#[test]
fn bf16_roundtrip() {
    assert_eq!(BF16::from_f32(1.0).to_bits(), 0x3F80);
    assert_eq!(BF16::from_f32(-2.0).to_bits(), 0xC000);
    assert!((BF16::from_f32(PI).to_f32() - PI).abs() < 0.02);
}

#[test]
fn f8_roundtrip() {
    assert_eq!(F8E4M3::from_f32(1.0).to_bits(), 0x38);
    assert_eq!(F8E4M3::from_f32(448.0).to_bits(), 0x7E);
    assert_eq!(F8E5M2::from_f32(1.0).to_bits(), 0x3C);
    assert!((F8E4M3::from_f32(0.5).to_f32() - 0.5).abs() < 1e-6);
}

#[test]
fn f128_arith() {
    let a = F128::from_f64(1.0);
    let b = F128::from_f64(3.0);
    let c = a / b;
    assert!((c.to_f64() - 1.0 / 3.0).abs() < 1e-15, "{}", c.to_f64());
    let d = c * b;
    assert!((d.to_f64() - 1.0).abs() < 1e-15, "{}", d.to_f64());
    let e = F128::from_f64(0.1) + F128::from_f64(0.2);
    assert!((e.to_f64() - 0.30000000000000004).abs() < 1e-16, "{}", e.to_f64());
    assert!((F128::from_f64(5.0) - F128::from_f64(7.5)).to_f64() == -2.5);
    assert_eq!(F128::from_f64(2.5).to_f64(), 2.5);
    assert!(F128::from_f64(f64::NAN).is_nan());
}

#[test]
fn x87_roundtrip() {
    for v in [1.0f64, -2.5, 1e300, 1e-300, 0.0, 123.456] {
        let q = F128::from_f64(v);
        let enc = encode_x87(q);
        let dec = decode_x87(&enc);
        assert_eq!(dec.to_f64(), v, "{v}");
    }
}
