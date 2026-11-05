//! `book_manager` 단위 테스트.

mod common;
#[path = "../shared/book_fixtures.rs"]
mod fx;
use crate::common::shared_fixtures::NCX_NS_STR;
use crate::common::shared_fixtures::epub_with_missing_page;
use crate::common::shared_fixtures::tmp_dir;
use book_manager::_prune_dead_ncx_navpoints;
use common::faulty_fs::FaultyFs;
use common::zipx::read_zip_entry;
use fx::{Opts, with_deps};
use std::collections::BTreeSet;
use std::{fs, io};

/// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
// TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
#[tokio::test]
async fn test_os_error_while_saving_reports_the_system_reason() {
    let faulty = FaultyFs::new();
    // OSError(28): 편집 파일을 백업하기 전에 동기화 단계에서 디스크 부족이 난다.
    faulty.set_sync_file(|_| Some(io::Error::from_raw_os_error(28)));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("파일 처리 실패: No space left on device"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
async fn test_unexpected_error_while_saving_does_not_leak_panic_details() {
    let faulty = FaultyFs::new();
    faulty.set_sync_file(|_| panic!("internal path /x/y"));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("PDF 페이지 삭제 실패 (InternalError)"), "{error:?}");
    e.assert_nothing_changed(&original);
}

fn ncx_srcs(data: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(data).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().filter(|n| n.has_tag_name((NCX_NS_STR, "content"))).filter_map(|c| c.attribute("src").map(str::to_string)).collect()
}

#[test]
fn test_prune_dead_ncx_navpoints_drops_missing_targets_and_keeps_children() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("m.epub"), false);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml"]);
    let pruned = _prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).expect("pruned");
    // 판권 항목만 사라지고, 그 아래 자식은 부모 자리로 올라온다
    assert_eq!(ncx_srcs(&pruned), ["Text/a.xhtml#top", "Text/a.xhtml#kid"]);
}

#[test]
fn test_prune_dead_ncx_navpoints_returns_none_when_nothing_is_dead() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("f.epub"), true);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml", "OEBPS/Text/colophon page.xhtml"]);
    assert!(_prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).is_none());
}

#[test]
fn test_prune_dead_ncx_navpoints_ignores_unparsable_ncx_and_empty_src() {
    assert!(_prune_dead_ncx_navpoints(b"<ncx", "OEBPS", &|_path| false).is_none());
    let no_src = format!(r#"<ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="x"><content src=""/></navPoint><navPoint id="y"/></navMap></ncx>"#).into_bytes();
    assert!(_prune_dead_ncx_navpoints(&no_src, "", &|_path| false).is_none());
}
