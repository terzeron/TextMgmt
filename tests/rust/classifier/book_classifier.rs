//! `classifier::book_classifier` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

mod common;

// `tests/rust/classifier/book_classifier.rs`에서 옮긴 테스트.
mod book_classifier {
    //! 원본: tests/test_book_classifier.py
    //!
    //! 가정한 API:
    //! - `classifier::book_classifier::inspect_epub_metadata(path: &Path) -> EpubMetadata` (title/author/subject/description: String, `PartialEq + Debug`)
    //! - `classifier::book_classifier::inspect_txt_content(path: &Path) -> TxtContent` (hashtags: Vec<String>, snippet: String, header_genre: Option<String>)
    //! - `classifier::book_classifier::clean_filename_to_author_title(name: &str) -> (String, String, String)`
    //! - `classifier::book_classifier::get_effective_filename(path: &Path, root: &Path) -> String`
    //! - `classifier::book_classifier::title_similarity(a: &str, b: &str) -> f64`
    //! - `classifier::book_classifier::is_single_match_valid(search: &str, found: &str) -> bool`
    //! - `classifier::book_classifier::is_title_match_reliable(search: &str, found: &str) -> bool`
    //! - `classifier::book_classifier::map_category(cat: &str, title: &str, author: &str) -> Option<String>`
    //! - `classifier::book_classifier::clean_empty_parent_dirs(path: &Path, stop_dir: &Path)`
    //! - `classifier::book_classifier::MAX_OPF_BYTES: usize`
    //! - `classifier::book_classifier::BookClassifierService::new(library_root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn classifier::Classify>>) -> Self` (구 `Option<Box<..>>`, docs/rust-test-seams.md 4.1)
    //!   - `with_delay(f64)`, `with_bookstore_cache_file(&Path)`, `with_bookstore_policy(BookstorePolicy)`, `with_bookstore_decider(Arc<dyn BookstoreDecider>)` 소비형 빌더 (3.12). 구 `for_test`, `set_bookstore_policy`를 대체한다
    //!   - `classify_file`, `_decide(...)` -> `Decision { category, method, reason, model_category, confidence }`, `_decide_by_bookstore(&mut Value) -> (Option<String>, String, String)`
    //! - `classifier::Classify` 트레이트: `is_available()`, `classify_path(&Path, Option<f64>) -> Result<Prediction, String>`, `classify_document(&Value, Option<f64>) -> Result<Prediction, String>`
    //! - `classifier::model::Prediction::new(category: Option<&str>, confidence: f64, top: Vec<(String, f64)>, reason: &str)`
    //! - `classifier::BookCategoryClassifier::new(model: Option<Arc<dyn CategoryModelApi>>, ClassifierOptions)`는 `Classify`를 구현한다
    //! - `classifier::book_classifier::BookstorePolicy { override_below: f64 }` (`prefers_bookstore(confidence) = confidence < override_below`)
    //!
    //! `test_book_classifier_service_methods_and_cache`와 `test_classify_file_exposes_model_confidence`는 서비스 내부 바꿔치기(`stub_*`, `set_fs`, `set_bookstores`) 대신
    //! `with_fs`(FaultyFs), 서점 클라이언트 pub 필드(`StubBookstore`), 모델 대역으로 같은 갈래를 만든다. 단언은 원본 그대로다.
    #![allow(clippy::type_complexity)]
    use super::{common};

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use classifier::book_classifier::*;
    use classifier::model::Prediction;
    use classifier::{BookCategoryClassifier, ClassifierOptions, Classify};
    use common::zipx::write_zip_text;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    fn empty_meta() -> EpubMetadata {
        EpubMetadata { title: String::new(), author: String::new(), subject: String::new(), description: String::new() }
    }

    #[test]
    fn test_clean_filename_to_author_title() {
        let (author, title, st) = clean_filename_to_author_title("[남희성] 달빛조각사 1-50권 完.txt");
        assert_eq!(author, "남희성");
        assert!(title.contains("달빛조각사"));
        assert_eq!(st, "달빛조각사");
        let (author2, _title2, st2) = clean_filename_to_author_title("화산귀환 001-500화 (비가).txt");
        assert_eq!(author2, "비가");
        assert!(st2.contains("화산귀환"));
    }

