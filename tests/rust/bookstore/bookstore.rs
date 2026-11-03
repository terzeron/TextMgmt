//! `bookstore` 최상위 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

// `tests/rust/bookstore/extract.rs`에서 옮긴 테스트.
mod extract {
    //! 추출 함수 테스트. 기대값은 Python 원본(BeautifulSoup `html.parser`)이 같은 HTML에서 만든 `expected.json`이다.

    use bookstore::{Store, strip_trailing_number, truncate_title};
    use serde_json::Value;

    fn expected() -> Value {
        serde_json::from_str(include_str!("expected.json")).unwrap()
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/../../tests/rust/bookstore/fixtures/{name}.html", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn store(name: &str) -> Store {
        Store::from_name(name).unwrap()
    }

    #[test]
    fn search_links_match_python() {
        for (name, case) in expected()["links"].as_object().unwrap() {
            let links = store(case[0].as_str().unwrap()).extract_search_links(&fixture(name));
            let want: Vec<&str> = case[1].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(links, want, "{name}");
        }
    }

    #[test]
    fn book_info_matches_python() {
        for (name, case) in expected()["info"].as_object().unwrap() {
            let info = store(case[0].as_str().unwrap()).extract_book_info(&fixture(name));
            let want = &case[1];
            assert_eq!(
                (info.title.as_str(), info.author.as_str(), info.category.as_str(), info.isbn.as_str()),
                (want["title"].as_str().unwrap(), want["author"].as_str().unwrap(), want["category"].as_str().unwrap(), want["isbn"].as_str().unwrap()),
                "{name}"
            );
        }
    }

    #[test]
    fn search_urls_match_python() {
        for (name, urls) in expected()["urls"].as_object().unwrap() {
            let s = store(name);
            assert_eq!(s.build_search_url("원피스 1 & 저자"), urls[0].as_str().unwrap(), "{name}");
            assert_eq!(s.build_isbn_search_url("9788900000001"), urls[1].as_str().unwrap(), "{name}");
        }
    }

    #[test]
    fn title_helpers_match_python() {
        for case in expected()["truncate"].as_array().unwrap() {
            assert_eq!(truncate_title(case[0].as_str().unwrap()), case[1].as_str().unwrap());
        }
        for case in expected()["strip_num"].as_array().unwrap() {
            assert_eq!(strip_trailing_number(case[0].as_str().unwrap()), case[1].as_str().unwrap());
        }
    }

    #[test]
    fn stores_know_their_names_and_capabilities() {
        assert_eq!(Store::ALL.map(|s| s.name()), ["yes24", "aladin", "kyobo"]);
        assert!(Store::from_name("nope").is_none());
        assert!(Store::Yes24.supports_isbn_search() && Store::Aladin.supports_isbn_search() && Store::Kyobo.supports_isbn_search());
        assert!(Store::Aladin.author_first() && !Store::Yes24.author_first());
        assert!(Store::Kyobo.user_agent().contains("Mozilla") && Store::Yes24.user_agent().starts_with("curl/"));
    }

    #[test]
    fn malformed_html_yields_empty_results_not_a_panic() {
        for s in Store::ALL {
            assert!(s.extract_search_links("").is_empty());
            let info = s.extract_book_info("<<<not html");
            assert_eq!(info.title, "");
        }
    }
}

// `tests/rust/bookstore/search.rs`에서 옮긴 테스트.
mod search {
    use std::collections::HashMap;

    use std::sync::Mutex;

    use bookstore::{Fetcher, Store, search};

    struct Fake {
        pages: HashMap<String, Result<String, String>>,
        requested: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(pages: Vec<(&str, Result<&str, &str>)>) -> Self {
            Fake { pages: pages.into_iter().map(|(u, r)| (u.to_owned(), r.map(str::to_owned).map_err(str::to_owned))).collect(), requested: Mutex::new(Vec::new()) }
        }
    }

    impl Fetcher for Fake {
        fn get(&self, _store: Store, url: &str) -> Result<String, String> {
            self.requested.lock().unwrap().push(url.to_owned());
            self.pages.get(url).cloned().unwrap_or_else(|| Err("404".into()))
        }
    }

