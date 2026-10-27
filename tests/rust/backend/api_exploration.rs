//! 원본: tests/test_api_exploration.py
//!
//! 원본은 테스트 파일 안에 `get_top_frequent_words`와 `run_api_exploration_test`를 직접 정의하고,
//! `monkeypatch`로 `Okt`와 `requests`를 가짜로 바꿔서 검증한다. 그래서 대응 로직도 이 파일에 옮겨 두고,
//! 바꿔치기 대신 trait 주입(`Okt`, `HttpGet`)으로 같은 효과를 낸다. 실서버를 부르는 테스트는 없다.
//!
//! 가정한 API (이 파일 안에서 정의한다):
//! - `trait Okt { fn nouns(&self, text: &str) -> Vec<String>; }`
//! - `trait HttpGet { fn get(&self, url: &str) -> Result<Value, RequestException>; }`
//! - `fn get_top_frequent_words(okt: Option<&dyn Okt>, text: &str, top_n: usize) -> Result<Vec<(String, usize)>, RuntimeError>`
//! - `fn run_api_exploration_test(http: &dyn HttpGet, okt: Option<&dyn Okt>)`

use std::fmt;

use serde_json::{Value, json};

const BASE_URL: &str = "http://127.0.0.1:8000";

/// `requests.RequestException`에 대응한다.
#[derive(Debug)]
struct RequestException(String);