    #[test]
    fn test_get_effective_filename() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("library");
        let series_dir = root.join("[전민희] 룬의 아이들 윈터러");
        fs::create_dir_all(&series_dir).unwrap();
        let vol_file = series_dir.join("01권.txt");
        fs::write(&vol_file, "content").unwrap();
        let eff = get_effective_filename(&vol_file, &root);
        assert!(eff.contains("[전민희] 룬의 아이들 윈터러") || eff.contains("01권.txt"));
    }

    #[test]
    fn test_get_effective_filename_keeps_direct_child_and_inherits_parent_author() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("library");
        fs::create_dir(&root).unwrap();
        let direct_file = root.join("제목.txt");
        fs::write(&direct_file, "content").unwrap();
        let series_dir = root.join("[작가] 시리즈");
        fs::create_dir(&series_dir).unwrap();
        let chapter_file = series_dir.join("긴 제목.txt");
        fs::write(&chapter_file, "content").unwrap();

        assert_eq!(get_effective_filename(&direct_file, &root), "제목.txt");
        assert_eq!(get_effective_filename(&chapter_file, &root), "[작가] 긴 제목.txt");
    }

    #[test]
    fn test_inspect_epub_metadata_extracts_opf_fields() {
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("metadata.epub");
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
    <package xmlns:dc="http://purl.org/dc/elements/1.1/">
      <metadata>
        <dc:title>룬의 아이들</dc:title>
        <dc:creator>전민희</dc:creator>
        <dc:subject>판타지</dc:subject>
        <dc:subject>한국소설</dc:subject>
        <dc:description>마법과 모험 이야기</dc:description>
      </metadata>
    </package>
    "#;
        write_zip_text(&epub_path, &[("OPS/content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta, EpubMetadata { title: "룬의 아이들".into(), author: "전민희".into(), subject: "판타지 > 한국소설".into(), description: "마법과 모험 이야기".into() });
    }

    #[test]
    fn test_inspect_epub_metadata_returns_empty_metadata_without_opf_or_on_error() {
        let tmp = TempDir::new().unwrap();
        let no_opf = tmp.path().join("no-opf.epub");
        write_zip_text(&no_opf, &[("mimetype", "application/epub+zip")], false);
        let broken = tmp.path().join("broken.epub");
        fs::write(&broken, "not a zip").unwrap();

        assert_eq!(inspect_epub_metadata(&no_opf), empty_meta());
        assert_eq!(inspect_epub_metadata(&broken), empty_meta());
    }

    // --- 악성 EPUB 방어 (CWE-611, CWE-776, CWE-409) ---

    /// 중첩 엔티티(billion laughs)를 확장하지 않아야 한다.
    #[test]
    fn test_inspect_epub_metadata_does_not_expand_entity_bomb() {
        let mut decls = vec![r#"<!ENTITY a0 "AAAAAAAAAA">"#.to_string()];
        for i in 1..9 {
            let body: String = (0..10).map(|_| format!("&a{};", i - 1)).collect();
            decls.push(format!(r#"<!ENTITY a{i} "{body}">"#));
        }
        let bomb = format!(r#"<?xml version="1.0"?><!DOCTYPE p [{}]><package><title>&a8;</title></package>"#, decls.concat());

        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("bomb.epub");
        write_zip_text(&epub_path, &[("content.opf", &bomb)], false);

        // 539 바이트가 확장되면 약 100GB 다. 빈 meta 로 끝나야 한다.
        assert_eq!(inspect_epub_metadata(&epub_path), empty_meta());
    }

    /// 외부 엔티티(file://)로 로컬 파일을 읽어오지 않아야 한다.
    #[test]
    fn test_inspect_epub_metadata_does_not_resolve_external_entity() {
        let tmp = TempDir::new().unwrap();
        let secret = tmp.path().join("canary.txt");
        fs::write(&secret, "XXE_CANARY_VALUE").unwrap();
        let xxe = format!(r#"<?xml version="1.0"?><!DOCTYPE p [<!ENTITY x SYSTEM "file://{}">]><package><title>&x;</title></package>"#, secret.display());

        let epub_path = tmp.path().join("xxe.epub");
        write_zip_text(&epub_path, &[("content.opf", &xxe)], false);

        assert!(!format!("{:?}", inspect_epub_metadata(&epub_path)).contains("XXE_CANARY_VALUE"));
    }

    /// 압축을 풀면 거대한 OPF 는 읽기 전에 건너뛴다.
    #[test]
    fn test_inspect_epub_metadata_skips_oversized_opf() {
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("big.epub");
        let body = format!("<package>{}</package>", "x".repeat(MAX_OPF_BYTES + 1));
        write_zip_text(&epub_path, &[("content.opf", &body)], true);

        assert!(fs::metadata(&epub_path).unwrap().len() < 100 * 1024, "압축본은 작다");
        assert_eq!(inspect_epub_metadata(&epub_path), empty_meta());
    }

    /// 주석과 처리명령이 섞여도 메타데이터를 읽어야 한다(lxml iter 회귀 방지).
    #[test]
    fn test_inspect_epub_metadata_reads_opf_with_comments() {
        let opf = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/">"#,
            "<!-- 주석 --><?pi 처리명령?>",
            "<metadata><dc:title>주석있음</dc:title><dc:creator>작가</dc:creator></metadata>",
            "</package>"
        );
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("comment.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);
        assert_eq!(meta.title, "주석있음");
        assert_eq!(meta.author, "작가");
    }

    /// Python의 lxml `recover=True`처럼 잘린 OPF에서도 이미 읽을 수 있는 메타데이터를 보존한다.
    #[test]
    fn test_inspect_epub_metadata_recovers_truncated_opf() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>recovered</dc:title><dc:creator>Author</dc:creator></metadata>"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("truncated.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "recovered");
        assert_eq!(meta.author, "Author");
    }

    /// Python lxml은 끝부분의 불완전한 태그를 버리고 앞서 읽은 메타데이터를 보존한다.
    #[test]
    fn test_inspect_epub_metadata_recovers_before_partial_tag() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title><dc:creator"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("partial-tag.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// 복구 중 끝나지 않은 주석은 버리고 앞서 읽은 메타데이터를 보존한다.
    #[test]
    fn test_inspect_epub_metadata_recovers_before_unterminated_comment() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title><!-- unfinished"#;
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("unterminated-comment.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// CDATA와 맞지 않는 닫는 태그가 있어도 XML 구조를 복구해 메타데이터를 읽는다.
    #[test]
    fn test_inspect_epub_metadata_recovers_cdata_and_mismatched_closing_tags() {
        let opf = concat!(r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title><![CDATA[kept]]></dc:title><?pi ok?>"#, "</package>");
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("mismatched-tags.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    /// 열린 태그와 대응하지 않는 종료 태그는 복구 과정에서 무시한다.
    #[test]
    fn test_inspect_epub_metadata_ignores_unmatched_closing_tag() {
        let opf = concat!(r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>kept</dc:title>"#, "</ghost></metadata></package>");
        let tmp = TempDir::new().unwrap();
        let epub_path = tmp.path().join("unmatched-tag.epub");
        write_zip_text(&epub_path, &[("content.opf", opf)], false);

        let meta = inspect_epub_metadata(&epub_path);

        assert_eq!(meta.title, "kept");
    }

    #[test]
    fn test_inspect_txt_content_extracts_snippet_and_hashtags() {
        let tmp = TempDir::new().unwrap();
        let txt_path = tmp.path().join("sample.txt");
        fs::write(&txt_path, format!("#무협 #강호\n{}", "문파와 마교의 대결\n".repeat(45))).unwrap();

        let info = inspect_txt_content(&txt_path);

        assert_eq!(info.hashtags, vec!["무협", "강호"]);
        assert!(info.snippet.contains("문파와 마교"));
        assert!(info.snippet.chars().count() <= 2000);
    }

    #[test]
    fn test_inspect_txt_content_handles_crlf_and_lone_cr_line_endings() {
        let tmp = TempDir::new().unwrap();
        let txt_path = tmp.path().join("line-endings.txt");
        fs::write(&txt_path, "첫째 줄\r\n둘째 줄\r셋째 줄\r\n").unwrap();

        let info = inspect_txt_content(&txt_path);

        assert_eq!(info.snippet, "첫째 줄 둘째 줄 셋째 줄");
    }

    #[test]
    fn test_inspect_txt_content_retries_legacy_decoder_without_hangul() {
        let tmp = TempDir::new().unwrap();
        let txt_path = tmp.path().join("ascii.txt");
        fs::write(&txt_path, "plain ASCII text\r\nsecond line\n").unwrap();

        let info = inspect_txt_content(&txt_path);

        assert_eq!(info.snippet, "plain ASCII text second line");
    }

    #[test]
    fn test_inspect_txt_content_returns_default_for_unreadable_file() {
        let tmp = TempDir::new().unwrap();
        let info = inspect_txt_content(&tmp.path().join("missing.txt"));
        assert_eq!(info.hashtags, Vec::<String>::new());
        assert_eq!(info.snippet, "");
    }

    #[test]
    fn test_title_similarity_and_single_match() {
        let sim = title_similarity("달빛조각사", "달빛 조각사 1권");
        assert!(sim > 0.4, "sim={sim}");
        assert!(is_single_match_valid("달빛조각사", "달빛조각사 1"));
        assert!(!is_single_match_valid("전혀다른책", "달빛조각사"));
    }

    #[test]
    fn test_title_similarity_and_single_match_edge_cases() {
        assert_eq!(title_similarity("", "달빛조각사"), 0.0);
        assert_eq!(title_similarity("a", "b"), 0.0);
        assert!(!is_single_match_valid("", "달빛조각사"));
        assert!(is_single_match_valid("알파 베타", "알파 감마 델타 엡실론 제타 에타 세타 베타"));
        assert!(is_single_match_valid("무협", "현대 무협 소설"));
    }

    #[test]
    fn test_map_category_rules() {
        // (cat_str, raw_title, raw_author, expected)
        let cases: &[(&str, &str, &str, Option<&str>)] = &[
            ("", "", "", None),
            ("", "평범한 제목", "히가시노 게이고", Some("2_소설일본게이고")),
            ("", "평범한 제목", "무라카미 하루키", Some("2_소설일본하루키")),
            ("", "을유세계문학 고전", "", Some("2_을유세계문학전집")),
            ("", "열린책들 세계문학", "", Some("2_열린책들세계문학")),
            ("", "문예세계문학선", "", Some("2_문예세계문학선")),
            ("", "살림지식총서 001", "", Some("4_살림지식총서")),
            ("", "시공디스커버리 총서", "", Some("4_시공디스커버리")),
            ("", "올재 클래식스", "", Some("1_올재")),
            ("", "누워서 읽는 법학", "", Some("4_누워서읽는법학")),
            ("", "이지사이언스", "", Some("5_이지사이언스")),
            ("", "그림으로 읽는 과학", "", Some("5_그림으로_읽는")),
            ("", "손자병법", "", Some("1_동양고전")),
            ("라이트노벨", "", "", Some("3_라이트노벨")),
            ("BL/GL", "", "", Some("9_BLGL")),
            ("그래픽노블", "", "", Some("3_그래픽노블")),
            ("미스터리", "", "", Some("3_스릴러")),
            ("과학소설 > SF", "", "", Some("3_SF")),
            ("판타지소설", "", "", Some("3_판타지")),
            ("무협소설", "", "", Some("3_무협")),
            ("로맨스판타지", "", "", Some("3_여성향")),
            ("에세이", "", "", Some("2_수필서간일기")),
            ("한국소설", "", "", Some("2_소설한국")),
            ("일본소설", "", "", Some("2_소설일본")),
            ("중국소설", "", "", Some("2_소설중국")),
            ("영미소설", "", "", Some("2_소설외국")),
            ("역사소설", "", "", Some("2_소설역사")),
            ("시집", "", "", Some("2_시")),
            ("글쓰기", "", "", Some("2_문학일반서평작법독서")),
            ("문헌정보", "", "", Some("1_문헌서지")),
            ("동양고전", "", "", Some("1_동양고전")),
            ("서양고전", "", "", Some("1_서양고전")),
            ("투자", "", "", Some("6_재테크")),
            ("국내도서 > 경제일반", "", "", Some("4_경제")),
            ("마케팅", "", "", Some("4_경영마케팅")),
            ("국내도서 > 경영", "", "", Some("4_경영마케팅")),
            ("자기계발", "", "", Some("6_처세술리더십창의성")),
            ("심리학", "", "", Some("4_심리학뇌과학")),
            ("철학", "", "", Some("4_철학윤리")),
            ("한국사", "", "", Some("4_역사인물")),
            ("정치학", "", "", Some("4_정치외교군사")),
            ("법학", "", "", Some("4_법")),
            ("사회과학", "", "", Some("4_사회인류")),
            ("종교", "", "", Some("4_종교신화")),
            ("인문학", "", "", Some("4_인문일반논픽션")),
            ("교양과학", "", "", Some("5_수학과학일반")),
            ("미술", "", "", Some("5_미술예술건축")),
            ("음악", "", "", Some("5_음악")),
            ("영화", "", "", Some("5_영화")),
            ("사진", "", "", Some("5_사진영상")),
            ("스포츠", "", "", Some("5_스포츠")),
            ("서브컬쳐", "", "", Some("5_서브컬쳐")),
            ("교육학", "", "", Some("7_교육일반")),
            ("국어", "", "", Some("7_국어교육")),
            ("영어회화", "", "", Some("7_영어교육")),
            ("토익", "", "", Some("7_영어교육")),
            ("일본어", "", "", Some("7_일어교육")),
            ("중국어", "", "", Some("7_중어한자교육")),
            ("HSK", "", "", Some("7_중어한자교육")),
            ("프랑스어", "", "", Some("7_외국어교육")),
            ("언어학", "", "", Some("7_언어일반")),
            ("영어원서", "", "", Some("7_영문일반")),
            ("영문원서", "", "", Some("7_영문일반")),
            ("프로그래밍", "", "", Some("8_IT")),
            ("건강", "", "", Some("8_건강일반")),
            ("요리", "", "", Some("8_요리음료")),
            ("여행", "", "", Some("8_여행")),
            ("공인중개사", "", "", Some("8_공인중개사")),
            ("프라모델", "", "", Some("8_모델링")),
            ("밀리터리", "", "", Some("8_밀리터리")),
            ("성교육", "", "", Some("8_성")),
            ("성의학", "", "", Some("8_성")),
            ("살림", "", "", Some("8_실용의학회계")),
            ("악보", "", "", Some("8_악보")),
            ("어린이", "", "", Some("9_어린이육아")),
            ("청소년문학", "", "", Some("9_청소년")),
            ("사주", "", "", Some("9_역학해몽퍼즐")),
            ("유머", "", "", Some("9_유머")),
            ("명언", "", "", Some("9_격언명언")),
        ];
        for (cat_str, raw_title, raw_author, expected) in cases {
            assert_eq!(map_category(cat_str, raw_title, raw_author).as_deref(), *expected, "case: ({cat_str:?}, {raw_title:?}, {raw_author:?})");
        }
    }

    #[test]
    fn test_clean_empty_parent_dirs_removes_empty_dirs_until_stop_dir() {
        let tmp = TempDir::new().unwrap();
        let stop_dir = tmp.path().join("library");
        let empty_child = stop_dir.join("series").join("volume");
        fs::create_dir_all(&empty_child).unwrap();

        clean_empty_parent_dirs(&empty_child, &stop_dir);

        assert!(stop_dir.exists());
        assert!(!stop_dir.join("series").exists());
    }

    #[test]
    fn test_clean_empty_parent_dirs_stops_when_directory_has_visible_items() {
        let tmp = TempDir::new().unwrap();
        let stop_dir = tmp.path().join("library");
        let child = stop_dir.join("series");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("keep.txt"), "content").unwrap();

        clean_empty_parent_dirs(&child, &stop_dir);

        assert!(child.exists());
    }

    /// rmdir은 숨김 파일까지 봐야 성공한다. 보이는 항목만 세면 코드와 실제 동작이 어긋난다.
    #[test]
    fn test_clean_empty_parent_dirs_keeps_dir_holding_only_hidden_files() {
        let tmp = TempDir::new().unwrap();
        let stop_dir = tmp.path().join("library");
        let child = stop_dir.join("series");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join(".nomedia"), "").unwrap();

        clean_empty_parent_dirs(&child, &stop_dir);

        assert!(child.exists());
    }

    /// stop_dir를 정규화 없이 비교하면 못 알아보고 조상까지 거슬러 올라간다.
    #[test]
    fn test_clean_empty_parent_dirs_never_climbs_past_stop_dir() {
        let tmp = TempDir::new().unwrap();
        let stop_dir = tmp.path().join("library");
        let child = stop_dir.join("series");
        fs::create_dir_all(&child).unwrap();
        // 같은 디렉토리를 '..'를 낀 다른 표기로 넘긴다.
        let unnormalized_stop = stop_dir.join("series").join("..");

        clean_empty_parent_dirs(&child, &unnormalized_stop);

        assert!(stop_dir.exists());
        assert!(tmp.path().exists());
    }

    /// 호출자가 문자열을 넘기면 Path와 절대 같아지지 않아 stop_dir를 지나친다.
    #[test]
    fn test_clean_empty_parent_dirs_accepts_str_stop_dir() {
        let tmp = TempDir::new().unwrap();
        let stop_dir = tmp.path().join("library");
        let child = stop_dir.join("series");
        fs::create_dir_all(&child).unwrap();

        // Python은 str을 넘긴다. Rust에서는 &str 경로도 `AsRef<Path>`로 받는다고 가정한다.
        clean_empty_parent_dirs(&child, Path::new(stop_dir.to_str().unwrap()));

        assert!(stop_dir.exists());
        assert!(!child.exists());
    }

    /// 항상 같은 카테고리를 확신 있게 내놓는 모델 대역.
    struct StubClassifier {
        prediction: Prediction,
    }

    impl StubClassifier {
        fn new(category: &str, confidence: f64) -> Self {
            Self { prediction: Prediction::new(Some(category), confidence, vec![(category.to_string(), confidence)], &format!("모델 확신도 {confidence:.3} -> {category}")) }
        }

        fn confident() -> Self {
            Self::new("3_판타지", 0.97)
        }
    }

    impl Classify for StubClassifier {
        fn is_available(&self) -> bool {
            true
        }

        fn classify_path(&self, _fpath: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            Ok(self.prediction.clone())
        }

        fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            Ok(self.prediction.clone())
        }
    }

    #[test]
    fn test_inspect_txt_content_cp949_and_header_genre() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("test_cp949.txt");
        // encoding_rs::EUC_KR.encode(...) 로 만든 바이트다(backend 에 encoding_rs 의존성을 더하지 않으려고 미리 계산해 두었다).
        let bytes: &[u8] = b"=====================\n[\xc6\xc7\xc5\xb8\xc1\xf6] \xb7\xe9\xc0\xc7 \xbe\xc6\xc0\xcc\xb5\xe9 1\xb1\xc7\n=====================\n\xc7\xc1\xb7\xd1\xb7\xce\xb1\xd7...";
        fs::write(&f, bytes).unwrap();

        let res = inspect_txt_content(&f);
        assert_eq!(res.header_genre.as_deref(), Some("판타지"));
        assert!(res.snippet.contains("룬의 아이들"));
    }

    #[test]
    fn test_title_similarity_and_reliable_edge_cases() {
        // st_words 빈 문자열 (172)
        assert!(!is_title_match_reliable("!@#$%", "제목"));
        // OST 키워드 필터링 (176)
        assert!(!is_title_match_reliable("도깨비", "도깨비 OST (CD)"));
        // 완전 포함 len >= 4 (184)
        assert!(is_title_match_reliable("달빛조각사", "달빛조각사 완결판"));
        // ratio >= 0.7 & sim >= 0.5 (191)
        assert!(is_title_match_reliable("해리포터와 마법사의 돌", "해리포터와 마법사의 돌 특별판"));
        // 단어 1개 일치 및 sim >= 0.6 (193)
        assert!(is_title_match_reliable("화산귀환", "화산귀환 1"));
        // t1 or t2 비어있을 때 (131)
        assert_eq!(title_similarity("", "abc"), 0.0);
        assert_eq!(title_similarity("a", "b"), 0.0);
    }

    #[test]
    fn test_title_match_reliable_ratio() {
        // 189: search_title 단어의 70% 이상 포함 및 sim >= 0.5
        assert!(is_title_match_reliable("해리 포터와 비밀의 방", "해리 포터와 비밀의 방 (전2권)"));
        // 191: ratio < 0.7로 마지막 return False 도달
        assert!(!is_title_match_reliable("해리포터 마법사의 돌", "해리포터 비밀의 방"));
    }

    #[test]
    fn test_clean_filename_author_title_edge_patterns() {
        // 773-774: @author 패턴
        let (a, _t, _s) = clean_filename_to_author_title("@홍길동 멋진 소설.txt");
        assert_eq!(a, "홍길동");

        // 779-780: [저자] 뒤쪽 패턴
        let (a, _t, _s) = clean_filename_to_author_title("멋진 소설 [설봉].txt");
        assert_eq!(a, "설봉");

        // 791-794: -저자 뒤쪽 대시 패턴
        let (a, _t, _s) = clean_filename_to_author_title("멋진 소설-이우혁.txt");
        assert_eq!(a, "이우혁");

        // 805-806: 저자 - 제목 패턴 (끝에 권수 등이 붙어 대시 저자에 안 걸리는 경우)
        let (a, t, _s) = clean_filename_to_author_title("김용 - 사조영웅전 1부.txt");
        assert_eq!(a, "김용");
        assert!(t.contains("사조영웅전"));

        // 826: 구분자 ' - ' 분리 (저자가 앞에 있고 제목에 ' - '가 포함된 경우)
        let (a, _t, s) = clean_filename_to_author_title("[김용] 영웅문 - 사조영웅전 1부.txt");
        assert_eq!(a, "김용");
        assert_eq!(s, "영웅문");
    }

    /// 서점 클라이언트 대역. `BookstoreSearch` 를 구현한다. `failing` 은 조회가 예외를 던지는 서점이다(패닉).
    struct StubBookstore {
        rows: Vec<bookstore::stores::SearchRow>,
        fail: Option<&'static str>,
    }

    impl StubBookstore {
        fn ok(rows: Vec<(&str, &str, &str, &str, &str, &str)>) -> Box<Self> {
            Box::new(Self { rows: rows.into_iter().map(|(a, b, c, d, e, f)| (a.into(), b.into(), c.into(), d.into(), e.into(), f.into())).collect(), fail: None })
        }

        fn failing(message: &'static str) -> Box<Self> {
            Box::new(Self { rows: Vec::new(), fail: Some(message) })
        }
    }

    impl bookstore::stores::BookstoreSearch for StubBookstore {
        fn search(&self, _isbn: &str, title: &str, _author: &str) -> (Vec<bookstore::stores::SearchRow>, String, String) {
            if let Some(message) = self.fail {
                panic!("{message}");
            }
            (self.rows.clone(), title.to_string(), "title".into())
        }
    }

    /// 원래는 Path.rmdir/Path.replace 실패, yes24/aladin/kyobo.search, query_bookstores, classify_file 을 바꿔치기했다.
    /// 지금은 같은 갈래를 `with_fs`(FaultyFs), 서점 대역(pub 필드), 모델 대역으로 만든다. 단언은 그대로다.
    #[test]
    fn test_book_classifier_service_methods_and_cache() {
        use common::faulty_fs::{FaultyFs, other_err};

        let tmp = TempDir::new().unwrap();

        // 1070-1071: clean_empty_parent_dirs 예외 처리 (디렉토리를 못 읽으면 아무것도 지우지 않는다)
        let sub = tmp.path().join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        let busy = FaultyFs::new();
        busy.set_scandir(|_| Some(other_err("Busy")));
        clean_empty_parent_dirs_with(&sub, tmp.path(), busy.as_ref());
        assert!(sub.exists());

        // 1098: cache_file default
        let svc_default = BookClassifierService::new(tmp.path(), None, None);
        assert_eq!(svc_default.cache_file(), tmp.path().join("classification_cache.json"));

        // 1110-1116: load_cache 손상 파일 예외 처리
        let corrupted_cache = tmp.path().join("corrupted_cache.json");
        fs::write(&corrupted_cache, "{bad json").unwrap();
        let svc_corrupt = BookClassifierService::new(tmp.path(), Some(&corrupted_cache), None);
        assert!(svc_corrupt.cache().is_empty());

        // 1113: 유효한 캐시 파일 로드 로그
        let valid_cache = tmp.path().join("valid_cache.json");
        fs::write(&valid_cache, json!({"test_item": {"search_title": "삼국지"}}).to_string()).unwrap();
        let svc_valid = BookClassifierService::new(tmp.path(), Some(&valid_cache), None);
        assert_eq!(svc_valid.cache().len(), 1);

        // 1121-1127: save_cache
        let cache_path = tmp.path().join("test_cache.json");
        let mut svc = BookClassifierService::new(tmp.path(), Some(&cache_path), None).with_delay(0.0);
        svc.set_cache(json!({
            "existing_key": {
                "search_title": "삼국지",
                "author": "나관중",
                "title": "삼국지",
                "status": "moved",
                "yes24": {"cat": "역사소설", "mapped": "2_소설한국", "title": "삼국지"}
            }
        }));
        svc.save_cache();
        assert!(cache_path.exists());

        // 1132-1135: build_title_index
        svc.build_title_index();
        assert!(svc.title_cache().contains_key("삼국지"));

        // save_cache 예외 처리 (1127): 이름 바꾸기가 실패해도 죽지 않고, 원래 캐시 파일은 그대로다.
        let before = fs::read(&cache_path).unwrap();
        let denied = FaultyFs::new();
        denied.set_rename(|_, _| Some(other_err("Permission Denied")));
        svc = svc.with_fs(denied);
        svc.set_cache(json!({}));
        svc.save_cache();
        assert_eq!(fs::read(&cache_path).unwrap(), before);
        svc.set_cache(serde_json::from_slice(&before).unwrap());
        svc = svc.with_fs(Arc::new(sysdeps::RealFs));

        // 1138-1162: query_bookstores & _do_query 예외
        svc.yes24 = StubBookstore::ok(vec![("삼국지", "나관중", "소설", "url", "", "")]);
        svc.aladin = StubBookstore::failing("Timeout");
        svc.kyobo = StubBookstore::ok(vec![]);
        let (y_res, a_res, k_res) = svc.query_bookstores("삼국지", "나관중", "삼국지", "");
        assert_eq!(y_res["title"], "삼국지");
        assert_eq!(a_res["cat"], "");
        assert_eq!(k_res["cat"], "");

        // 1179-1180: rel_path ValueError (fpath가 source_dir 하위가 아닐 때)
        let f_outside = tmp.path().join("outside.txt");
        fs::write(&f_outside, "내용").unwrap();
        let other_src = PathBuf::from("/some/nonexistent/source_dir");
        svc.classify_file(&f_outside, &other_src, ClassifyOptions { use_bookstore: false, ..Default::default() });

        // 1186-1201: title_cache hit
        let f_samguk = tmp.path().join("나관중 - 삼국지 1권.txt");
        fs::write(&f_samguk, "유비 관우 장비 도원결의").unwrap();
        let (_cat, _meth, _reas, entry) = svc.classify_file(&f_samguk, tmp.path(), ClassifyOptions { use_bookstore: false, ..Default::default() });
        assert!(entry.is_some());

        // 1205: cache_only
        let f_uncached = tmp.path().join("미캐시책.txt");
        fs::write(&f_uncached, "내용").unwrap();
        let (_cat_co, m_co, _r_co, _) = svc.classify_file(&f_uncached, tmp.path(), ClassifyOptions { cache_only: true, use_bookstore: true, ..Default::default() });
        assert_eq!(m_co, "cache_only_skip");

        // 1209: use_bookstore=True (query_bookstores 호출)
        svc.classify_file(&f_uncached, tmp.path(), ClassifyOptions { use_bookstore: true, ..Default::default() });
    }
    #[test]
    fn test_map_category_and_effective_filename_branches() {
        // 936: cat_str="거시경제 > 총론" -> leaf_cat="총론" (933 패스, 936 매칭)
        assert_eq!(map_category("거시경제 > 총론", "제목", "저자").as_deref(), Some("4_경제"));
        // 994: 영어, 영문법 (969의 영어회화 등이 없는 카테고리)
        assert_eq!(map_category("기타어학 > 기초 영문법", "제목", "저자").as_deref(), Some("7_영어교육"));
        // 1034: cat_str="지역경제 > 총론" -> 933/936 패스 후 1034 경제 폴백 매칭
        assert_eq!(map_category("지역경제 > 총론", "제목", "저자").as_deref(), Some("4_경제"));

        let tmp = TempDir::new().unwrap();
        let parent_dir = tmp.path().join("작품");
        fs::create_dir_all(&parent_dir).unwrap();
        let f = parent_dir.join("[저자] 제목 1권.txt");
        fs::write(&f, "").unwrap();
        assert_eq!(get_effective_filename(&f, tmp.path()), "[저자] 제목 1권.txt");
    }

    #[test]
    fn map_category_does_not_classify_a_poetry_label_that_also_names_fiction() {
        assert_eq!(map_category("한국시 소설", "", ""), None);
    }

    // ---------------------------------------------------------------------------
    // 판정 결합 — 모델이 먼저, 못 하면 서점
    // ---------------------------------------------------------------------------

    #[test]
    fn test_bookstore_decider_default_delegates_to_original() {
        struct DefaultDecider;
        impl classifier::book_classifier::BookstoreDecider for DefaultDecider {}

        let mut entry = json!({"before": true});
        let vote = DefaultDecider.decide(&mut entry, false, &|entry, trust| {
            entry["delegated"] = json!(trust);
            (Some("3_SF".to_string()), "test".to_string(), "delegated".to_string())
        });

        assert_eq!(vote, (Some("3_SF".to_string()), "test".to_string(), "delegated".to_string()));
        assert_eq!(entry["delegated"], false);
    }

    #[test]
    fn test_book_classifier_service_exposes_roots_cache_and_policy() {
        let tmp = TempDir::new().unwrap();
        let custom_cache = tmp.path().join("cache.json");
        let policy = classifier::bookstore_policy::BookstorePolicy { override_below: 0.42, ..Default::default() };
        let service = BookClassifierService::new(tmp.path(), Some(&custom_cache), Some(Arc::new(StubClassifier::confident()))).with_bookstore_policy(policy.clone());

        assert_eq!(service.library_root(), tmp.path());
        assert_eq!(service.classifier_library_root(), tmp.path());
        assert_eq!(service.cache_file(), custom_cache);
        assert_eq!(service.bookstore_policy(), &policy);
    }

    fn service(tmp: &TempDir, classifier: Arc<dyn Classify>) -> BookClassifierService {
        BookClassifierService::new(tmp.path(), Some(&tmp.path().join("cache.json")), Some(classifier))
    }

    /// 확신이 모자라 판정을 거부하는 모델 대역.
    struct RefusingClassifier;

    impl Classify for RefusingClassifier {
        fn is_available(&self) -> bool {
            true
        }

        fn classify_path(&self, _fpath: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            Ok(Prediction::new(None, 0.42, vec![("3_판타지".to_string(), 0.42)], "확신도 0.420 < 임계값 0.90 이라 판정하지 않음"))
        }

        fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            self.classify_path(Path::new(""), None)
        }
    }

    /// `_entry(yes24=..., aladin=..., kyobo=...)` 대응. 주어진 서점만 덮어쓴다.
    fn entry(stores: &[(&str, Value)]) -> Value {
        let mut base = json!({"search_title": "달빛조각사", "yes24": {}, "aladin": {}, "kyobo": {}});
        for (k, v) in stores {
            base[*k] = v.clone();
        }
        base
    }

    fn hit(mapped: &str, title: &str) -> Value {
        json!({"mapped": mapped, "title": title})
    }

    #[test]
    fn test_decide_prefers_the_model_over_bookstores() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(StubClassifier::new("3_판타지", 0.97)));
        let mut e = entry(&[("yes24", hit("2_소설외국", "달빛조각사")), ("aladin", hit("2_소설외국", "달빛조각사"))]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());
        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("3_판타지"), "model"));
        assert!(d.reason.contains("모델 확신도"));
    }

    #[test]
    fn test_decide_falls_back_to_bookstore_majority_when_the_model_refuses() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[("yes24", hit("2_소설외국", "달빛조각사")), ("aladin", hit("2_소설외국", "달빛조각사")), ("kyobo", hit("3_판타지", "달빛조각사"))]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());
        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("2_소설외국"), "bookstore_majority"));
        // 모델이 왜 못 했는지도 근거에 남아야 한다
        assert!(d.reason.contains("확신도"));
    }

    #[test]
    fn test_decide_takes_a_single_bookstore_only_when_trusted() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[("yes24", hit("3_무협", "달빛조각사"))]);

        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions { trust_single_match: true });
        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("3_무협"), "bookstore_single"));

        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions { trust_single_match: false });
        assert_eq!(d.category, None);
        assert_eq!(d.method, "conflict");
    }

    #[test]
    fn test_decide_reports_conflict_when_two_bookstores_disagree() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[("yes24", hit("3_무협", "달빛조각사")), ("aladin", hit("3_판타지", "달빛조각사"))]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions { trust_single_match: true });
        assert_eq!(d.category, None);
        assert_eq!(d.method, "conflict");
        assert!(d.reason.contains("갈림"));
    }

    #[test]
    fn test_decide_ignores_bookstore_hits_whose_title_does_not_match() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[("yes24", hit("3_무협", "전혀 다른 책 제목")), ("aladin", hit("3_무협", "또 다른 책"))]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());
        assert_eq!(d.category, None);
        assert_eq!(d.method, "not_found");
    }

    /// 갈리면(target=None) 버리던 득표를 entry에 남겨야 화면이 후보 2개를 보여줄 수 있다.
    #[test]
    fn test_decide_by_bookstore_records_top_two_votes_on_conflict() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[("yes24", hit("3_무협", "달빛조각사")), ("aladin", hit("3_판타지", "달빛조각사"))]);

        let (cat, method, _) = svc._decide_by_bookstore(&mut e);

        assert_eq!(cat, None);
        assert_eq!(method, "conflict");
        assert_eq!(e["bookstore_candidates"], json!([["3_무협", 1], ["3_판타지", 1]]));
    }

    /// 득표가 아예 없어도 키는 있어야 호출자가 .get() 없이 읽다가 깨지지 않는다.
    #[test]
    fn test_decide_by_bookstore_records_empty_candidates_when_not_found() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(RefusingClassifier));
        let mut e = entry(&[]);

        let (cat, method, _) = svc._decide_by_bookstore(&mut e);

        assert_eq!(cat, None);
        assert_eq!(method, "not_found");
        assert_eq!(e["bookstore_candidates"], json!([]));
    }

    #[test]
    fn test_decide_survives_a_model_that_raises() {
        struct BrokenClassifier;

        impl Classify for BrokenClassifier {
            fn is_available(&self) -> bool {
                true
            }

            fn classify_path(&self, _fpath: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
                Err("모델 폭발".to_string())
            }

            fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
                Err("모델 폭발".to_string())
            }
        }

        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(BrokenClassifier));
        let mut e = entry(&[("yes24", hit("3_무협", "달빛조각사")), ("aladin", hit("3_무협", "달빛조각사"))]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());
        // 모델이 터져도 서점 경로로 답한다
        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("3_무협"), "bookstore_majority"));
        assert!(d.reason.contains("모델 판정 실패"));
    }

    #[test]
    fn test_decide_returns_nothing_when_there_is_no_model_and_no_bookstore() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(BookCategoryClassifier::new(None, ClassifierOptions::default())));
        let mut e = entry(&[]);
        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());
        assert_eq!(d.category, None);
        assert_eq!(d.method, "not_found");
        assert!(d.reason.contains("모델 파일이 없어"));
    }

    /// 등급을 매기려면 모델 점수가 밖으로 나와야 한다.
    /// 원래는 `_decide_by_model`/`_decide_by_bookstore`/`query_bookstores` 를 바꿔치기했다. 지금은 모델 대역이 (3_SF, 0.87)을 돌려준다.
    #[test]
    fn test_classify_file_exposes_model_confidence() {
        let tmp = TempDir::new().unwrap();
        // Policy: override_below = 0.5, prefers_bookstore(confidence) = confidence < override_below
        let mut svc = BookClassifierService::new(tmp.path(), None, Some(Arc::new(StubClassifier::new("3_SF", 0.87))))
            .with_delay(0.0)
            .with_bookstore_policy(BookstorePolicy { override_below: 0.5, ..Default::default() });
        svc.yes24 = StubBookstore::ok(vec![]);
        svc.aladin = StubBookstore::ok(vec![]);
        svc.kyobo = StubBookstore::ok(vec![]);
        let target = tmp.path().join("[저자] 제목.epub");
        fs::write(&target, "x").unwrap();

        let (_cat, _method, _reason, entry) = svc.classify_file(&target, tmp.path(), ClassifyOptions::default());
        let entry = entry.unwrap();

        assert_eq!(entry["confidence"], json!(0.87));
        assert_eq!(entry["model_category"], "3_SF");
    }
    // ---------------------------------------------------------------------------
    // 확신도 사각지대 — 모델이 답은 했지만 '확실'이라 부를 만큼은 아닌 구간
    // ---------------------------------------------------------------------------

    /// 확신도가 신뢰 기준 미만이면 서점 다수결이 이긴다.
    ///
    /// 예전에는 이 구간에서 서점을 세지도 않고 모델 답을 돌려줬다. 그런데 화면은
    /// 그 답을 '확실'로 인정하지 않아 목적지가 빈 채로 나갔다. 서점 3곳이 한목소리로
    /// 말해도 반영되지 않았다.
    #[test]
    fn test_decide_takes_bookstore_majority_when_the_model_is_not_confident_enough() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(StubClassifier::new("2_수필서간일기", 0.076)));
        let mut e = entry(&[("yes24", hit("2_소설외국", "달빛조각사")), ("aladin", hit("2_소설외국", "달빛조각사")), ("kyobo", hit("2_소설외국", "달빛조각사"))]);

        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());

        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("2_소설외국"), "bookstore_majority"));
        assert_eq!(e["bookstore_candidates"], json!([["2_소설외국", 3]]));
        // 모델 답도 근거에 남아야 관리자가 왜 갈렸는지 안다
        assert_eq!(d.model_category.as_deref(), Some("2_수필서간일기"));
        assert_eq!(d.confidence, Some(0.076));
    }

    /// 다수결이 안 서도 득표는 남긴다. 화면이 후보로 보여줄 수 있어야 한다.
    #[test]
    fn test_decide_records_bookstore_votes_even_when_they_lose() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(StubClassifier::new("2_수필서간일기", 0.076)));
        let mut e = entry(&[("yes24", hit("2_소설외국", "달빛조각사"))]);

        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());

        // 한 곳뿐이라 다수결이 아니다. 모델 답이 그대로 남는다.
        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("2_수필서간일기"), "model"));
        assert_eq!(e["bookstore_candidates"], json!([["2_소설외국", 1]]));
    }

    /// 신뢰 기준을 넘긴 모델은 예전대로 먼저다. 서점을 세는 비용도 안 낸다.
    #[test]
    fn test_decide_keeps_a_confident_model_without_consulting_bookstores() {
        let tmp = TempDir::new().unwrap();
        let svc = service(&tmp, Arc::new(StubClassifier::new("3_판타지", 0.97)));
        let mut e = entry(&[("yes24", hit("2_소설외국", "달빛조각사")), ("aladin", hit("2_소설외국", "달빛조각사"))]);

        let d = svc._decide(None, "달빛조각사.txt", &mut e, DecideOptions::default());

        assert_eq!((d.category.as_deref(), d.method.as_str()), (Some("3_판타지"), "model"));
        assert!(e.get("bookstore_candidates").is_none());
    }
}

