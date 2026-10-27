use super::*;
use std::sync::Arc;

struct NullEs;

impl crate::es_manager::EsManagerApi for NullEs {}

#[test]
fn preview_failure_converts_string_and_zip_errors() {
    assert!(matches!(PreviewFailure::from("parse failed".to_string()), PreviewFailure::Other(message) if message == "parse failed"));
    assert!(matches!(PreviewFailure::from(ZipError::FileNotFound), PreviewFailure::Other(message) if message.contains("not found")));
}

#[test]
fn file_response_reports_read_errors_and_returns_file_contents() {
    let root = tempfile::tempdir().unwrap();
    let manager = BookManager::with_deps(crate::book_manager::BookManagerDeps::new(root.path().to_path_buf(), Arc::new(NullEs)));

    let missing = manager.file_response(&root.path().join("missing.epub"), "application/epub+zip", Vec::new());
    assert_eq!(missing.status_code, 500);
    assert!(String::from_utf8(missing.body).unwrap().starts_with("File read failed:"));

    let path = root.path().join("preview.epub");
    std::fs::write(&path, b"epub bytes").unwrap();
    let response = manager.file_response(&path, "application/epub+zip", vec![("X-Test".into(), "yes".into())]);
    assert_eq!(response.status_code, 200);
    assert_eq!(response.media_type.as_deref(), Some("application/epub+zip"));
    assert_eq!(response.body, b"epub bytes");
    assert_eq!(response.headers, [("X-Test".to_string(), "yes".to_string())]);
}

#[test]
fn ncx_filter_keeps_included_and_empty_targets_and_removes_missing_targets() {
    let ncx = format!(
        r#"<ncx xmlns="{NCX_NS}"><navMap>
<navPoint id="included"><content src="chapter1.xhtml"/></navPoint>
<navPoint id="missing"><content src="chapter2.xhtml#section"/></navPoint>
<navPoint id="empty"><content src=""/></navPoint>
<navPoint id="no-content"><navLabel/></navPoint>
</navMap></ncx>"#
    );
    let include = BTreeSet::from(["OPS/chapter1.xhtml".to_string()]);
    let filtered = String::from_utf8(filter_ncx(ncx.as_bytes(), "OPS/toc.ncx", &include).unwrap()).unwrap();
    assert!(filtered.contains("id=\"included\""));
    assert!(!filtered.contains("id=\"missing\""));
    assert!(filtered.contains("id=\"empty\""));
    assert!(filtered.contains("id=\"no-content\""));
    assert!(filter_ncx(b"<ncx", "OPS/toc.ncx", &include).is_none());
}
