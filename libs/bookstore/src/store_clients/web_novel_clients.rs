//! 웹소설 서점(문피아·네이버시리즈·조아라) 검색 구현.

use super::*;

// ---------------------------------------------------------------------------
// 문피아
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 문피아.
    MunpiaBookstore, supports_isbn = false
);

const MUNPIA_BASE_URL: &str = "https://novel.munpia.com";

impl AbstractBookstore for MunpiaBookstore {
    bookstore_common!(MUNPIA_BASE_URL, author_first = true);
    default_keyword_search!();

    /// 공백은 `%20`으로 인코딩한다.
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{MUNPIA_BASE_URL}/page/hd.platinum/view/search/keyword/{}/order/search_result", quote(keyword, ""))
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let doc = Html::parse_document(soup);
        let mut links: Vec<String> = Vec::new();
        for a in doc.select(sel!("div#SEARCH-BOX.section2 > div.ebook_lists > div.article_wrap > div.article > dl.detail > dt > a")) {
            let Some(href) = attr(&a, "href").filter(|h| !h.is_empty()) else { continue };
            let full = if href.starts_with('/') { urljoin(MUNPIA_BASE_URL, href) } else { href.to_owned() };
            if !links.contains(&full) {
                links.push(full);
            }
        }
        if self.verbose {
            log::info!("문피아에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let mut title = String::new();
        let mut author = String::new();
        let mut category = String::new();
        if let Some(content) = meta_content(first_meta(&doc, "property", "og:title").or_else(|| first_meta(&doc, "name", "title"))) {
            title = content.trim().to_owned();
        }
        // 저자: og:description의 '저자명 - 설명'. 없으면 작가 링크
        if let Some(content) = meta_content(first_meta(&doc, "property", "og:description").or_else(|| first_meta(&doc, "name", "description"))) {
            author = content.trim().split(" - ").next().unwrap_or("").to_owned();
        } else if let Some(a) = doc.select(sel!(r#"a[href^="/writer/"]"#)).next() {
            author = text_strip(a);
        }
        if let Some(path) = doc.select(sel!("p.meta-path")).next() {
            category = text_strip(path).split('>').map(str::trim).collect::<Vec<_>>().join(" > ");
        }
        info_value(&title, &author, &category, "")
    }
}

// ---------------------------------------------------------------------------
// 네이버시리즈
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버시리즈. 웹소설과 만화를 함께 검색한다.
    NaverSeriesBookstore, supports_isbn = false
);

const NAVER_SERIES_BASE_URL: &str = "https://series.naver.com";

impl AbstractBookstore for NaverSeriesBookstore {
    bookstore_common!(NAVER_SERIES_BASE_URL, author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_SERIES_BASE_URL}/search/search.series?t=all&q={}", quote(keyword, "/"))
    }

    /// 웹소설은 `nov.title`, 만화는 `com.title` 링크다.
    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let doc = Html::parse_document(soup);
        let mut links: Vec<String> = Vec::new();
        for a in doc.select(sel!(r#"ul.lst_list li a[class^="N=a:"]"#)) {
            if let Some(href) = attr(&a, "href").filter(|h| !h.is_empty()) {
                let full = urljoin(NAVER_SERIES_BASE_URL, href);
                if !links.contains(&full) {
                    links.push(full);
                }
            }
        }
        if self.verbose {
            log::info!("NaverSeries에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let title = title_string(&doc).map(|t| t.trim().to_owned()).unwrap_or_default();
        let mut author = String::new();
        // 작가 정보 컨테이너의 <strong>
        if let Some(container) = doc.select(sel!("div#_otherProductByPerson")).next() {
            let strongs: Vec<ElementRef> = container.select(sel!("strong")).collect();
            if strongs.len() >= 2 {
                author = text_strip(strongs[1]);
            }
        }
        // meta name='description', 그다음 og:description
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "name", "description"))
            && let Some(m) = rx!(r"작가[:：]\s*([^,]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "property", "og:description"))
            && let Some(m) = rx!(r"작가[:：]\s*([^,」]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }
        let category = doc.select(sel!("div#content > ul.end_info > li.info_lst > ul > li span a")).next().map(text_strip).unwrap_or_default();
        info_value(&title, &author, &category, "")
    }
}

// ---------------------------------------------------------------------------
// 조아라
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 조아라 웹소설. 검색 결과를 서버에서 그리지 않아서 `api.joara.com` JSON을 직접 부른다.
    ///
    /// API 키는 환경 변수 `TM_JOARA_API_KEY`에서 읽는다. `with_env`로 조회 함수를 바꿀 수 있다.
    JoaraBookstore, supports_isbn = false, env: EnvSource = Arc::new(|name| std::env::var(name).ok())
);

const JOARA_BASE_URL: &str = "https://www.joara.com";
const JOARA_API_URL: &str = "https://api.joara.com/v2/search/query";

impl JoaraBookstore {
    /// 환경 변수 조회 함수를 바꾼다. 테스트가 프로세스 환경을 건드리지 않고 키를 넣는 자리다.
    pub fn with_env(mut self, env: EnvSource) -> Self {
        self.env = env;
        self
    }

    /// 없으면 빈 문자열이다.
    fn api_key(&self) -> String {
        (self.env)(JOARA_API_KEY_ENV).unwrap_or_default()
    }

    /// 오류 문장에서 키 값을 가린다. 세션 구현이 요청 주소를 오류에 넣을 수 있기 때문이다.
    fn redact(&self, message: &str, key: &str) -> String {
        if key.is_empty() { message.to_owned() } else { message.replace(key, "****") }
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let api_key = self.api_key();
        let max = self.max_results.to_string();
        let device_uid = random_hex32();
        let query = params(&[
            ("api_key", &api_key),
            ("device", "mw"),
            ("devicetoken", "mw"),
            // API가 필수로 요구하지만 값은 검증하지 않는다. 호출마다 새로 만든다.
            ("deviceuid", &device_uid),
            ("query", keyword),
            ("target", "subject"),
            ("page", "1"),
            ("offset", &max),
            ("store", "all"),
            ("orderby", "score"),
        ]);

        let Some(resp) = fetch_ok(&*self.session, self.verbose, "조아라", JOARA_API_URL, &query, HttpTimeout::from_secs(10, 10), &api_key) else { return Vec::new() };
        let data = resp.json();
        // 조아라는 파라미터가 빠져도 HTTP 200을 주고 status로만 실패를 알린다.
        if data.get("status").and_then(Value::as_i64) != Some(1) {
            if self.verbose {
                log::warn!("조아라 API 오류: {}", self.redact(&str_field(&data, "message"), &api_key));
            }
            return Vec::new();
        }
        let books = data.get("data").map_or(&[][..], |d| array_field(d, "list"));
        if books.is_empty() {
            if self.verbose {
                log::info!("조아라 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for book in books.iter().take(self.max_results) {
            let code = if truthy(book.get("book_code")) { json_text(book.get("book_code")) } else { String::new() };
            let (title, author, category) = (str_field(book, "subject"), str_field(book, "member_name"), str_field(book, "category_name"));
            let detail_url = if code.is_empty() { String::new() } else { format!("{JOARA_BASE_URL}/book/{code}") };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, category, detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("조아라에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for JoaraBookstore {
    bookstore_common!(JOARA_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 조아라 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{JOARA_BASE_URL}/search?target=subject&word={}&search=", quote(keyword, "/"))
    }

    // 조아라 검색은 제목(target=subject)만 본다. 저자를 붙이면 늘 0건이라 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}