// `tests/rust/classifier/book_classifier_parity.rs`에서 옮긴 테스트.
mod book_classifier_parity {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use classifier::book_classifier::*;

    use classifier::model::Prediction;
    use classifier::{BookClassifierApi, Classify};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    fn fixture() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/book_classifier_parity.json");
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn s(v: &Value) -> &str {
        v.as_str().unwrap()
    }

    #[test]
    fn map_category_matches_python() {
        let f = fixture();
        let mut bad = Vec::new();
        for c in f["map_category"].as_array().unwrap() {
            let got = map_category(s(&c["cat"]), s(&c["title"]), s(&c["author"]));
            if got.as_deref() != c["expected"].as_str() {
                bad.push(format!("{:?} -> rust {got:?} python {}", c["cat"], c["expected"]));
            }
        }
        assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    }

    #[test]
    fn filename_parsing_matches_python() {
        let f = fixture();
        let mut bad = Vec::new();
        for c in f["filenames"].as_array().unwrap() {
            let name = s(&c["name"]);
            let (a, t, st) = clean_filename_to_author_title(name);
            let want: Vec<&str> = c["clean"].as_array().unwrap().iter().map(s).collect();
            if [a.as_str(), t.as_str(), st.as_str()] != want[..] {
                bad.push(format!("clean {name:?}: rust {:?} python {want:?}", (&a, &t, &st)));
            }
        }
        assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    }

