//! `backend::book_manager::cover_list` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use common::es_fake::FakeEsManager;
use crate::common::shared_fixtures::{asyncio_runner, make_doc, make_manager, tmp_dir};

#[test]
fn test_get_cover_rejects_missing_required_es_fields() {
    let tmp = tmp_dir();
    let mut doc = make_doc("A/book.txt", "txt");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));

    let resp = asyncio_runner(manager.get_cover(1, None));

    assert_eq!(resp.status_code, 500);
}

mod cover_member_lookup {
    use crate::common::shared_fixtures::{CONTAINER};
    use crate::common::shared_fixtures::{make_deps, make_doc};
    use super::common;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use backend::book_metadata::Book;
    use backend::book_manager::{BookManager, CoverPage, CoverPageOpts, encode_category_cursor, MAX_CATEGORY_PAGE_SIZE};

    use backend::es_manager::{EsError, EsManagerApi};
    use common::es_fake::FakeEsManager;
    use common::faulty_fs::FaultyFs;

    use common::zipx::{open_zip, write_zip};
    use serde_json::{json, Value};
    use tempfile::TempDir;
    use zip::ZipArchive;

    // ---- 커버 추출 ----

    fn epub_with(root: &Path, members: &[(&str, &str)]) -> ZipArchive<fs::File> {
        let path = root.join("t.epub");
        let owned: Vec<(&str, &[u8])> = members.iter().map(|(n, c)| (*n, c.as_bytes())).collect();
        write_zip(&path, &owned);
        open_zip(&path)
    }

