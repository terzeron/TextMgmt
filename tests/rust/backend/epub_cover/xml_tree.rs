//! `backend::epub_cover::xml_tree` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs::File;
use backend::book_manager::BookManager;
use common::epub::{EpubSpec, make_epub};
use common::zipx::Members;
use crate::common::shared_fixtures::{spine_idrefs, xml_text};
use crate::common::shared_fixtures::URL;
use crate::common::cover_env::env;

/// 교체로 빠지는 기존 표지는 새 표지 바로 뒤, 본문 앞의 도판으로 남는다.
#[tokio::test]
async fn test_replaced_cover_stays_as_an_illustration_after_the_new_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let old_cover = Members::read(&env.path).get("OPS/cover.jpg").to_vec();

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    let after = Members::read(&env.path);
    assert!(!after.contains("OPS/cover.jpg"));
    let plates: Vec<String> = after.names().into_iter().filter(|name| name.starts_with("OPS/tm-plate-") && name.ends_with(".jpg")).collect();
    assert!(plates.len() == 1 && after.get(&plates[0]) == old_cover);
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-cover-page-") && ids[1].starts_with("tm-plate-page-") && ids[2] == "chapter");
    let page = xml_text(after.get(&format!("OPS/{}.xhtml", ids[1])));
    assert!(page.contains(plates[0].strip_prefix("OPS/").unwrap()));
    // 도판은 표지로 오인되지 않는다. 표지는 새 이미지다.
    let mut archive = zip::ZipArchive::new(File::open(&env.path).unwrap()).unwrap();
    assert!(BookManager::find_epub_cover_member(&mut archive).unwrap().contains("tm-cover-"));
}