    #[test]
    fn title_matching_matches_python() {
        let f = fixture();
        let mut bad = Vec::new();
        for c in f["titles"].as_array().unwrap() {
            let (a, b) = (s(&c["a"]), s(&c["b"]));
            let sim = title_similarity(a, b);
            if (sim - c["sim"].as_f64().unwrap()).abs() > 1e-12 || is_single_match_valid(a, b) != c["single"] || is_title_match_reliable(a, b) != c["reliable"] {
                bad.push(format!("{a:?} / {b:?}: rust sim {sim} python {}", c["sim"]));
            }
        }
        assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    }

    #[test]
    fn effective_filename_matches_python() {
        for c in fixture()["effective"].as_array().unwrap() {
            let got = get_effective_filename(Path::new(s(&c["path"])), Path::new(s(&c["target"])));
            assert_eq!(got, s(&c["expected"]), "{}", c["path"]);
        }
    }

    // ---------------------------------------------------------------------------
    // 보안과 경계
    // ---------------------------------------------------------------------------

    struct Fixed(Prediction);

    impl Classify for Fixed {
        fn is_available(&self) -> bool {
            true
        }

        fn classify_path(&self, _p: &Path, _m: Option<f64>) -> Result<Prediction, String> {
            Ok(self.0.clone())
        }

