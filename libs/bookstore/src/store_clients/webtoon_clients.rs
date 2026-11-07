//! 웹툰 서점(네이버 웹툰·카카오웹툰·봄툰·레진코믹스·탑툰) 검색 구현.

use super::*;

// ---------------------------------------------------------------------------
// 네이버 웹툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버 웹툰. `comic.naver.com/api/search/webtoon` JSON을 직접 부른다.
    NaverWebtoonBookstore, supports_isbn = false
);

const NAVER_WEBTOON_BASE_URL: &str = "https://comic.naver.com";
const NAVER_WEBTOON_API_URL: &str = "https://comic.naver.com/api/search/webtoon";

impl NaverWebtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let Some(resp) = fetch_ok(&*self.session, self.verbose, "네이버 웹툰", NAVER_WEBTOON_API_URL, &params(&[("keyword", keyword), ("page", "1")]), HttpTimeout::from_secs(10, 10), "") else {
            return Vec::new();
        };
        let data = resp.json();
        let webtoons = array_field(&data, "searchList");
        if webtoons.is_empty() {
            if self.verbose {
                log::info!("네이버 웹툰 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for webtoon in webtoons.iter().take(self.max_results) {
            let title_id = webtoon.get("titleId");
            let (title, author) = (str_field(webtoon, "titleName"), str_field(webtoon, "displayAuthor"));
            let category = array_field(webtoon, "genreList").iter().map(|g| str_field(g, "description")).filter(|d| !d.is_empty()).collect::<Vec<_>>().join("/");
            let detail_url = if has_content(title_id) { format!("{NAVER_WEBTOON_BASE_URL}/webtoon/list?titleId={}", json_text(title_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push(SearchRow { title, author, category, detail_url, search_url: search_url.clone(), isbn: String::new() });
            }
        }
        if self.verbose {
            log::info!("네이버 웹툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for NaverWebtoonBookstore {
    bookstore_common!(NAVER_WEBTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 네이버 웹툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_WEBTOON_BASE_URL}/search?keyword={}", quote(keyword))
    }

    // 제목에 저자를 붙인 키워드는 늘 0건이다. 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 카카오웹툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 카카오웹툰. `gateway-kw.kakao.com` JSON을 직접 부른다.
    KakaoWebtoonBookstore, supports_isbn = false
);

const KAKAO_WEBTOON_BASE_URL: &str = "https://webtoon.kakao.com";
const KAKAO_WEBTOON_API_URL: &str = "https://gateway-kw.kakao.com/search/v2/content";
/// authors에는 출판사(PUBLISHER)도 섞여 있다. 사람만 저자로 본다.
const KAKAO_AUTHOR_TYPES: [&str; 3] = ["AUTHOR", "ILLUSTRATOR", "ORIGINAL_STORY"];

impl KakaoWebtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let limit = self.max_results.to_string();
        let Some(resp) =
            fetch_ok(&*self.session, self.verbose, "카카오웹툰", KAKAO_WEBTOON_API_URL, &params(&[("word", keyword), ("offset", "0"), ("limit", &limit)]), HttpTimeout::from_secs(10, 10), "")
        else {
            return Vec::new();
        };
        let data = resp.json();
        let contents = data.get("data").map_or(&[][..], |d| array_field(d, "content"));
        if contents.is_empty() {
            if self.verbose {
                log::info!("카카오웹툰 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for content in contents.iter().take(self.max_results) {
            let content_id = content.get("id");
            let title = str_field(content, "title");
            let names = array_field(content, "authors").iter().filter(|a| KAKAO_AUTHOR_TYPES.contains(&str_field(a, "type").as_str())).map(|a| str_field(a, "name")).filter(|n| !n.is_empty());
            // 글·그림이 같은 사람이면 한 번만 쓴다
            let author = join_unique(names, ", ");
            let seo_id = str_field(content, "seoId");
            let slug = if seo_id.is_empty() { &title } else { &seo_id };
            let detail_url = if has_content(content_id) { format!("{KAKAO_WEBTOON_BASE_URL}/content/{}/{}", quote(slug), json_text(content_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push(SearchRow { title, author, category: str_field(content, "genre"), detail_url, search_url: search_url.clone(), isbn: String::new() });
            }
        }
        if self.verbose {
            log::info!("카카오웹툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for KakaoWebtoonBookstore {
    bookstore_common!(KAKAO_WEBTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 카카오웹툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{KAKAO_WEBTOON_BASE_URL}/search?keyword={}", quote(keyword))
    }

    // 제목에 저자를 붙인 키워드는 0건이다. 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 봄툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 봄툰. `balcony-search-api` JSON을 직접 부른다. 웹소설(novel)이 섞여 오므로 뺀다.
    BomtoonBookstore, supports_isbn = false
);

const BOMTOON_BASE_URL: &str = "https://www.bomtoon.com";
const BOMTOON_API_URL: &str = "https://www.bomtoon.com/api/balcony-search-api/search";
const BOMTOON_NOVEL_TYPE: &str = "novel";
const BOMTOON_CATEGORY_TAG_COUNT: usize = 3;

impl BomtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let query = params(&[
            ("searchText", keyword),
            ("contentsType", "ALL"),
            ("page", "0"),
            ("size", "50"),
            ("device", "WEB"),
            ("isExcludeAdult", "false"),
            ("onlyComplete", "false"),
            ("isSearchTool", "true"),
            ("sortType", "ACCURACY"),
        ]);
        let Some(resp) = fetch_ok(&*self.session, self.verbose, "봄툰", BOMTOON_API_URL, &query, HttpTimeout::from_secs(10, 10), "") else { return Vec::new() };
        let data = resp.json();
        let contents = data.get("data").map_or(&[][..], |d| array_field(d, "results"));

        let mut results = Vec::new();
        for content in contents {
            if content.get("contentsType").and_then(Value::as_str) == Some(BOMTOON_NOVEL_TYPE) {
                continue;
            }
            let alias = str_field(content, "alias");
            let title = str_field(content, "title");
            let author = join_unique(array_field(content, "author").iter().map(|a| json_text(Some(a))), ", ");
            let category = array_field(content, "tags").iter().take(BOMTOON_CATEGORY_TAG_COUNT).map(|t| json_text(Some(t))).collect::<Vec<_>>().join("/");
            let detail_url = if alias.is_empty() { String::new() } else { format!("{BOMTOON_BASE_URL}/detail/{}", quote(&alias)) };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push(SearchRow { title, author, category, detail_url, search_url: search_url.clone(), isbn: String::new() });
            }
            if results.len() >= self.max_results {
                break;
            }
        }
        if self.verbose {
            log::info!("봄툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for BomtoonBookstore {
    bookstore_common!(BOMTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 봄툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{BOMTOON_BASE_URL}/search?q={}&ref=input", quote(keyword))
    }

    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 레진코믹스
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 레진코믹스. `lz-api` advanced-search JSON을 직접 부른다.
    LezhinBookstore, supports_isbn = false
);

const LEZHIN_BASE_URL: &str = "https://www.lezhin.com";
const LEZHIN_API_URL: &str = "https://www.lezhin.com/lz-api/v2/advanced-search";
/// artists에는 출판사(publisher)·레이블(label)도 섞여 있다. 사람만 저자로 본다.
const LEZHIN_AUTHOR_ROLES: [&str; 4] = ["writer", "painter", "scripter", "original"];

impl LezhinBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let limit = self.max_results.to_string();
        let query = params(&[("q", keyword), ("t", "all"), ("order", "popular"), ("offset", "0"), ("limit", &limit)]);
        let Some(resp) = fetch_ok(&*self.session, self.verbose, "레진코믹스", LEZHIN_API_URL, &query, HttpTimeout::from_secs(10, 10), "") else { return Vec::new() };
        let data = resp.json();
        let contents = data.get("data").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);

        let mut results = Vec::new();
        for content in contents.iter().take(self.max_results) {
            let alias = str_field(content, "alias");
            let title = str_field(content, "title");
            let names = array_field(content, "artists").iter().filter(|a| LEZHIN_AUTHOR_ROLES.contains(&str_field(a, "role").as_str())).map(|a| str_field(a, "name")).filter(|n| !n.is_empty());
            let author = join_unique(names, ", ");
            let category = array_field(content, "genres").iter().map(|g| json_text(Some(g))).collect::<Vec<_>>().join("/");
            let detail_url = if alias.is_empty() { String::new() } else { format!("{LEZHIN_BASE_URL}/ko/comic/{}", quote(&alias)) };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push(SearchRow { title, author, category, detail_url, search_url: search_url.clone(), isbn: String::new() });
            }
        }
        if self.verbose {
            log::info!("레진코믹스에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for LezhinBookstore {
    bookstore_common!(LEZHIN_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 레진코믹스 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{LEZHIN_BASE_URL}/ko/search?t=all&q={}", quote(keyword))
    }

    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 탑툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 탑툰. 전체 작품 목록 JSON(comicTotal)을 받아 제목으로 직접 거른다.
    /// JSON 주소에는 내용 해시가 들어 있어 바뀌므로 매번 페이지 HTML에서 찾는다.
    ToptoonBookstore, supports_isbn = false
);

const TOPTOON_BASE_URL: &str = "https://toptoon.com";

impl ToptoonBookstore {
    /// 공백을 없애고 소문자로 바꿔 제목 포함 여부를 비교한다.
    fn normalize(text: &str) -> String {
        text.split_whitespace().collect::<String>().to_lowercase()
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let needle = Self::normalize(keyword);
        if needle.is_empty() {
            return Vec::new();
        }
        let fail = |e: HttpError| {
            if self.verbose {
                log::error!("탑툰 검색 실패: {e}");
            }
            Vec::new()
        };

        let page = match self.session.get_with_timeout(&search_url, &[], HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        if page.status_code != 200 {
            if self.verbose {
                log::warn!("탑툰 페이지 응답 실패: {}", page.status_code);
            }
            return Vec::new();
        }
        let page_text = page.text();
        let Some(found) = rx!(r"https://[a-z0-9.-]+\.cloudfront\.net/production/comicTotal/[0-9a-f]+\.json").find(&page_text) else {
            if self.verbose {
                log::warn!("탑툰 작품 목록 JSON 주소를 찾지 못했습니다");
            }
            return Vec::new();
        };

        let catalog_resp = match self.session.get_with_timeout(found.as_str(), &[], HttpTimeout::from_secs(20, 20)) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        if catalog_resp.status_code != 200 {
            if self.verbose {
                log::warn!("탑툰 작품 목록 응답 실패: {}", catalog_resp.status_code);
            }
            return Vec::new();
        }
        let catalog = catalog_resp.json();

        // 제목이 같은 작품, 제목이 키워드로 시작하는 작품, 나머지 순. 같은 순위는 조회수 순.
        let mut matches: Vec<(u8, f64, &Value)> = Vec::new();
        for comic in catalog.as_array().map_or(&[][..], Vec::as_slice) {
            let Some(meta) = comic.get("meta") else { continue };
            let normalized = Self::normalize(&str_field(meta, "title"));
            if !normalized.contains(&needle) || !has_content(meta.get("comicsListUrl")) {
                continue;
            }
            let rank = if normalized == needle {
                0
            } else if normalized.starts_with(&needle) {
                1
            } else {
                2
            };
            matches.push((rank, -meta.get("viewCount").and_then(Value::as_f64).unwrap_or(0.0), comic));
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));

        let mut results = Vec::new();
        for (_, _, comic) in matches.into_iter().take(self.max_results) {
            let meta = &comic["meta"];
            let author = [meta.pointer("/authorList/authorStr"), meta.pointer("/author/authorString")].into_iter().find(|v| has_content(*v)).map(|v| json_text(v)).unwrap_or_default();
            let category = array_field(meta, "genre").iter().map(|g| str_field(g, "name")).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("/");
            results.push(SearchRow {
                title: str_field(meta, "title"),
                author,
                category,
                detail_url: format!("{TOPTOON_BASE_URL}{}", str_field(meta, "comicsListUrl")),
                search_url: search_url.clone(),
                isbn: String::new(),
            });
        }
        if self.verbose {
            log::info!("탑툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl Bookstore for ToptoonBookstore {
    bookstore_common!(TOPTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 탑툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{TOPTOON_BASE_URL}/hashtag?keyword={}", quote(keyword))
    }

    title_only_search!();
    unused_html_hooks!();
}