impl fmt::Display for RequestException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// `RuntimeError`에 대응한다. konlpy가 없을 때 던진다.
#[derive(Debug)]
struct RuntimeError(String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

trait Okt {
    fn nouns(&self, text: &str) -> Vec<String>;
}

trait HttpGet {
    fn get(&self, url: &str) -> Result<Value, RequestException>;
}

fn get_top_frequent_words(okt: Option<&dyn Okt>, text: &str, top_n: usize) -> Result<Vec<(String, usize)>, RuntimeError> {
    if text.is_empty() {
        return Ok(vec![]);
    }

    let okt = okt.ok_or_else(|| RuntimeError("konlpy가 설치되어 있지 않습니다. 라이브 탐색 실행에는 konlpy가 필요합니다.".to_string()))?;

    let meaningful_words: Vec<String> = okt.nouns(text).into_iter().filter(|w| w.chars().count() > 1).collect();
    if meaningful_words.is_empty() {
        return Ok(vec![]);
    }

    // Counter.most_common처럼 처음 나온 순서를 유지하는 안정 정렬이다.
    let mut counts: Vec<(String, usize)> = Vec::new();
    for word in meaningful_words {
        match counts.iter_mut().find(|(w, _)| *w == word) {
            Some((_, c)) => *c += 1,
            None => counts.push((word, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts.truncate(top_n);
    Ok(counts)
}

fn result_of(v: &Value) -> Vec<Value> {
    v.get("result").and_then(|r| r.as_array()).cloned().unwrap_or_default()
}

fn run_api_exploration_test(http: &dyn HttpGet, okt: Option<&dyn Okt>) {
    println!("🚀 API 탐색 테스트를 시작합니다.");

    println!("\n[1단계] 가장 책이 많은 카테고리 검색 중...");
    let categories = match http.get(&format!("{BASE_URL}/categories")) {
        Ok(v) => {
            let categories = result_of(&v);
            if categories.is_empty() {
                println!("❌ 실패: 카테고리를 찾을 수 없습니다.");
                return;
            }
            categories
        }
        Err(e) => {
            println!("❌ 실패: 카테고리 목록 조회 중 오류 발생: {e}");
            return;
        }
    };

    let mut selected_category: Option<String> = None;
    let mut category_books: Vec<Value> = vec![];
    let mut max_book_count = 0;

    for category in &categories {
        let category = category.as_str().unwrap_or_default().to_string();
        if let Ok(v) = http.get(&format!("{BASE_URL}/categories/{category}")) {
            let books = result_of(&v);
            if books.len() > max_book_count {
                max_book_count = books.len();
                selected_category = Some(category);
                category_books = books;
            }
        }
    }

    match &selected_category {
        Some(c) => println!("✅ 성공: 가장 책이 많은 '{c}' 카테고리에서 책 {}권을 찾았습니다.", category_books.len()),
        None => {
            println!("❌ 실패: 책이 있는 카테고리를 찾지 못했습니다.");
            return;
        }
    }

    println!("\n[2단계 & 3단계] 유사한 책이 있는 책 ID를 찾습니다...");
    let mut similar_books: Vec<Value> = vec![];
    let mut selected_book_id: Option<Value> = None;

    for book in &category_books {
        let book_id = book.get("book_id").cloned().unwrap_or(Value::Null);
        let file_type = book.get("file_type").and_then(|v| v.as_str()).unwrap_or("");
        let is_falsy_id = book_id.is_null() || book_id == json!(0) || book_id == json!("");
        if is_falsy_id || ["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"].contains(&file_type) {
            continue;
        }

        println!("\n  - [2단계] 책 ID '{book_id}'(제목: {}, 타입: {file_type}) 선택.", book.get("title").unwrap_or(&Value::Null));
        println!("  - [3단계] 해당 ID로 유사한 책 검색 시도...");
        match http.get(&format!("{BASE_URL}/similar/{book_id}")) {
            Ok(v) => {
                let result = result_of(&v);
                if !result.is_empty() {
                    selected_book_id = Some(book_id);
                    similar_books = result;
                    println!("✅ [3단계] 성공: 책 ID '{}'에서 유사한 책 {}권을 찾았습니다.", selected_book_id.as_ref().unwrap(), similar_books.len());
                    println!("  - 첫 번째 유사 책: {}", similar_books[0].get("title").unwrap_or(&Value::Null));
                    break;
                } else {
                    println!("  - [3단계] 정보: 유사한 책이 없습니다. 다른 책으로 재시도합니다.");
                }
            }
            Err(e) => {
                println!("  - [3단계] 오류: {e}. 다른 책으로 재시도합니다.");
                continue;
            }
        }
    }

    if selected_book_id.is_none() {
        println!("❌ 실패: 유사한 책을 가진 책을 찾지 못했습니다.");
        return;
    }

    println!("\n[4단계] 유사한 책 제목과 요약에서 키워드를 추출하여 검색 수행 중...");

    let combined_text = similar_books
        .iter()
        .map(|b| format!("{} {}", b.get("title").and_then(|v| v.as_str()).unwrap_or(""), b.get("summary").and_then(|v| v.as_str()).unwrap_or("")).trim().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    println!("  - 수집된 전체 텍스트의 길이: {} 자", combined_text.chars().count());

    let top_keywords = get_top_frequent_words(okt, &combined_text, 10).unwrap_or_default();

    if top_keywords.is_empty() {
        println!("❌ 실패: 유사한 책들의 제목과 요약에서 유효한 키워드를 찾지 못했습니다.");
        return;
    }

    println!("  - 유사 책 제목과 요약에서 찾은 명사 (빈도순):");
    for (i, (word, count)) in top_keywords.iter().enumerate() {
        println!("    {:2}. {word} ({count}회)", i + 1);
    }

    let keyword = &top_keywords[0].0;
    println!("\n  - 검색에 사용할 키워드: '{keyword}'");

    match http.get(&format!("{BASE_URL}/search/{keyword}")) {
        Ok(v) => {
            let search_results = result_of(&v);
            if !search_results.is_empty() {
                println!("✅ 성공: 키워드 '{keyword}'로 검색하여 {}개의 결과를 얻었습니다.", search_results.len());
                println!("  - 첫 번째 결과: {}", search_results[0].get("title").unwrap_or(&Value::Null));
            } else {
                println!("❌ 실패: 키워드 '{keyword}'에 대한 검색 결과가 없습니다.");
                return;
            }
        }
        Err(e) => {
            println!("❌ 실패: 키워드 검색 중 오류 발생: {e}");
            return;
        }
    }

    println!("\n🎉 모든 테스트 시나리오를 성공적으로 완료했습니다.");
}

// ---- tests ----

/// 고정 명사 목록을 돌려주는 가짜 Okt. 원본의 `FakeOkt`에 대응한다.
struct FakeOkt(Vec<&'static str>);

impl Okt for FakeOkt {
    fn nouns(&self, _text: &str) -> Vec<String> {
        self.0.iter().map(|s| s.to_string()).collect()
    }
}

/// 클로저로 응답을 정하는 가짜 `requests.get`.
struct FakeGet<F: Fn(&str) -> Result<Value, RequestException>>(F);

impl<F: Fn(&str) -> Result<Value, RequestException>> HttpGet for FakeGet<F> {
    fn get(&self, url: &str) -> Result<Value, RequestException> {
        (self.0)(url)
    }
}

/// `/similar/1`과 `/search/` 응답만 바꿔 가며 쓰는 공통 가짜 서버.
fn fake_get_with(similar: Value, search: Option<Value>) -> impl Fn(&str) -> Result<Value, RequestException> {
    move |url: &str| {
        if url.ends_with("/categories") {
            return Ok(json!({"result": ["cat1"]}));
        }
        if url.contains("/categories/cat1") {
            return Ok(json!({"result": [{"book_id": 1, "title": "t1", "file_type": "epub"}]}));
        }
        if url.contains("/similar/1") {
            return Ok(similar.clone());
        }
        if url.contains("/search/")
            && let Some(s) = &search
        {
            return Ok(s.clone());
        }
        Ok(json!({"result": []}))
    }
}

#[test]
fn test_get_top_frequent_words_handles_empty() {
    assert_eq!(get_top_frequent_words(None, "", 100).unwrap(), Vec::<(String, usize)>::new());
}

#[test]
fn test_get_top_frequent_words_filters_and_counts() {
    let okt = FakeOkt(vec!["가", "나", "테스트", "테스트", "데이터", "가", "나"]);

    let result = get_top_frequent_words(Some(&okt), "dummy", 10).unwrap();
    assert_eq!(result[0].0, "테스트");
    assert_eq!(result[0].1, 2);
    assert!(result.contains(&("데이터".to_string(), 1)));
    assert!(result.iter().all(|(word, _)| word.chars().count() > 1));
}

#[test]
fn test_run_api_exploration_test_success() {
    let okt = FakeOkt(vec!["테스트", "키워드", "키워드"]);
    let http = FakeGet(fake_get_with(json!({"result": [{"title": "t1", "summary": "s1"}]}), Some(json!({"result": [{"title": "t1"}]}))));

    run_api_exploration_test(&http, Some(&okt));
}

#[test]
fn test_run_api_exploration_test_no_categories() {
    let http = FakeGet(|_url: &str| Ok(json!({"result": []})));

    run_api_exploration_test(&http, None);
}

#[test]
fn test_get_top_frequent_words_returns_empty_when_no_meaningful_words() {
    let okt = FakeOkt(vec!["가", "나", "다"]);
    assert_eq!(get_top_frequent_words(Some(&okt), "dummy", 100).unwrap(), Vec::<(String, usize)>::new());
}

#[test]
fn test_run_api_exploration_test_request_error() {
    let http = FakeGet(|_url: &str| Err(RequestException("boom".to_string())));
    run_api_exploration_test(&http, None);
}

#[test]
fn test_run_api_exploration_test_no_similar_books() {
    let okt = FakeOkt(vec!["키워드", "키워드"]);
    let http = FakeGet(fake_get_with(json!({"result": []}), None));
    run_api_exploration_test(&http, Some(&okt));
}

#[test]
fn test_run_api_exploration_test_no_keywords() {
    let okt = FakeOkt(vec!["가", "나"]);
    let http = FakeGet(fake_get_with(json!({"result": [{"title": "t1", "summary": "s1"}]}), None));
    run_api_exploration_test(&http, Some(&okt));
}

#[test]
fn test_run_api_exploration_test_search_empty() {
    let okt = FakeOkt(vec!["키워드", "키워드"]);
    let http = FakeGet(fake_get_with(json!({"result": [{"title": "t1", "summary": "s1"}]}), Some(json!({"result": []}))));
    run_api_exploration_test(&http, Some(&okt));
}