        fn classify_document(&self, _d: &Value, _m: Option<f64>) -> Result<Prediction, String> {
            Ok(self.0.clone())
        }

        fn min_confidence(&self) -> Option<f64> {
            Some(0.056)
        }

        fn expected_accuracy(&self, confidence: f64) -> Option<f64> {
            Some(confidence * 2.0)
        }
    }

    fn service(tmp: &TempDir, category: &str, confidence: f64) -> BookClassifierService {
        BookClassifierService::new(tmp.path(), Some(&tmp.path().join("cache.json")), Some(Arc::new(Fixed(Prediction::new(Some(category), confidence, vec![(category.to_string(), confidence)], "stub")))))
            .with_delay(0.0)
    }

    #[test]
    fn cache_files_are_read_defensively_and_saved_atomically() {
        let tmp = TempDir::new().unwrap();
        // 객체가 아닌 JSON 과 심볼릭 링크는 빈 캐시로 본다.
        let list = tmp.path().join("list.json");
        fs::write(&list, "[1, 2]").unwrap();
        assert!(BookClassifierService::new(tmp.path(), Some(&list), None).cache().is_empty());
        let real = tmp.path().join("real.json");
        fs::write(&real, r#"{"a": {"search_title": "x"}}"#).unwrap();
        let link = tmp.path().join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(BookClassifierService::new(tmp.path(), Some(&link), None).cache().is_empty());

        // 저장은 임시 파일 뒤 교체이고, 임시 파일이 남지 않는다.
        let mut svc = BookClassifierService::new(tmp.path(), Some(&tmp.path().join("c.json")), None);
        svc.set_cache(json!({"k": {"search_title": "삼국지"}}));
        svc.save_cache();
        svc.save_cache();
        let names: Vec<String> = fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        let saved: Value = serde_json::from_slice(&fs::read(tmp.path().join("c.json")).unwrap()).unwrap();
        assert_eq!(saved["k"]["search_title"], "삼국지");
    }

    // ---------------------------------------------------------------------------
    // book_manager 연결: `BookClassifierApi`
    // ---------------------------------------------------------------------------

    #[test]
    fn shared_service_serves_the_book_manager_facade() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("3_판타지").join("[남희성] 달빛조각사 1권.txt");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "본문").unwrap();
        let shared: Arc<dyn BookClassifierApi> = Arc::new(SharedBookClassifier::new(service(&tmp, "3_판타지", 0.9).with_bookstore_policy(BookstorePolicy { override_below: 0.08, ..Default::default() })));