    fn detail(title: &str, cat: &str) -> String {
        format!(r#"<h2 class="gd_name">{title}</h2><span class="gd_auth"><a>저자</a></span><div>관련분류 <a href="/product/category/display/1">{cat}</a></div> ISBN13 9788900000001"#)
    }

    const SEARCH: &str = "https://www.yes24.com/Product/Search?domain=ALL&query=";

    fn yes24_results(links: &[&str]) -> String {
        let items: String =
            links.iter().map(|l| format!(r#"<li><div class="itemUnit"><div class="item_info"><div class="info_row info_name"><a class="gd_name" href="{l}">x</a></div></div></div></li>"#)).collect();
        format!(r#"<ul id="yesSchList">{items}</ul>"#)
    }

    #[test]
    fn search_fetches_details_in_order_and_skips_failures_and_empty_pages() {
        let search_url = format!("{SEARCH}%EC%A0%9C%EB%AA%A9");
        let page = yes24_results(&["/product/goods/1", "/product/goods/2", "/product/goods/3", "/product/goods/4", "/product/goods/5"]);
        let fake = Fake::new(vec![
            (&search_url, Ok(&page)),
            ("https://www.yes24.com/product/goods/1", Ok(&detail("첫째", "만화 > 일본 > 소년 > 더깊이"))),
            ("https://www.yes24.com/product/goods/2", Err("timeout")),
            ("https://www.yes24.com/product/goods/3", Ok("   ")),
            ("https://www.yes24.com/product/goods/4", Ok(&detail("넷째", "소설"))),
        ]);
        let out = search(Store::Yes24, &fake, "", "제목", "");
        assert_eq!(out.method, "title");
        assert_eq!(out.keyword, "제목");
        let titles: Vec<_> = out.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["첫째", "넷째"]);
        assert_eq!(out.rows[0].category, "만화 > 일본 > 소년", "카테고리 문자열은 > 기준 세 단계까지");
        assert_eq!(out.rows[0].detail_url, "https://www.yes24.com/product/goods/1");
        assert_eq!(out.rows[0].search_url, search_url);
        assert_eq!(out.rows[0].isbn, "9788900000001");
        // 상세 페이지는 앞의 4개만 요청한다.
        let requested = fake.requested.lock().unwrap();
        assert_eq!(requested.len(), 1 + 4);
        assert!(!requested.iter().any(|u| u.ends_with("/5")));
    }

    #[test]
    fn a_failed_search_page_gives_no_rows() {
        let fake = Fake::new(vec![]);
        let out = search(Store::Yes24, &fake, "", "제목", "");
        assert!(out.rows.is_empty());
        assert_eq!((out.keyword.as_str(), out.method), ("제목", "title"));
    }

    #[test]
    fn title_and_author_are_tried_before_title_only_and_aladin_puts_the_author_first() {
        let fake = Fake::new(vec![]);
        let out = search(Store::Yes24, &fake, "", "제목", "저자");
        assert_eq!((out.keyword.as_str(), out.method), ("제목 저자", "title_author"));
        assert_eq!(fake.requested.lock().unwrap().len(), 2, "조합 검색 뒤 제목만 검색");

        let fake = Fake::new(vec![]);
        let out = search(Store::Aladin, &fake, "", "마왕의 딸 3", "저자");
        assert_eq!((out.keyword.as_str(), out.method), ("저자 마왕의 딸", "title_author"), "알라딘은 권수를 떼고 저자를 앞에 둔다");
    }

    #[test]
    fn the_combined_search_wins_when_it_has_rows() {
        let url = format!("{SEARCH}%EC%A0%9C%EB%AA%A9%20%EC%A0%80%EC%9E%90");
        let page = yes24_results(&["/product/goods/9"]);
        let fake = Fake::new(vec![(&url, Ok(&page)), ("https://www.yes24.com/product/goods/9", Ok(&detail("결과", "만화")))]);
        let out = search(Store::Yes24, &fake, "", "제목", "저자");
        assert_eq!((out.keyword.as_str(), out.method, out.rows.len()), ("제목 저자", "title_author", 1));
    }

    #[test]
    fn isbn_search_comes_first_and_the_subtitle_is_cut() {
        let url = format!("{SEARCH}9788900000001");
        let page = yes24_results(&["/product/goods/7"]);
        let fake = Fake::new(vec![(&url, Ok(&page)), ("https://www.yes24.com/product/goods/7", Ok(&detail("isbn 결과", "만화")))]);
        let out = search(Store::Yes24, &fake, "9788900000001", "제목 - 부제", "저자");
        assert_eq!((out.keyword.as_str(), out.method), ("9788900000001", "isbn"));

        let fake = Fake::new(vec![]);
        let out = search(Store::Yes24, &fake, "", "제목 - 부제", "");
        assert_eq!(out.keyword, "제목", "부제 구분자 왼쪽만 쓴다");
    }

    #[test]
    fn nothing_to_search_for_reports_unknown_or_the_isbn() {
        let fake = Fake::new(vec![]);
        let out = search(Store::Yes24, &fake, "", "", "");
        assert_eq!((out.keyword.as_str(), out.method, out.rows.len()), ("", "unknown", 0));
        assert!(fake.requested.lock().unwrap().is_empty());
        let out = search(Store::Yes24, &fake, "9788900000001", "", "");
        assert_eq!(out.method, "isbn");
    }
}