    /// Python `make_manager(tmp_path, es)`.
    fn make_manager(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManager {
        BookManager::with_deps(make_deps(tmp_path, es))
    }

    // ---- 커버 목록(작품 묶음 + 커서) ----

    fn cover_doc(path: &str, title: &str, category: &str, file_type: &str) -> Value {
        let mut doc = make_doc(path, file_type);
        doc.as_object_mut().unwrap().extend([("title".to_string(), json!(title)), ("category".to_string(), json!(category)), ("file_path".to_string(), json!(path))]);
        doc
    }

    #[derive(Default)]
    struct FileCall {
        categories: Vec<String>,
        size: usize,
        exclude: Vec<String>,
    }

    #[derive(Default)]
    struct RepCall {
        keys: Vec<String>,
        exclude: Vec<String>,
    }

    #[derive(Default)]
    struct Calls {
        files: Vec<FileCall>,
        reps: Vec<RepCall>,
    }

    type Stats = HashMap<String, (usize, f64, f64)>;

    type CoverEntry = (i64, Value, (i64, i64, String));

    /// 커버 목록 조회용 ES 대역. `FakeEsManager`에 커버 목록 메서드가 없어 이 세 메서드만 가진 로컬 대역을 쓴다.
    /// 나머지 `EsManagerApi` 메서드는 기본 구현(`Unsupported`)이다. TODO(common): `FakeEsManager`에 커버 목록 훅이 생기면 지운다.
    type CoverFilesFn = dyn Fn(&[String], i64, Option<(i64, i64, String)>, &[String]) -> (Vec<(i64, Value, Vec<Value>)>, i64) + Send + Sync;

    type CoverRepsFn = dyn Fn(&[String], &[String]) -> HashMap<String, Vec<(i64, Value)>> + Send + Sync;

    struct CoverEs {
        stats: Stats,
        files: Box<CoverFilesFn>,
        reps: Box<CoverRepsFn>,
    }

    impl EsManagerApi for CoverEs {
        fn search_cover_files_paged(&self, categories: &[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]) -> Result<(Vec<(i64, Value, Vec<Value>)>, i64), EsError> {
            Ok((self.files)(categories, size, search_after, exclude))
        }

        fn search_group_representatives(&self, keys: &[String], exclude: &[String]) -> Result<HashMap<String, Vec<(i64, Value)>>, EsError> {
            Ok((self.reps)(keys, exclude))
        }

        fn aggregate_cover_categories(&self) -> Result<Stats, EsError> {
            Ok(self.stats.clone())
        }
    }

    /// ES 없이 커서 조회가 하는 일(최신순 정렬, search_after, 전체 건수)을 흉내 낸다. 받은 인자는 calls에 모은다.
    /// files는 (문서, 수정 시각) 쌍이다. 정렬값은 ES와 같은 (수정 시각, 생성 시각, 파일 경로)다.
    fn fake_cover_es(stats: &Stats, files: &[(Value, i64)], reps: &HashMap<String, Vec<Value>>) -> (Arc<CoverEs>, Arc<Mutex<Calls>>) {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mut entries: Vec<CoverEntry> = files.iter().enumerate().map(|(i, (doc, updated))| (i as i64 + 1, doc.clone(), (*updated, 0, doc["file_path"].as_str().unwrap().to_string()))).collect();
        let key = |item: &CoverEntry| (-item.2.0, -item.2.1, item.2.2.clone());
        entries.sort_by_key(key);

        let files_calls = calls.clone();
        let files_fn = move |categories: &[String], size: i64, search_after: Option<(i64, i64, String)>, exclude: &[String]| {
            let size = size as usize;
            files_calls.lock().unwrap().files.push(FileCall { categories: categories.to_vec(), size, exclude: exclude.to_vec() });
            let eligible: Vec<&CoverEntry> = entries.iter().filter(|item| categories.iter().any(|c| c == item.1["category"].as_str().unwrap())).collect();
            let window: Vec<(i64, Value, Vec<Value>)> = eligible
                .iter()
                .filter(|item| match &search_after {
                    None => true,
                    Some(sa) => key(item) > (-sa.0, -sa.1, sa.2.clone()),
                })
                .take(size)
                .map(|item| (item.0, item.1.clone(), vec![json!(item.2.0), json!(item.2.1), json!(item.2.2)]))
                .collect();
            (window, eligible.len() as i64)
        };
        let rep_calls = calls.clone();
        let reps = reps.clone();
        let reps_fn = move |keys: &[String], exclude: &[String]| {
            rep_calls.lock().unwrap().reps.push(RepCall { keys: keys.to_vec(), exclude: exclude.to_vec() });
            keys.iter().filter(|k| reps.contains_key(*k)).map(|k| (k.clone(), reps[k].iter().enumerate().map(|(i, d)| (900 + i as i64, d.clone())).collect::<Vec<_>>())).collect::<HashMap<_, _>>()
        };
        let es = Arc::new(CoverEs { stats: stats.clone(), files: Box::new(files_fn), reps: Box::new(reps_fn) });
        (es, calls)
    }

    fn titles(items: &[(Book, String)]) -> Vec<(String, String)> {
        items.iter().map(|(book, display)| (book.title.clone(), display.clone())).collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// 디렉토리 A: 직속 파일 가(수정 400), 다(수정 200)와 작품 디렉토리 나작품(최근 300), 라작품(최근 100).
    type CoverFixture = (Vec<(Value, i64)>, HashMap<String, Vec<Value>>, Stats);

    fn cover_fixture() -> CoverFixture {
        let files = vec![(cover_doc("A/가.epub", "가", "A", "epub"), 400), (cover_doc("A/다.pdf", "다", "A", "pdf"), 200)];
        let reps = HashMap::from([
            ("A/나작품".to_string(), vec![cover_doc("A/나작품/나작품 02권.pdf", "나작품 02권", "A/나작품", "pdf"), cover_doc("A/나작품/나작품 01권.pdf", "나작품 01권", "A/나작품", "pdf")]),
            ("A/라작품".to_string(), vec![cover_doc("A/라작품/라작품 1권.pdf", "라작품 1권", "A/라작품", "pdf")]),
        ]);
        let stats = HashMap::from([("A".to_string(), (2, 400.0, 0.0)), ("A/나작품".to_string(), (12, 300.0, 0.0)), ("A/라작품".to_string(), (1, 100.0, 0.0))]);
        (files, reps, stats)
    }

    fn page(category: Option<&str>, size: usize, cursor: Option<&str>, exclude: &[&str]) -> CoverPageOpts {
        CoverPageOpts { category: category.map(str::to_string), size, cursor: cursor.map(str::to_string), exclude_categories: strings(exclude) }
    }

    /// 원본 테스트의 `(항목, 전체 수, 다음 커서, 오류)` 쪽.
    type RawPage = (Vec<(Book, String)>, usize, Option<String>, Option<String>);

    /// `Result<CoverPage, _>`를 원본 테스트 모양으로 펼친다.
    fn unpage(result: Result<CoverPage, String>) -> RawPage {
        match result {
            Ok(page) => (page.items, page.total.unwrap_or(0), page.next_cursor, None),
            Err(error) => (Vec::new(), 0, None, Some(error)),
        }
    }

    /// 쪽 크기 66x165pt인 빈 한 쪽짜리 PDF를 직접 만든다(pikepdf 대신).
    fn blank_pdf(width: u32, height: u32) -> Vec<u8> {
        let objs = ["<< /Type /Catalog /Pages 2 0 R >>".to_string(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(), format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>")];
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, obj) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{obj}\nendobj\n", i + 1));
        }
        let xref = out.len();
        out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for off in &offsets {
            out.push_str(&format!("{off:010} 00000 n \n"));
        }
        out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
        out.into_bytes()
    }

    fn vector_pdf() -> Vec<u8> {
        let content = b"q 1 0 0 rg 40 80 180 220 re f Q\nBT /F1 36 Tf 30 300 Td (RUST COVER) Tj ET\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 450] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            [format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(), content.to_vec(), b"endstream".to_vec()].concat(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(object);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    fn append_large_unreferenced_stream(path: &Path, pdf: &[u8], stream_size: u64) {
        let marker = b"startxref\n";
        let start = pdf.windows(marker.len()).rposition(|window| window == marker).unwrap() + marker.len();
        let previous_xref = String::from_utf8_lossy(&pdf[start..]).lines().next().unwrap().parse::<u64>().unwrap();
        let mut file = fs::File::create(path).unwrap();
        file.write_all(pdf).unwrap();
        let object_offset = file.stream_position().unwrap();
        write!(file, "6 0 obj\n<< /Length {stream_size} >>\nstream\n").unwrap();
        file.seek(SeekFrom::Current(stream_size as i64)).unwrap();
        file.write_all(b"\nendstream\nendobj\n").unwrap();
        let xref_offset = file.stream_position().unwrap();
        write!(file, "xref\n6 1\n{object_offset:010} 00000 n \ntrailer\n<< /Size 7 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{xref_offset}\n%%EOF\n").unwrap();
    }

    #[test]
    fn test_find_epub_cover_member_returns_none_without_opf() {
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &[("readme.txt", "x")]);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
    }

    #[test]
    fn test_find_epub_cover_member_returns_none_when_opf_member_is_missing() {
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER)]);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
    }

    #[test]
    fn test_find_epub_cover_member_returns_none_when_opf_is_not_xml() {
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", "xml 이 아닌 문자열")]);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin), None);
    }

    #[test]
    fn test_find_epub_cover_member_ignores_comment_nodes() {
        let opf = r#"<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <!-- 주석 노드는 태그가 문자열이 아니다 -->
      <manifest><item id="cover" href="img/c.png" media-type="image/png"/></manifest>
    </package>"#;
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &[("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf)]);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin).as_deref(), Some("OEBPS/img/c.png"));
    }

    #[test]
    fn test_find_epub_cover_member_prefers_exact_cover_name_over_incover() {
        // 표지 선언이 없을 때 id/href에 "cover"가 들어간다는 이유만으로 속표지(incover.jpg)를 먼저 고르면 안 된다.
        let opf = r#"<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <manifest>
        <item id="incover.jpg" href="Images/incover.jpg" media-type="image/jpeg"/>
        <item id="cover.jpg" href="Images/cover.jpg" media-type="image/jpeg"/>
      </manifest>
    </package>"#;
        let members = [("META-INF/container.xml", CONTAINER), ("OEBPS/content.opf", opf), ("OEBPS/Images/incover.jpg", "x"), ("OEBPS/Images/cover.jpg", "x")];
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &members);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin).as_deref(), Some("OEBPS/Images/cover.jpg"));
    }

    #[test]
    fn test_find_epub_cover_member_falls_back_to_first_spine_page_image() {
        // 표지 선언이 없고 meta cover 가 존재하지 않는 id 를 가리켜도 첫 spine 페이지의 이미지를 쓴다.
        let opf = r#"<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <metadata><meta name="cover" content="cover"/></metadata>
      <manifest>
        <item id="id0" href="0.html" media-type="application/xhtml+xml"/>
        <item id="id1" href="1.html" media-type="application/xhtml+xml"/>
        <item id="img1" href="images/img1.webp" media-type="image/webp"/>
        <item id="img2" href="images/img2.webp" media-type="image/webp"/>
      </manifest>
      <spine><itemref idref="id0"/><itemref idref="id1"/></spine>
    </package>"#;
        let members = [
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", opf),
            ("OEBPS/0.html", r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img1.webp"/></body></html>"#),
            ("OEBPS/1.html", r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img2.webp"/></body></html>"#),
            ("OEBPS/images/img1.webp", "x"),
            ("OEBPS/images/img2.webp", "x"),
        ];
        let tmp = TempDir::new().unwrap();
        let mut zin = epub_with(tmp.path(), &members);
        assert_eq!(BookManager::_find_epub_cover_member(&mut zin).as_deref(), Some("OEBPS/images/img1.webp"));
    }

    #[test]
    fn test_extract_cover_thumbnail_ignores_unsupported_type() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(BookManager::_extract_cover_thumbnail(&tmp.path().join("a.txt"), "txt", None), None);
    }

    #[tokio::test]
    async fn test_get_cover_returns_404_when_file_is_missing() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::with_doc(make_doc("gone.epub", "epub")));
        let resp = manager.get_cover(1, None).await;
        assert_eq!(resp.status_code, 404);
    }

    #[tokio::test]
    async fn test_get_cover_returns_503_when_storage_access_fails() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("book.epub"), b"PK").unwrap();

        // is_file 확인이 book.epub에서만 EIO로 실패하고 나머지는 실제 파일 시스템에 맡긴다
        let fs_ops = FaultyFs::new();
        fs_ops.set_is_file(|p: &Path| if p.file_name().unwrap() == "book.epub" { Some(std::io::Error::from_raw_os_error(5)) } else { None });
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::with_doc(make_doc("book.epub", "epub"))).with_fs(fs_ops));

        let resp = manager.get_cover(1, None).await;

        assert_eq!(resp.status_code, 503);
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_merges_files_and_work_groups_newest_first() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, total, next_cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

        assert_eq!(error, None);
        assert_eq!(next_cursor, None);
        assert_eq!(total, 4);
        // 묶음은 1권을 대표로 하고 작품 디렉토리 이름을 display_title로 갖는다. 순서는 최신순이다.
        assert_eq!(titles(&items), pairs(&[("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.files[0].categories, strings(&["A"]));
        assert_eq!(calls.reps[0].keys, strings(&["A/나작품", "A/라작품"]));
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_rejects_malformed_es_book() {
        let tmp = TempDir::new().unwrap();
        let mut doc = make_doc("A/bad.epub", "epub");
        doc.as_object_mut().unwrap().remove("category");
        let es = Arc::new(CoverEs {
            stats: HashMap::from([("A".to_string(), (1, 10.0, 0.0))]),
            files: Box::new(move |_, _, _, _| (vec![(1, doc.clone(), vec![json!(10), json!(0), json!("A/bad.epub")])], 1)),
            reps: Box::new(|_, _| HashMap::new()),
        });
        let manager = make_manager(tmp.path(), es);

        let (items, total, cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

        assert!(items.is_empty());
        assert_eq!(total, 0);
        assert!(cursor.is_none());
        assert!(error.unwrap().contains("category"));
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_pages_continue_without_gaps_or_repeats() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, _calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let mut seen: Vec<(String, String)> = Vec::new();
        let mut cursor = String::new();
        let mut finished = false;
        for _ in 0..10 {
            let (items, total, cursor_next, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 1, Some(&cursor), &[])).await);
            assert_eq!(error, None);
            assert_eq!(total, 4);
            seen.extend(titles(&items));
            match cursor_next {
                None => {
                    finished = true;
                    break;
                }
                Some(next) => cursor = next,
            }
        }
        assert!(finished, "커서가 끝나지 않는다");

        assert_eq!(seen, pairs(&[("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]));
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_root_shows_root_files_and_one_card_per_top_level_directory() {
        let tmp = TempDir::new().unwrap();
        let files = vec![(cover_doc("루트책.epub", "루트책", "_root", "epub"), 50), (cover_doc("3_판타지/가.epub", "가", "3_판타지", "epub"), 10)];
        let reps = HashMap::from([("3_판타지".to_string(), vec![files[1].0.clone()]), ("0_웹툰".to_string(), vec![cover_doc("0_웹툰/작품 01권.pdf", "작품 01권", "0_웹툰/작품", "pdf")])]);
        let stats = HashMap::from([("_root".to_string(), (1, 50.0, 0.0)), ("3_판타지".to_string(), (12, 70.0, 0.0)), ("0_웹툰/작품".to_string(), (3, 90.0, 0.0))]);
        let (es, calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, total, next_cursor, error) = unpage(manager.get_cover_books_paged(page(None, 10, None, &[])).await);

        assert_eq!(error, None);
        assert_eq!(next_cursor, None);
        // 3_판타지 직속 책은 펼치지 않고 디렉토리마다 대표 한 권만 나온다. 최신 시각이 큰 묶음부터다.
        let calls = calls.lock().unwrap();
        assert_eq!(calls.files[0].categories, strings(&["_root"]));
        assert_eq!(calls.reps[0].keys, strings(&["0_웹툰", "3_판타지"]));
        assert_eq!(total, 3);
        assert_eq!(titles(&items), pairs(&[("작품 01권", "0_웹툰"), ("가", "3_판타지"), ("루트책", "")]));
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_scopes_to_selected_directory() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, total, _cursor, error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

        assert_eq!(error, None);
        assert_eq!(total, 4);
        assert_eq!(calls.lock().unwrap().files[0].categories, strings(&["A"]));
        let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
        assert_eq!(displays, vec!["", "나작품", "", "라작품"]);
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_excludes_hidden_categories_everywhere() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, total, _cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &["A/나작품"])).await);

        assert_eq!(total, 3);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.reps[0].keys, strings(&["A/라작품"]));
        assert_eq!(calls.files[0].exclude, strings(&["A/나작품"]));
        assert_eq!(calls.reps[0].exclude, strings(&["A/나작품"]));
        let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
        assert_eq!(displays, vec!["", "", "라작품"]);
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_skips_group_without_representative() {
        let tmp = TempDir::new().unwrap();
        let (files, mut reps, stats) = cover_fixture();
        reps.remove("A/나작품");
        let (es, _calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, _total, _cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 10, None, &[])).await);

        let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
        assert_eq!(displays, vec!["", "", "라작품"]);
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_keeps_paging_when_only_files_remain() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, _calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        let (items, _total, next_cursor, _error) = unpage(manager.get_cover_books_paged(page(Some("A"), 2, None, &[])).await);

        let displays: Vec<&str> = items.iter().map(|(_, d)| d.as_str()).collect();
        assert_eq!(displays, vec!["", "나작품"]);
        assert!(next_cursor.is_some());
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_rejects_malformed_cursor() {
        // parametrize: 디코딩 실패, 디코딩은 되지만 [그룹, 파일] 두 칸 형식이 아닌 두 경우
        for bad_cursor in ["bad_cursor!!!", "WzFd", "WyJ4IiwgbnVsbF0="] {
            let tmp = TempDir::new().unwrap();
            let (files, reps, stats) = cover_fixture();
            let (es, _calls) = fake_cover_es(&stats, &files, &reps);
            let manager = make_manager(tmp.path(), es);

            let result = unpage(manager.get_cover_books_paged(page(None, 10, Some(bad_cursor), &[])).await);

            assert!(result.0.is_empty() && result.1 == 0 && result.2.is_none() && result.3.as_deref() == Some("invalid cursor"), "cursor {bad_cursor}");
        }
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_rejects_wrong_cursor_member_shapes() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, _calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        // 묶음·파일 커서는 [수정 시각, 생성 시각, 경로] 세 칸이어야 한다.
        for member in [json!([["a"], null]), json!([null, "x"]), json!([null, [1, 2]]), json!([[1, 2, "k", "extra"], null])] {
            let cursor = encode_category_cursor(&member);
            let result = unpage(manager.get_cover_books_paged(page(None, 10, Some(&cursor), &[])).await);
            assert_eq!(result.3.as_deref(), Some("invalid cursor"), "member {member}");
        }
    }

    #[tokio::test]
    async fn test_get_cover_books_paged_clamps_page_size() {
        let tmp = TempDir::new().unwrap();
        let (files, reps, stats) = cover_fixture();
        let (es, calls) = fake_cover_es(&stats, &files, &reps);
        let manager = make_manager(tmp.path(), es);

        unpage(manager.get_cover_books_paged(page(None, 0, None, &[])).await);
        unpage(manager.get_cover_books_paged(page(None, 1_000_000_000, None, &[])).await);

        let calls = calls.lock().unwrap();
        assert_eq!(calls.files[0].size, 1);
        assert_eq!(calls.files[1].size, MAX_CATEGORY_PAGE_SIZE as usize);
    }

    #[test]
    fn test_extract_cover_thumbnail_scales_tall_first_page_to_exact_cover_size() {
        // 쪽 폭 66pt 의 세로로 긴 쪽은 Poppler 렌더 결과가 한 픽셀 어긋나도 썸네일 크기가 고정된다.
        let tmp = TempDir::new().unwrap();
        let pdf_path = tmp.path().join("tall.pdf");
        fs::write(&pdf_path, blank_pdf(66, 165)).unwrap();

        let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("썸네일");

        assert_eq!(image::guess_format(&data).unwrap(), image::ImageFormat::Jpeg);
        let thumb = image::load_from_memory(&data).unwrap();
        // Python round()는 반올림 동점을 짝수로 보내지만 Rust round()는 0에서 먼 쪽이다. 동점이 나오는 비율이면 달라질 수 있다.
        assert_eq!((thumb.width(), thumb.height()), (200, (200.0 / BookManager::COVER_ASPECT_RATIO).round() as u32));
    }

    #[test]
    fn test_extract_cover_thumbnail_renders_text_and_vector_content() {
        let tmp = TempDir::new().unwrap();
        let pdf_path = tmp.path().join("vector.pdf");
        fs::write(&pdf_path, vector_pdf()).unwrap();

        let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("벡터 PDF 표지");
        let thumb = image::load_from_memory(&data).unwrap().to_rgb8();

        assert_eq!((thumb.width(), thumb.height()), (200, 300));
        assert!(thumb.pixels().any(|pixel| pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100), "빨간 벡터 사각형이 렌더링돼야 한다");
    }

    #[test]
    fn test_extract_cover_thumbnail_renders_first_page_of_large_pdf() {
        let tmp = TempDir::new().unwrap();
        let pdf_path = tmp.path().join("large.pdf");
        let stream_size = 256 * 1024 * 1024 + 1;
        append_large_unreferenced_stream(&pdf_path, &vector_pdf(), stream_size);
        assert!(fs::metadata(&pdf_path).unwrap().len() > 256 * 1024 * 1024);

        let data = BookManager::_extract_cover_thumbnail(&pdf_path, "pdf", Some(200)).expect("큰 PDF 첫 쪽 표지");
        let thumb = image::load_from_memory(&data).unwrap().to_rgb8();

        assert_eq!((thumb.width(), thumb.height()), (200, 300));
        assert!(thumb.pixels().any(|pixel| pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100), "첫 쪽 벡터 표지를 렌더링해야 한다");
    }
}

mod get_cover {
    use crate::common::cover_docs::{create_test_pdf, use_doc};
    use crate::common::shared_fixtures::{thumbnailer};
    use super::common;
    use std::path::Path;
    use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};
    use backend::book_manager::{BookManager, BookManagerDeps};
    use common::epub::{EpubSpec, make_epub};
    use common::es_fake::FakeEsManager;
    use common::fsx::count_matching;
    use common::image::{paste, png_bytes, solid};
    use common::pdf::write_image_pdf;
    use image::{GenericImageView, ImageFormat};
    use serde_json::json;

    struct Env {
        tmp: tempfile::TempDir,
        es: Arc<FakeEsManager>,
        manager: BookManager,
    }

    impl Env {
        fn dir(&self) -> &Path {
            self.tmp.path()
        }
    }

    /// Python `book_manager_module` fixture: ESManager를 대역으로 바꾼 BookManager.
    fn env() -> Env {
        common::setup_env();
        let tmp = tempfile::tempdir().unwrap();
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()));
        Env { tmp, es, manager }
    }

    fn assert_jpeg_thumbnail(body: &[u8]) {
        assert_eq!(image::guess_format(body).unwrap(), ImageFormat::Jpeg);
        let img = image::load_from_memory(body).unwrap();
        assert!(img.width() <= 300, "너비 {}", img.width());
    }

    /// OPF metadata/manifest 조각과 이미지 멤버를 받아 최소 EPUB을 만든다.
    fn create_test_epub(output_path: &Path, metadata: &str, manifest_items: &str, images: &[(&str, Vec<u8>)]) {
        create_test_epub_with_chapters(output_path, metadata, manifest_items, &["<html><body><p>x</p></body></html>"], images);
    }

    /// `create_test_epub`에 본문 쪽(spine 순서)을 직접 정해서 준다. 쪽 파일 이름은 `ch1.xhtml`, `ch2.xhtml`, … 이다.
    fn create_test_epub_with_chapters(output_path: &Path, metadata: &str, manifest_items: &str, chapters: &[&str], images: &[(&str, Vec<u8>)]) {
        let pages: String = (1..=chapters.len()).map(|n| format!(r#"<item id="ch{n}" href="ch{n}.xhtml" media-type="application/xhtml+xml"/>"#)).collect();
        let spine: String = (1..=chapters.len()).map(|n| format!(r#"<itemref idref="ch{n}"/>"#)).collect();
        let opf = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title>{metadata}</metadata>
  <manifest>
    {pages}
    {manifest_items}
  </manifest>
  <spine>{spine}</spine>
</package>"#
        );
        let mut spec = EpubSpec::cover("3.0", false).with_opf(&opf).with_opf_path("OEBPS/content.opf");
        spec.chapters = chapters.iter().enumerate().map(|(index, html)| (format!("ch{}.xhtml", index + 1), html.to_string())).collect();
        spec.include_style = false;
        for (name, data) in images {
            spec = spec.with_extra(&format!("OEBPS/{name}"), data.clone());
        }
        make_epub(output_path, &spec);
    }

    fn decode(body: &[u8]) -> image::DynamicImage {
        image::load_from_memory(body).unwrap()
    }

    const FIGURE: &str = r#"<item id="i1" href="fig.png" media-type="image/png"/>"#;

    const FIGURE_PAGE: &str = r#"<html><body><img src="fig.png"/></body></html>"#;

    const TEXT_PAGE: &str = "<html><body><p>x</p></body></html>";

    fn create_test_spread_pdf(output_path: &Path) {
        let mut spread = solid(1200, 800, [30, 30, 200]);
        paste(&mut spread, (0, 0, 600, 800), [200, 30, 30]);
        write_image_pdf(output_path, &[spread]);
    }

    fn pixel(img: &image::DynamicImage, x: u32, y: u32) -> (u8, u8, u8) {
        let p = img.get_pixel(x, y);
        (p[0], p[1], p[2])
    }

    fn create_test_webtoon_pdf(output_path: &Path) {
        let mut webtoon = solid(600, 2400, [30, 30, 200]);
        paste(&mut webtoon, (0, 0, 600, 900), [200, 30, 30]);
        write_image_pdf(output_path, &[webtoon]);
    }

    fn create_test_pdf_with_blank_first_cover_area(output_path: &Path, background: [u8; 3]) {
        let first_page = solid(600, 900, background);
        let second_page = solid(600, 900, [30, 30, 200]);
        write_image_pdf(output_path, &[first_page, second_page]);
    }

    fn create_test_pdf_with_detailed_first_cover(output_path: &Path) {
        let mut first_page = solid(600, 900, [255, 255, 255]);
        for x in (0..600).step_by(20) {
            paste(&mut first_page, (x, 0, x + 2, 900), [0, 0, 0]);
        }
        for y in (0..900).step_by(20) {
            paste(&mut first_page, (0, y, 600, y + 2), [0, 0, 0]);
        }
        let second_page = solid(600, 900, [30, 30, 200]);
        write_image_pdf(output_path, &[first_page, second_page]);
    }

    #[tokio::test]
    async fn cover_slot_io_error_returns_503_without_running_extractor() {
        let e = env();
        std::fs::write(e.dir().join("source.pdf"), b"pdf").unwrap();
        std::fs::write(e.dir().join(".cover_cache"), b"not a directory").unwrap();
        use_doc(&e.es, "source.pdf", "pdf");
        let manager = BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(thumbnailer(|_, _, _| panic!("must not run without a slot"))));
        assert_eq!(manager.get_cover(1, None).await.status_code, 503);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_cover_generation_stays_at_ten_across_managers_after_cancellation() {
        let e = env();
        std::fs::write(e.dir().join("slow.pdf"), b"pdf").unwrap();
        use_doc(&e.es, "slow.pdf", "pdf");
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let extractor = {
            let (active, peak, started, release) = (active.clone(), peak.clone(), started.clone(), release.clone());
            thumbnailer(move |_, _, _| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                started.fetch_add(1, Ordering::SeqCst);
                let (lock, ready) = &*release;
                let mut released = lock.lock().unwrap();
                while !*released {
                    let (next, timeout) = ready.wait_timeout(released, std::time::Duration::from_secs(5)).unwrap();
                    released = next;
                    if timeout.timed_out() {
                        break;
                    }
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Some(b"cover".to_vec())
            })
        };
        let first = Arc::new(BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(extractor.clone())));
        let second = Arc::new(BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(extractor)));
        let mut tasks = Vec::new();
        for id in 0..10 {
            let manager = first.clone();
            tasks.push(tokio::spawn(async move { manager.get_cover(id, None).await }));
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while started.load(Ordering::SeqCst) < 10 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        // HTTP handler를 버려도 이미 시작한 blocking 추출은 끝날 때까지 슬롯을 쥐어야 한다.
        for task in &tasks {
            task.abort();
        }
        for id in 10..20 {
            let manager = second.clone();
            tasks.push(tokio::spawn(async move { manager.get_cover(id, None).await }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let before_release = started.load(Ordering::SeqCst);
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        for task in tasks {
            let _ = task.await;
        }
        assert_eq!(before_release, 10, "취소된 추출과 다음 관리자 요청이 겹치면 안 된다");
        assert_eq!(started.load(Ordering::SeqCst), 20);
        assert_eq!(peak.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn test_pdf_pixel_stream_formats_and_decode_errors() {
        // pypdfium2 renders malformed Flate data as black and ignores mismatched image dimensions.
        for (kind, expected_status) in [("rgb", 200), ("gray", 200), ("rgb_png", 200), ("corrupt", 200), ("cmyk", 200), ("smask", 200)] {
            let e = env();
            let path = e.dir().join("pixels.pdf");
            common::pdf::make_pixel_pdf(&path, &[kind]);
            use_doc(&e.es, "pixels.pdf", "pdf");
            let response = e.manager.get_cover(1, None).await;
            assert_eq!(response.status_code, expected_status, "{kind}");
            if expected_status == 200 {
                assert_jpeg_thumbnail(&response.body);
            }
        }
    }

    #[tokio::test]
    async fn test_pdf_pixel_stream_size_mismatch_matches_pdfium_rendering() {
        let e = env();
        let path = e.dir().join("pixels.pdf");
        common::pdf::make_pixel_pdf(&path, &["rgb"]);
        common::pdf::edit_pdf(&path, |doc| {
            let image = common::pdf::first_xobject_mut(doc, 1);
            image.dict.set("Width", 1);
            image.dict.set("Height", 1);
        });
        use_doc(&e.es, "pixels.pdf", "pdf");
        assert_eq!(e.manager.get_cover(1, None).await.status_code, 200);
    }

    #[tokio::test]
    async fn test_epub_cover_image_property() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/prop.epub"),
            "",
            r#"<item id="img" href="images/front.png" media-type="image/png" properties="cover-image"/>"#,
            &[("images/front.png", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/prop.epub", "epub");

        let response = e.manager.get_cover(101, None).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.media_type.as_deref(), Some("image/jpeg"));
        assert_jpeg_thumbnail(&response.body);
    }

    #[tokio::test]
    async fn test_epub_meta_cover() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/meta.epub"),
            r#"<meta name="cover" content="img1"/>"#,
            r#"<item id="img1" href="a.jpg" media-type="image/png"/>"#,
            &[("a.jpg", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/meta.epub", "epub");

        let response = e.manager.get_cover(102, None).await;

        assert_eq!(response.status_code, 200);
        assert_jpeg_thumbnail(&response.body);
    }

    #[tokio::test]
    async fn test_epub_cover_named_item() {
        let e = env();
        create_test_epub(
            &e.dir().join("test_category/named.epub"),
            "",
            r#"<item id="i1" href="img/other.png" media-type="image/png"/><item id="i2" href="img/Cover.png" media-type="image/png"/>"#,
            &[("img/other.png", png_bytes(10, 10)), ("img/Cover.png", png_bytes(600, 900))],
        );
        use_doc(&e.es, "test_category/named.epub", "epub");

        let response = e.manager.get_cover(103, None).await;

        assert_eq!(response.status_code, 200);
        // other.png(10x10)가 아니라 Cover.png(600x900)를 골라야 세로 비율이 나온다
        let img = decode(&response.body);
        assert!(img.height() > img.width());
    }

    #[tokio::test]
    async fn test_epub_without_cover_returns_404() {
        let e = env();
        create_test_epub(&e.dir().join("test_category/nocover.epub"), "", r#"<item id="i1" href="fig.png" media-type="image/png"/>"#, &[("fig.png", png_bytes(600, 900))]);
        use_doc(&e.es, "test_category/nocover.epub", "epub");

        let response = e.manager.get_cover(104, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_epub_without_explicit_cover_uses_first_spine_image() {
        let e = env();
        create_test_epub_with_chapters(&e.dir().join("test_category/first.epub"), "", FIGURE, &[FIGURE_PAGE], &[("fig.png", png_bytes(600, 900))]);
        use_doc(&e.es, "test_category/first.epub", "epub");

        let response = e.manager.get_cover(140, None).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.media_type.as_deref(), Some("image/jpeg"));
        assert_jpeg_thumbnail(&response.body);
    }

    #[tokio::test]
    async fn test_explicit_cover_wins_over_first_spine_image() {
        let e = env();
        let manifest = r#"<item id="front" href="front.png" media-type="image/png" properties="cover-image"/><item id="i1" href="fig.png" media-type="image/png"/>"#;
        let images = [("front.png", png_bytes(600, 900)), ("fig.png", png_bytes(900, 600))];
        create_test_epub_with_chapters(&e.dir().join("test_category/explicit.epub"), "", manifest, &[FIGURE_PAGE], &images);
        use_doc(&e.es, "test_category/explicit.epub", "epub");

        let response = e.manager.get_cover(141, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        assert!(img.height() > img.width(), "세로 표지가 아니라 본문 그림(가로)을 썼다: {}x{}", img.width(), img.height());
    }

    #[tokio::test]
    async fn test_epub_image_after_page_limit_is_not_a_cover() {
        let e = env();
        let mut chapters = vec![TEXT_PAGE; 5];
        chapters.push(FIGURE_PAGE);
        create_test_epub_with_chapters(&e.dir().join("test_category/late.epub"), "", FIGURE, &chapters, &[("fig.png", png_bytes(600, 900))]);
        use_doc(&e.es, "test_category/late.epub", "epub");

        assert_eq!(e.manager.get_cover(142, None).await.status_code, 404);
    }

    #[tokio::test]
    async fn test_epub_image_on_fifth_page_is_still_a_cover() {
        let e = env();
        let mut chapters = vec![TEXT_PAGE; 4];
        chapters.push(FIGURE_PAGE);
        create_test_epub_with_chapters(&e.dir().join("test_category/fifth.epub"), "", FIGURE, &chapters, &[("fig.png", png_bytes(600, 900))]);
        use_doc(&e.es, "test_category/fifth.epub", "epub");

        assert_eq!(e.manager.get_cover(143, None).await.status_code, 200);
    }

    #[tokio::test]
    async fn test_epub_page_image_missing_from_zip_is_not_a_cover() {
        let e = env();
        create_test_epub_with_chapters(&e.dir().join("test_category/dangling.epub"), "", FIGURE, &[FIGURE_PAGE], &[]);
        use_doc(&e.es, "test_category/dangling.epub", "epub");

        assert_eq!(e.manager.get_cover(144, None).await.status_code, 404);
    }

    #[tokio::test]
    async fn test_broken_epub_returns_404() {
        let e = env();
        let broken = e.dir().join("test_category/broken.epub");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"not a zip").unwrap();
        use_doc(&e.es, "test_category/broken.epub", "epub");

        let response = e.manager.get_cover(105, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_pdf_first_page() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/doc.pdf"));
        use_doc(&e.es, "test_category/doc.pdf", "pdf");

        let response = e.manager.get_cover(106, None).await;

        assert_eq!(response.status_code, 200);
        assert_jpeg_thumbnail(&response.body);
        let img = decode(&response.body);
        assert_eq!(img.width(), 200);
        let expected = (200.0_f64 * 792.0 / 612.0).round() as i64;
        assert!((img.height() as i64 - expected).abs() <= 1, "높이 {}, 기대 {expected}", img.height());
    }

    #[tokio::test]
    async fn test_pdf_spread_uses_left_half_as_cover() {
        let e = env();
        create_test_spread_pdf(&e.dir().join("test_category/spread.pdf"));
        use_doc(&e.es, "test_category/spread.pdf", "pdf");

        let response = e.manager.get_cover(109, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        assert_eq!((img.width(), img.height()), (200, 267));
        for x in [img.width() / 4, img.width() * 3 / 4] {
            let (red, _green, blue) = pixel(&img, x, img.height() / 2);
            assert!(red > blue, "x={x}: red={red}, blue={blue}");
        }
    }

    #[tokio::test]
    async fn test_pdf_webtoon_uses_top_cover_area() {
        let e = env();
        create_test_webtoon_pdf(&e.dir().join("test_category/webtoon.pdf"));
        use_doc(&e.es, "test_category/webtoon.pdf", "pdf");

        let response = e.manager.get_cover(110, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        assert_eq!((img.width(), img.height()), (200, 300));
        let (red, _green, blue) = pixel(&img, img.width() / 2, img.height() - 10);
        assert!(red > blue, "red={red}, blue={blue}");
    }

    #[tokio::test]
    async fn test_pdf_blank_first_cover_area_uses_second_page() {
        for (background, book_id) in [([255, 255, 255], 111), ([0, 0, 0], 112)] {
            let e = env();
            create_test_pdf_with_blank_first_cover_area(&e.dir().join(format!("test_category/blank-{book_id}.pdf")), background);
            use_doc(&e.es, &format!("test_category/blank-{book_id}.pdf"), "pdf");

            let response = e.manager.get_cover(book_id, None).await;

            assert_eq!(response.status_code, 200, "background={background:?}");
            let img = decode(&response.body);
            assert_eq!((img.width(), img.height()), (200, 300), "background={background:?}");
            let (red, green, blue) = pixel(&img, img.width() / 2, img.height() / 2);
            assert!(blue > red, "background={background:?}");
            assert!(blue > green, "background={background:?}");
        }
    }

    #[tokio::test]
    async fn test_pdf_detailed_first_cover_area_is_kept() {
        let e = env();
        create_test_pdf_with_detailed_first_cover(&e.dir().join("test_category/detailed.pdf"));
        use_doc(&e.es, "test_category/detailed.pdf", "pdf");

        let response = e.manager.get_cover(113, None).await;

        assert_eq!(response.status_code, 200);
        let img = decode(&response.body);
        let (red, green, blue) = pixel(&img, img.width() / 2, img.height() / 2);
        assert!(red.max(green).max(blue) < 150, "({red}, {green}, {blue})");
    }

    #[tokio::test]
    async fn test_text_format_returns_404() {
        let e = env();
        let txt = e.dir().join("test_category/a.txt");
        std::fs::create_dir_all(txt.parent().unwrap()).unwrap();
        std::fs::write(&txt, "hello").unwrap();
        use_doc(&e.es, "test_category/a.txt", "txt");

        let response = e.manager.get_cover(107, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_book_not_found() {
        let e = env();
        e.es.set_doc(None);

        let response = e.manager.get_cover(999, None).await;

        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_cache_hit_skips_extraction() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/cached.pdf"));
        use_doc(&e.es, "test_category/cached.pdf", "pdf");

        let first = e.manager.get_cover(108, None).await;
        assert_eq!(first.status_code, 200);
        assert_eq!(count_matching(&e.dir().join(".cover_cache"), &format!("108.v{}.", BookManager::COVER_CACHE_VERSION), ".jpg"), 1);

        // 두 번째 호출에서 추출기를 부르면 실패해야 한다.
        let strict = thumbnailer(|_path, _type, _width| panic!("must use cache"));
        let second_manager = BookManager::with_deps(BookManagerDeps::new(e.dir().to_path_buf(), e.es.clone()).with_thumbnailer(strict));
        let second = second_manager.get_cover(108, None).await;

        assert_eq!(second.status_code, 200);
    }

    #[tokio::test]
    async fn test_width_tier_is_whitelisted() {
        for (requested, expected) in [(Some(100), 100), (Some(200), 200), (None, 200), (Some(0), 200), (Some(300), 200), (Some(-1), 200)] {
            let e = env();
            create_test_pdf(&e.dir().join("test_category/tier.pdf"));
            use_doc(&e.es, "test_category/tier.pdf", "pdf");

            let response = e.manager.get_cover(130, requested).await;

            assert_eq!(response.status_code, 200, "requested={requested:?}");
            assert!(decode(&response.body).width() <= expected, "requested={requested:?}");
            assert!(count_matching(&e.dir().join(".cover_cache"), &format!("130.v{}.w{expected}.", BookManager::COVER_CACHE_VERSION), ".jpg") > 0, "requested={requested:?}");
        }
    }

    #[tokio::test]
    async fn test_width_tiers_do_not_share_cache() {
        let e = env();
        create_test_pdf(&e.dir().join("test_category/tiers.pdf"));
        use_doc(&e.es, "test_category/tiers.pdf", "pdf");

        let small = e.manager.get_cover(131, Some(100)).await;
        let large = e.manager.get_cover(131, Some(200)).await;

        assert!(decode(&small.body).width() <= 100);
        assert!(decode(&large.body).width() > 100);
        assert_eq!(count_matching(&e.dir().join(".cover_cache"), &format!("131.v{}.w", BookManager::COVER_CACHE_VERSION), ".jpg"), 2);
    }

    #[test]
    fn test_normalize_cover_width_rejects_non_int() {
        assert_eq!(BookManager::normalize_cover_width(json!(100).as_i64()), 100);
        assert_eq!(BookManager::normalize_cover_width(json!("100").as_i64()), BookManager::COVER_THUMBNAIL_WIDTH);
    }
}