        let (category, method, reason, entry) = shared.classify_file(&f);
        assert_eq!((category.as_deref(), method.as_str()), (Some("3_판타지"), "model"));
        assert!(reason.contains("stub"));
        assert_eq!(entry["model_category"], "3_판타지");
        assert_eq!(entry["confidence"], json!(0.9));

        assert_eq!(shared.override_below(), 0.08);
        assert!(shared.prefers_bookstore(0.05) && !shared.prefers_bookstore(0.5));
        assert_eq!(shared.min_confidence(), Some(0.056));
        assert_eq!(shared.expected_accuracy(Some(0.25)), Some(Some(0.5)));
        assert_eq!(shared.expected_accuracy(None), Some(None));
    }

    #[test]
    fn policy_and_cache_files_reject_symlinks_and_oversized_input() {
        use classifier::bookstore_cache::BookstoreResponseCache;
        let tmp = TempDir::new().unwrap();

        // 정책 파일: 심볼릭 링크와 상한 초과는 기본값으로 본다.
        let real = tmp.path().join("real.json");
        fs::write(&real, json!({"bookstore_override_below": 0.08}).to_string()).unwrap();
        assert_eq!(BookstorePolicy::load(Some(&real)).override_below, 0.08);
        let link = tmp.path().join("bookstore_policy.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(BookstorePolicy::load(Some(&link)).override_below, 0.0);
        let big = tmp.path().join("big.json");
        fs::write(&big, format!("{{\"sample\": 1, \"pad\": \"{}\"}}", "x".repeat(classifier::bookstore_policy::MAX_POLICY_BYTES as usize))).unwrap();
        assert_eq!(BookstorePolicy::load(Some(&big)).sample, 0);
        // 깊이가 한도를 넘는 JSON 도 기본값이다.
        let deep = tmp.path().join("deep.json");
        fs::write(&deep, format!("{}1{}", "[".repeat(5000), "]".repeat(5000))).unwrap();
        assert_eq!(BookstorePolicy::load(Some(&deep)), BookstorePolicy { ..Default::default() });

        // 서점 캐시: 링크를 따라 읽거나 쓰지 않는다.
        let stores = json!({"yes24": {"cat": "소설", "isbn": "9791162203484", "author": "남희성"}, "aladin": {}, "kyobo": {}});
        let target = tmp.path().join("target.jsonl");
        BookstoreResponseCache::new(&target).put(&stores, "달빛조각사", "남희성", "");
        let cache_link = tmp.path().join("bookstore_cache.jsonl");
        std::os::unix::fs::symlink(&target, &cache_link).unwrap();
        let before = fs::read(&target).unwrap();
        let mut cache = BookstoreResponseCache::new(&cache_link);
        assert_eq!(cache.get("", "달빛조각사", "남희성"), None, "링크는 읽지 않는다");
        cache.put(&stores, "다른책", "남희성", "");
        assert_eq!(fs::read(&target).unwrap(), before, "링크로 쓰지 않는다");
    }
}

