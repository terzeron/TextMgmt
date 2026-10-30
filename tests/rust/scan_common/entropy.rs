use scan_common::entropy::{is_text_name, shannon};

#[test]
fn shannon_of_known_inputs() {
    assert_eq!(shannon(&[]), 0.0);
    assert_eq!(shannon(&[0u8; 1000]), 0.0);
    let all: Vec<u8> = (0..=255u8).cycle().take(256 * 8).collect();
    assert!((shannon(&all) - 8.0).abs() < 1e-9, "{}", shannon(&all));
    let two: Vec<u8> = [0u8, 1].iter().copied().cycle().take(1000).collect();
    assert!((shannon(&two) - 1.0).abs() < 1e-9, "{}", shannon(&two));
    // 영어 문장은 대략 4~5 비트다.
    let text = b"the quick brown fox jumps over the lazy dog ".repeat(40);
    let e = shannon(&text);
    assert!(e > 3.0 && e < 5.0, "{e}");
}

#[test]
fn is_text_name_cases() {
    for name in ["word/document.xml", "a/b/c.RELS", "index.HTML", "x.htm", "p.xhtml", "s.js", "s.css", "n.txt", "d.json", "i.svg", "Contents/section0.xml"] {
        assert!(is_text_name(name), "{name}");
    }
    for name in ["a.jpg", "a.png", "a.zip", "a.gz", "BinData/BIN0001.bin", "noext", "dir/", "a.xml.bak", "a.", ".xml", "xml"] {
        assert!(!is_text_name(name), "{name}");
    }
}
