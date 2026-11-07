//! 일반 서점(Yes24·알라딘·교보문고·리디북스·네이버쇼핑) 검색 구현.

use super::*;

// ---------------------------------------------------------------------------
// Yes24
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// Yes24. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    Yes24Bookstore, supports_isbn = true
);

impl Yes24Bookstore {
    /// 상세 페이지 카테고리. `관련분류` 아래 카테고리 링크를 `a > b`로 잇는다.
    pub fn extract_yes24_category(&self, markup: &str) -> String {
        Store::Yes24.extract_book_info(markup).category
    }

    /// 상세 페이지 ISBN. ISBN13, ISBN10, ISBN, 숫자 10~13자리 순으로 찾는다.
    pub fn extract_yes24_isbn(&self, markup: &str) -> String {
        Store::Yes24.extract_book_info(markup).isbn
    }

    /// 조회가 실패할 수 있는 markup용. 실패하면 빈 문자열이다.
    pub fn extract_yes24_category_from_source(&self, markup: &dyn MarkupSource) -> String {
        match markup.select(r#"a[href*="/product/category/display/"]"#) {
            Ok(labels) => join_unique(labels.into_iter().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()), " > "),
            Err(e) => {
                log::error!("카테고리 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    /// 조회가 실패할 수 있는 markup용. 실패하면 빈 문자열이다.
    pub fn extract_yes24_isbn_from_source(&self, markup: &dyn MarkupSource) -> String {
        match markup.get_text() {
            Ok(text) => isbn_from_text(&text),
            Err(e) => {
                log::error!("ISBN 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    /// 조회가 실패할 수 있는 markup용. 도중에 실패하면 거기까지 뽑은 부분 결과를 돌려준다.
    pub fn extract_book_info_from_source(&self, markup: &dyn MarkupSource) -> BookInfo {
        let mut title = String::new();
        let mut author = String::new();
        let partial = (|| -> Result<(String, String), String> {
            if let Some(t) = markup.find("h2")? {
                title = t.trim().to_owned();
            }
            if let Some(a) = markup.find("span")? {
                author = a.trim().to_owned();
            }
            Ok((self.extract_yes24_category_from_source(markup), self.extract_yes24_isbn_from_source(markup)))
        })();
        match partial {
            Ok((category, isbn)) => book_info(&title, &author, &category, &isbn),
            Err(e) => {
                log::error!("책 정보 추출 중 오류: {e}");
                book_info(&title, &author, "", "")
            }
        }
    }
}

/// 페이지 글자에서 ISBN을 찾는다. Yes24의 패턴 순서를 따른다.
fn isbn_from_text(text: &str) -> String {
    for pattern in [rx!(r"ISBN13\s*(\d{13})"), rx!(r"ISBN10\s*(\d{10})"), rx!(r"ISBN\s*(\d{10,13})"), rx!(r"(\d{10,13})")] {
        if let Some(c) = pattern.captures(text) {
            return c[1].to_owned();
        }
    }
    String::new()
}

impl Bookstore for Yes24Bookstore {
    bookstore_common!(Store::Yes24.base_url(), author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Yes24.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Yes24.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, markup: &str) -> Vec<String> {
        let links = Store::Yes24.extract_search_links(markup);
        if self.verbose {
            log::info!("yes24에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, markup: &str) -> BookInfo {
        Store::Yes24.extract_book_info(markup)
    }
}

// ---------------------------------------------------------------------------
// 알라딘
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 알라딘. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    AladinBookstore, supports_isbn = true
);

impl AladinBookstore {
    /// 제목 끝의 권수 번호를 뗀다. 예: `마왕의 딸 3` -> `마왕의 딸`.
    pub fn strip_trailing_number(&self, title: &str) -> String {
        crate::strip_trailing_number(title)
    }

    pub fn extract_aladin_isbn(&self, markup: &str) -> String {
        Store::Aladin.extract_book_info(markup).isbn
    }

    /// 조회가 실패할 수 있는 markup용. 실패하면 빈 문자열이다.
    pub fn extract_aladin_isbn_from_source(&self, markup: &dyn MarkupSource) -> String {
        let find = || -> Result<String, String> {
            for text in markup.select("#Ere_prod_allwrap div.Ere_prod_mconts_R > div.conts_info_list1 > ul li")? {
                let text = text.trim();
                if text.starts_with("ISBN")
                    && let Some(m) = rx!(r"(\d{13})").captures(text)
                {
                    return Ok(m[1].to_owned());
                }
            }
            for text in markup.select("#Ere_prod_allwrap > div.Ere_prod_middlewrap div.Ere_prod_mconts_R ul > li")? {
                let text = text.trim();
                if text.contains("ISBN")
                    && let Some(m) = rx!(r"(\d{10,13})").captures(text)
                {
                    return Ok(m[1].to_owned());
                }
            }
            Ok(String::new())
        };
        find().unwrap_or_else(|e| {
            log::error!("ISBN 알라딘 추출 중 오류: {e}");
            String::new()
        })
    }
}

impl Bookstore for AladinBookstore {
    bookstore_common!(Store::Aladin.base_url(), author_first = true);
    default_keyword_search!();

    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        let title = if title.is_empty() { String::new() } else { self.strip_trailing_number(title) };
        default_search(self, isbn, &title, author)
    }

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Aladin.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Aladin.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, markup: &str) -> Vec<String> {
        limited_search_links(Store::Aladin, "알라딘", markup, self.max_results, self.verbose)
    }

    fn extract_book_info(&self, markup: &str) -> BookInfo {
        Store::Aladin.extract_book_info(markup)
    }
}

/// `store`의 검색 결과 링크를 `max_results`개까지 뽑는다. `verbose`면 찾은 개수를 `label`과 함께 남긴다.
fn limited_search_links(store: Store, label: &str, markup: &str, max_results: usize, verbose: bool) -> Vec<String> {
    let mut links = store.extract_search_links(markup);
    links.truncate(max_results);
    if verbose {
        log::info!("{label}에서 {}개의 상세 페이지 링크를 찾았습니다.", links.len());
    }
    links
}

// ---------------------------------------------------------------------------
// 교보문고
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 교보문고. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    KyoboBookstore, supports_isbn = true
);

impl Bookstore for KyoboBookstore {
    bookstore_common!(Store::Kyobo.base_url(), author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Kyobo.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Kyobo.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, markup: &str) -> Vec<String> {
        limited_search_links(Store::Kyobo, "교보문고", markup, self.max_results, self.verbose)
    }

    fn extract_book_info(&self, markup: &str) -> BookInfo {
        Store::Kyobo.extract_book_info(markup)
    }
}

// ---------------------------------------------------------------------------
// 리디북스
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 리디북스. 검색은 `search-api.ridibooks.com` JSON을 직접 부른다. ISBN 검색은 지원하지 않는다.
    RidibooksBookstore, supports_isbn = false, blocked: AtomicBool = AtomicBool::new(false)
);

const RIDI_BASE_URL: &str = "https://ridibooks.com";
const RIDI_API_URL: &str = "https://search-api.ridibooks.com/search";

/// RIDI(Cloudflare)가 자동 요청에 돌려주는 차단 상태. 403은 챌린지 페이지다.
const RIDI_BLOCK_STATUSES: [u16; 3] = [403, 418, 429];

impl RidibooksBookstore {
    /// 첫 `ISBN` 라벨 `div` 다음 형제 `div`에서 13자리 코드를 뽑는다.
    pub fn extract_ridi_isbn(&self, markup: &str) -> String {
        let doc = Html::parse_document(markup);
        for label in doc.select(sel!("div")) {
            if text_strip(label) != "ISBN" {
                continue;
            }
            if let Some(sibling) = next_sibling_div(label) {
                let code = text_strip(sibling);
                if rx!(r"^\d{13}$").is_match(&code) {
                    return code;
                }
            }
        }
        String::new()
    }

    /// 조회가 실패할 수 있는 markup용. 실패하면 빈 문자열이다.
    /// 형제 관계를 알 수 없어서 `find_all("div")` 목록에서 `ISBN` 바로 다음 항목을 본다.
    pub fn extract_ridi_isbn_from_source(&self, markup: &dyn MarkupSource) -> String {
        match markup.find_all("div") {
            Ok(divs) => {
                let texts: Vec<&str> = divs.iter().map(|d| d.trim()).collect();
                texts.windows(2).find(|w| w[0] == "ISBN" && rx!(r"^\d{13}$").is_match(w[1])).map_or_else(String::new, |w| w[1].to_owned())
            }
            Err(e) => {
                log::error!("ISBN 리디북스 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        self.blocked.store(false, Ordering::Relaxed);
        let resp = match self.session.get_with_timeout(RIDI_API_URL, &params(&[("keyword", keyword)]), HttpTimeout::from_secs(10, 10)) {
            Ok(resp) => resp,
            Err(e) => {
                log::warn!("RIDI 검색 실패: {e}");
                return Vec::new();
            }
        };
        if RIDI_BLOCK_STATUSES.contains(&resp.status_code) {
            self.blocked.store(true, Ordering::Relaxed);
            log::warn!("RIDI가 자동 검색을 차단했습니다: {}", resp.status_code);
            return Vec::new();
        }
        if resp.status_code != 200 {
            log::warn!("RIDI API 응답 실패: {}", resp.status_code);
            return Vec::new();
        }
        let data = resp.json();
        let books = array_field(&data, "books");
        if books.is_empty() {
            if self.verbose {
                log::info!("RIDI 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for book in books.iter().take(self.max_results) {
            let book_id = book.get("b_id");
            let (title, author) = (str_field(book, "title"), str_field(book, "author"));
            let pair = |parent: &str, child: &str| -> Option<String> {
                let (parent, child) = (str_field(book, parent), str_field(book, child));
                if !parent.is_empty() && !child.is_empty() {
                    Some(format!("{parent} > {child}"))
                } else if !child.is_empty() {
                    Some(child)
                } else if !parent.is_empty() {
                    Some(parent)
                } else {
                    None
                }
            };
            let paths: Vec<String> = [pair("parent_category_name", "category_name"), pair("parent_category_name2", "category_name2")].into_iter().flatten().collect();
            let detail_url = if has_content(book_id) { format!("{RIDI_BASE_URL}/books/{}", json_text(book_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push(SearchRow { title, author, category: paths.join(" || "), detail_url, search_url: search_url.clone(), isbn: String::new() });
            }
        }
        if self.verbose {
            log::info!("RIDI에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for RidibooksBookstore {
    bookstore_common!(RIDI_BASE_URL, author_first = true);
    api_keyword_search!(search_api);

    fn request_blocked(&self) -> bool {
        self.blocked.load(Ordering::Relaxed)
    }

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{RIDI_BASE_URL}/search?q={}&adult_exclude=n", quote(keyword))
    }

    /// RIDI는 `search_by_keyword`를 대체하므로 쓰이지 않는다.
    fn extract_search_links(&self, _markup: &str) -> Vec<String> {
        Vec::new()
    }

    fn extract_book_info(&self, markup: &str) -> BookInfo {
        let doc = Html::parse_document(markup);
        let (mut title, mut author, mut category) = (String::new(), String::new(), String::new());

        // 제목: og:title 메타 우선, 실패하면 <h1>
        let meta = first_meta(&doc, "property", "og:title").or_else(|| first_meta(&doc, "name", "title"));
        if let Some(content) = meta_content(meta) {
            title = content.trim().to_owned();
        } else if let Some(h1) = doc.select(sel!("h1")).next().map(text_strip).filter(|t| !t.is_empty()) {
            title = h1;
        }

        // 헤더 영역의 저자 (id 대소문자 오타 포함). html5ever는 `<li><li>`를 형제로 펴므로 비어 있지 않은 첫 li를 쓴다.
        let mut header: Vec<ElementRef> = doc.select(sel!("div#ISLANDS__Header ul > li")).collect();
        if header.is_empty() {
            header = doc.select(sel!("div#iSLANDS__Header ul > li")).collect();
        }
        if let Some(raw) = header.into_iter().map(text_strip).find(|t| !t.is_empty()) {
            author = raw.replace("저자", "").trim().to_owned();
        }
        // '저자'로 끝나는 li 텍스트
        if author.is_empty() {
            for li in doc.select(sel!("li")) {
                let txt = text_strip_spaced(li);
                if txt.ends_with("저자") && txt.chars().count() > "저자".chars().count() {
                    let raw = txt.strip_suffix("저자").unwrap_or(&txt);
                    // '외13명' 형태에 공백을 넣는다
                    author = rx!(r"외(\d+명)").replace_all(raw, " 외 ${1}").trim().to_owned();
                    break;
                }
            }
        }
        // /author 링크
        if author.is_empty()
            && let Some(a) = doc.select(sel!(r#"a[href*="/author"]"#)).next()
        {
            author = text_strip(a);
        }
        // 메타 태그
        if author.is_empty()
            && let Some(content) = meta_content(first_meta(&doc, "name", "author").or_else(|| first_meta(&doc, "property", "og:author")))
        {
            author = content.trim().to_owned();
        }
        // meta description의 `저자: 이름, ...`
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "property", "og:description").or_else(|| first_meta(&doc, "name", "description")))
            && let Some(m) = rx!(r"저자[:：]\s*([^,]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }

        // 카테고리: 각 li가 하나의 경로, 다중 경로는 " || "로 잇는다
        let mut paths = Vec::new();
        for li in doc.select(sel!("#books_contents section.detail_body ul li")) {
            let parts: Vec<String> = li.select(sel!("a[href^='/category/']")).map(text_strip).filter(|t| !t.is_empty()).collect();
            if !parts.is_empty() {
                paths.push(parts.join(" > "));
            }
        }
        if !paths.is_empty() {
            category = paths.join(" || ");
        }

        book_info(&title, &author, &category, &self.extract_ridi_isbn(markup))
    }
}

// ---------------------------------------------------------------------------
// 네이버쇼핑
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버쇼핑 도서.
    NaverShoppingBookstore, supports_isbn = false
);

const NAVER_SHOPPING_BASE_URL: &str = "https://search.shopping.naver.com";

impl NaverShoppingBookstore {
    /// 조회가 실패할 수 있는 markup용. 도중에 실패하면 거기까지 뽑은 부분 결과를 돌려준다.
    pub fn extract_book_info_from_source(&self, markup: &dyn MarkupSource) -> BookInfo {
        let find_title = || -> Result<String, String> {
            let found = match markup.find("h2")? {
                Some(t) => Some(t),
                None => markup.find("div")?,
            };
            Ok(found.map(|t| t.trim().to_owned()).unwrap_or_default())
        };
        match find_title() {
            Ok(title) => book_info(&title, "", "", ""),
            Err(e) => {
                log::error!("네이버쇼핑 상세 정보 추출 중 오류: {e}");
                BookInfo::default()
            }
        }
    }
}

impl Bookstore for NaverShoppingBookstore {
    bookstore_common!(NAVER_SHOPPING_BASE_URL, author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_SHOPPING_BASE_URL}/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&query={}&sort=REL", quote(keyword))
    }

    fn extract_search_links(&self, markup: &str) -> Vec<String> {
        let mut links = Vec::new();
        let doc = Html::parse_document(markup);
        // 모든 <script>에서 JSON 텍스트를 찾는다
        let raw = doc.select(sel!("script")).map(text_raw).find(|t| t.trim().starts_with('{') && t.contains("SearchAll"));
        let Some(raw) = raw else {
            if self.verbose {
                log::warn!("네이버쇼핑 JSON 스크립트를 찾을 수 없습니다");
            }
            return links;
        };
        let data: Value = match serde_json::from_str(&raw) {
            Ok(d) => d,
            Err(e) => {
                if self.verbose {
                    log::error!("네이버쇼핑 JSON 파싱 실패: {e}");
                }
                return links;
            }
        };
        // React Query state에서 SearchAll 아이템을 꺼낸다
        let queries = data.pointer("/props/pageProps/dehydratedState/queries").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let mut seen: Vec<&Value> = Vec::new();
        for q in queries {
            let is_search_all = q.get("queryKey").and_then(Value::as_array).and_then(|k| k.first()).is_some_and(|k| k == "SearchAll");
            if !is_search_all {
                continue;
            }
            match q.pointer("/state/data/SearchAll/bookSasResult/itemList") {
                None => {}
                Some(Value::Array(items)) => {
                    for item in items {
                        let id = item.get("id");
                        if let Some(id) = id.filter(|_| has_content(id))
                            && !seen.contains(&id)
                        {
                            seen.push(id);
                            links.push(format!("{NAVER_SHOPPING_BASE_URL}/book/catalog/{}", json_text(Some(id))));
                        }
                    }
                }
                Some(_) => {}
            }
            break;
        }
        if self.verbose {
            log::info!("네이버쇼핑에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    /// CSS 클래스에 해시 접미사가 붙어서 접두사로 찾는다.
    fn extract_book_info(&self, markup: &str) -> BookInfo {
        let doc = Html::parse_document(markup);
        let title_prefix = "bookTitle_book_name__";
        let title = find_by_class_prefix(&doc, sel!("h2"), title_prefix).or_else(|| find_by_class_prefix(&doc, sel!("div"), title_prefix)).map(text_strip).unwrap_or_default();

        // 저자: '저자' 라벨이 있는 info_title 다음의 info_content
        let mut author = String::new();
        for label in doc.select(sel!("div")).filter(|e| has_class_prefix(e, "bookTitle_info_title__")) {
            if text_strip(label) == "저자" {
                let content = label.next_siblings().filter_map(ElementRef::wrap).find(|e| e.value().name() == "div" && has_class_prefix(e, "bookTitle_info_content__"));
                if let Some(content) = content {
                    author = text_strip(content);
                }
                break;
            }
        }
        if author.is_empty() {
            author = find_by_class_prefix(&doc, sel!("div"), "bookTitle_info_content__").map(text_strip).unwrap_or_default();
        }

        let category = find_by_class_prefix(&doc, sel!("div"), "bookCatalogTop_breadcrumb__").map(|e| text_raw(e).split('>').map(str::trim).collect::<Vec<_>>().join(" > ")).unwrap_or_default();
        book_info(&title, &author, &category, "")
    }
}