// `tests/rust/classifier/bookstore_cache.rs`에서 옮긴 테스트.
mod bookstore_cache {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use classifier::book_classifier::BookClassifierService;
    use bookstore::stores::{BookstoreSearch, SearchRow};

    use tempfile::TempDir;

    /// 서점 대역. 호출한 서점 이름을 `calls`에 쌓는다.
    struct Store {
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl BookstoreSearch for Store {
        fn search(&self, _isbn: &str, title: &str, _author: &str) -> (Vec<SearchRow>, String, String) {
            self.calls.lock().unwrap().push(self.name.to_owned());
            (vec![("달빛조각사".into(), "남희성".into(), "소설 > 판타지".into(), "http://x".into(), "http://s".into(), "9791162203484".into())], title.to_owned(), "title".into())
        }
    }

    fn service_with_stores(tmp: &Path, calls: &Arc<Mutex<Vec<String>>>) -> BookClassifierService {
        let mut service = BookClassifierService::new(tmp, Some(&tmp.join("cache.json")), None).with_delay(0.0).with_bookstore_cache_file(&tmp.join("bs.jsonl"));
        service.yes24 = Box::new(Store { name: "y", calls: Arc::clone(calls) });
        service.aladin = Box::new(Store { name: "a", calls: Arc::clone(calls) });
        service.kyobo = Box::new(Store { name: "k", calls: Arc::clone(calls) });
        service
    }

    /// 같은 제목을 두 번 물으면 두 번째는 서점을 안 부른다. 한 건에 3.6초가 걸린다.
    #[test]
    fn test_query_bookstores_uses_the_cache_instead_of_calling_stores_again() {
        let tmp = TempDir::new().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut service = service_with_stores(tmp.path(), &calls);

        let first = service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
        assert_eq!(calls.lock().unwrap().len(), 3);

        let second = service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
        assert_eq!(calls.lock().unwrap().len(), 3);
        assert_eq!(second, first);
    }

    /// 제목 표기가 달라도 ISBN 이 같으면 같은 책이다.
    #[test]
    fn test_query_bookstores_finds_the_cache_by_isbn_when_the_title_differs() {
        let tmp = TempDir::new().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut service = service_with_stores(tmp.path(), &calls);

        service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
        assert_eq!(calls.lock().unwrap().len(), 3);

        service.query_bookstores("달빛 조각사 1부", "남희성", "달빛 조각사 1부", "9791162203484");
        assert_eq!(calls.lock().unwrap().len(), 3);
    }
}

// `tests/rust/classifier/bookstore_policy.rs`에서 옮긴 테스트.
mod bookstore_policy {
    use classifier::book_classifier::BookClassifierService;

    use tempfile::TempDir;

    /// pod 는 책을 /books 로 마운트한다. 설정 기본값 /mnt/data/text 를 쓰면 경로 조회가 죽는다.
    ///
    /// ES 의 file_path 는 라이브러리 루트 기준 상대 경로라, 루트가 어긋나면
    /// 상대 경로를 못 만들고 절대 경로로 질의해 0건이 된다.
    #[test]
    fn test_service_passes_its_library_root_to_the_classifier() {
        let tmp = TempDir::new().unwrap();

        let service = BookClassifierService::new(&tmp.path().join("books"), Some(&tmp.path().join("cache.json")), None);
        assert_eq!(service.classifier_library_root(), tmp.path().join("books"));
    }
}
