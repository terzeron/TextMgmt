//! 검색(목록·키워드·유사 문서·단건·카테고리 통계)과 카테고리 파일 경로 반복자.

use super::*;

impl ESManager {
    fn normalized_hits(response: &Value, max_score: f64) -> Result<Vec<Hit>, EsError> {
        hits_of(response).iter().map(|hit| Ok(Hit { id: hit_id(hit)?, source: source_of(hit), score: normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false) })).collect()
    }

    /// 질의를 실행하고 점수를 최고점 기준 0~100으로 정규화한다. `max_score`가 없으면 빈 결과다.
    /// 1만 건 이하는 scroll 없이 한 번에, 초과하면 scroll로 읽는다.
    pub fn search(&self, query: &Value, sort: Option<&Value>, max_result_count: i64, source_fields: Option<&[&str]>) -> Result<Vec<Hit>, EsError> {
        if let Some(stub) = &self.search_stub {
            let owned: Option<Vec<String>> = source_fields.map(|fields| fields.iter().map(|f| f.to_string()).collect());
            return stub(query, sort, max_result_count, owned.as_deref());
        }
        let max_result_count = resolve_count(max_result_count);
        // max_result_window 오류를 피하려고 크기를 자른다.
        let size = max_result_count.min(MAX_RESULT_WINDOW);
        log::debug!("search(max_result_count={max_result_count}, size={size}, query='{query}')");

        if max_result_count <= MAX_RESULT_WINDOW {
            let mut body = json!({"query": query, "size": size, "track_scores": true});
            if let Some(sort) = sort {
                body["sort"] = sort.clone();
            }
            if let Some(fields) = source_fields {
                body["_source"] = json!(fields);
            }
            let response = self.es.send(&EsRequest::search(body))?;
            let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
            let mut result = Self::normalized_hits(&response, max_score)?;
            result.truncate(max_result_count as usize);
            return Ok(result);
        }

        let mut scroll_id: Option<String> = None;
        let outcome = self.scroll_search(query, sort, size, max_result_count, &mut scroll_id);
        if let Some(id) = scroll_id
            && let Err(e) = self.es.send(&EsRequest::ClearScroll { scroll_id: id })
        {
            log::debug!("Failed to clear scroll: {e}");
        }
        outcome
    }

    fn scroll_search(&self, query: &Value, sort: Option<&Value>, size: i64, max_result_count: i64, scroll_id: &mut Option<String>) -> Result<Vec<Hit>, EsError> {
        let mut body = json!({"query": query, "track_scores": true, "size": size});
        if let Some(sort) = sort {
            body["sort"] = sort.clone();
        }
        let mut response = self.es.send(&EsRequest::scroll_search(body, "10m"))?;
        if let Some(id) = response["_scroll_id"].as_str() {
            *scroll_id = Some(id.to_string());
        }
        let limit = max_result_count as usize;
        let mut result: Vec<Hit> = Vec::new();
        let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
        for hit in hits_of(&response) {
            result.push(Hit { id: hit_id(hit)?, source: source_of(hit), score: normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false) });
            if result.len() >= limit {
                return Ok(result);
            }
        }
        while !hits_of(&response).is_empty() {
            response = self.es.send(&EsRequest::Scroll { scroll_id: scroll_id.clone().unwrap_or_default(), scroll: "10m".to_string() })?;
            if let Some(id) = response["_scroll_id"].as_str() {
                *scroll_id = Some(id.to_string());
            }
            let Some(max_score) = response["hits"]["max_score"].as_f64() else { return Ok(Vec::new()) };
            for hit in hits_of(&response) {
                result.push(Hit { id: hit_id(hit)?, source: source_of(hit), score: normalize(hit["_score"].as_f64().unwrap_or(0.0), max_score, false) });
                if result.len() >= limit {
                    return Ok(result);
                }
            }
        }
        result.truncate(limit);
        Ok(result)
    }

    /// `(결과, 전체 건수)`를 돌려주는 from/size 페이지 검색. `ref_score`가 0이면 결과 안의 max_score를 기준으로 쓴다.
    pub fn search_paged(&self, query: &Value, sort: Option<&Value>, size: i64, offset: i64, ref_score: f64, source_fields: Option<&[&str]>) -> Result<(Vec<Hit>, i64), EsError> {
        let size = size.min(MAX_RESULT_WINDOW);
        log::debug!("search_paged(size={size}, offset={offset}, query='{query}')");
        let mut body = json!({"query": query, "from": offset, "size": size, "track_scores": true, "track_total_hits": true});
        if let Some(sort) = sort {
            body["sort"] = sort.clone();
        }
        if let Some(fields) = source_fields {
            body["_source"] = json!(fields);
        }
        let response = self.es.send(&EsRequest::search(body))?;
        let total = total_of(&response)?;
        let base_score = if ref_score > 0.0 { ref_score } else { response["hits"]["max_score"].as_f64().unwrap_or(0.0) };
        if base_score <= 0.0 {
            return Ok((Vec::new(), total));
        }
        let result = hits_of(&response)
            .iter()
            .map(|hit| Ok(Hit { id: hit_id(hit)?, source: source_of(hit), score: normalize(hit["_score"].as_f64().unwrap_or(0.0), base_score, true) }))
            .collect::<Result<Vec<Hit>, EsError>>()?;
        Ok((result, total))
    }

    pub fn search_by_title(&self, title: &str, file_type: &str, file_size: i64, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_title(max_result_count={max_result_count}, title='{title}', file_type='{file_type}', file_size={file_size})");
        self.search(&es_client::title_query(title, file_type, file_size), None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_by_category(&self, category: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_category(category='{category}')");
        let sort = json!(["author.keyword", "title.keyword"]);
        self.search(&json!({"term": {"category": category}}), Some(&sort), max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 카테고리 내 문서를 search_after 커서로 한 페이지 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    /// 반환: (문서 목록, 전체 건수, 다음 커서 | None)
    pub fn search_by_category_paged(&self, category: &str, size: i64, search_after: Option<&[Value]>) -> PagedHits {
        log::debug!("search_by_category_paged(category='{category}', size={size}, search_after={search_after:?})");
        let mut body = json!({"query": {"term": {"category": category}}, "sort": Self::category_sort(), "size": size, "track_total_hits": true, "_source": Self::LIST_SOURCE_FIELDS});
        if let Some(after) = search_after.filter(|a| !a.is_empty()) {
            body["search_after"] = json!(after);
        }
        let parsed = self.es.send(&EsRequest::search(body)).and_then(|response| {
            let hits = hits_of(&response);
            let total = total_of(&response)?;
            let result = hits.iter().map(|hit| Ok(Hit { id: hit_id(hit)?, source: source_of(hit), score: 0.0 })).collect::<Result<Vec<Hit>, EsError>>()?;
            // 마지막 페이지(요청 크기 미만)면 다음 커서를 주지 않는다.
            let next = if hits.len() as i64 == size { hits.last().and_then(|hit| hit["sort"].as_array().cloned()) } else { None };
            Ok(PagedHits { hits: result, total, search_after: next })
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_by_category_paged error: {e}");
            PagedHits::default()
        })
    }

    fn cover_type_filter() -> Value {
        json!({"terms": {"file_type": Self::COVER_FILE_TYPES}})
    }

    /// 커버 형식(epub, pdf) 문서가 있는 카테고리별로 (문서 수, 최근 수정 시각, 최근 생성 시각)을 센다. 시각은 epoch 밀리초다.
    /// 오류는 삼키고 빈 결과를 돌려준다.
    pub fn aggregate_cover_categories(&self) -> HashMap<String, (i64, Option<f64>, Option<f64>)> {
        let mut stats = HashMap::new();
        let mut after: Option<Value> = None;
        loop {
            let mut composite = json!({"size": 2000, "sources": [{"category": {"terms": {"field": "category"}}}]});
            if let Some(after) = &after {
                composite["after"] = after.clone();
            }
            let sub_aggs = json!({"updated": {"max": {"field": "updated_time"}}, "created": {"max": {"field": "created_time"}}});
            let body = json!({"size": 0, "query": {"bool": {"filter": [Self::cover_type_filter()]}}, "aggs": {"categories": {"composite": composite, "aggs": sub_aggs}}, "track_total_hits": false});
            let response = match self.es.send(&EsRequest::search(body)) {
                Ok(response) => response,
                Err(e) => {
                    log::error!("aggregate_cover_categories error: {e}");
                    return HashMap::new();
                }
            };
            let aggregation = &response["aggregations"]["categories"];
            let buckets = aggregation["buckets"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            for bucket in buckets {
                let Some(category) = bucket["key"]["category"].as_str() else { continue };
                stats.insert(category.to_string(), (bucket["doc_count"].as_i64().unwrap_or(0), bucket["updated"]["value"].as_f64(), bucket["created"]["value"].as_f64()));
            }
            after = aggregation.get("after_key").filter(|k| has_content(Some(k))).cloned();
            if buckets.is_empty() || after.is_none() {
                return stats;
            }
        }
    }

    pub fn exclude_category_queries(categories: Option<&[&str]>) -> Vec<Value> {
        categories.unwrap_or(&[]).iter().filter(|c| !c.is_empty()).map(|c| category_or_children(c)).collect()
    }

    /// 주어진 카테고리에 직접 속한 커버 형식 파일을 최신순으로 한 페이지 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    /// 반환: ([(문서 id, 문서, 정렬값)], 전체 건수). 전체 건수는 search_after와 무관하다.
    pub fn search_cover_files_paged(&self, categories: &[&str], size: i64, search_after: Option<&[Value]>, exclude_categories: Option<&[&str]>) -> CoverFileHits {
        if categories.is_empty() {
            return CoverFileHits::default();
        }
        log::debug!("search_cover_files_paged(categories={}, size={size}, search_after={search_after:?})", categories.len());
        let mut query = json!({"bool": {"filter": [Self::cover_type_filter(), {"terms": {"category": categories}}]}});
        let excluded = Self::exclude_category_queries(exclude_categories);
        if !excluded.is_empty() {
            query["bool"]["must_not"] = json!(excluded);
        }
        let mut body = json!({"query": query, "sort": Self::latest_sort(), "size": size, "track_total_hits": true, "_source": Self::LIST_SOURCE_FIELDS});
        if let Some(after) = search_after.filter(|a| !a.is_empty()) {
            body["search_after"] = json!(after);
        }
        let parsed = self.es.send(&EsRequest::search(body)).and_then(|response| {
            let hits = hits_of(&response)
                .iter()
                .map(|hit| Ok(CoverFileHit { id: hit_id(hit)?, source: source_of(hit), sort: hit["sort"].as_array().cloned().unwrap_or_default() }))
                .collect::<Result<Vec<_>, EsError>>()?;
            Ok(CoverFileHits { hits, total: total_of(&response)? })
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_cover_files_paged error: {e}");
            CoverFileHits::default()
        })
    }

    /// 작품 디렉토리마다 대표 후보(파일 경로순 앞쪽 몇 권)를 한 번에 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    pub fn search_group_representatives(&self, group_keys: &[&str], exclude_categories: Option<&[&str]>) -> HashMap<String, Vec<(i64, Value)>> {
        if group_keys.is_empty() {
            return HashMap::new();
        }
        let excluded = Self::exclude_category_queries(exclude_categories);
        let mut searches: Vec<Value> = Vec::new();
        for key in group_keys {
            let mut query = json!({"bool": {"filter": [Self::cover_type_filter()], "should": [{"term": {"category": key}}, {"prefix": {"category": format!("{key}/")}}], "minimum_should_match": 1}});
            if !excluded.is_empty() {
                query["bool"]["must_not"] = json!(excluded);
            }
            searches.push(json!({}));
            searches.push(json!({"query": query, "sort": [{"file_path": "asc"}], "size": Self::REPRESENTATIVE_CANDIDATES, "_source": Self::LIST_SOURCE_FIELDS}));
        }
        let response = match self.es.send(&EsRequest::MultiSearch { index: Some(self.index().to_string()), searches }) {
            Ok(response) => response,
            Err(e) => {
                log::error!("search_group_representatives error: {e}");
                return HashMap::new();
            }
        };
        let mut result = HashMap::new();
        for (key, item) in group_keys.iter().zip(response["responses"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
            if item.get("error").is_some() {
                log::error!("search_group_representatives({key}) error: {}", item["error"]);
                continue;
            }
            let rows = hits_of(item).iter().map(|hit| Ok((hit_id(hit)?, source_of(hit)))).collect::<Result<Vec<_>, EsError>>();
            match rows {
                Ok(rows) => {
                    result.insert(key.to_string(), rows);
                }
                Err(e) => log::error!("search_group_representatives({key}) error: {e}"),
            }
        }
        result
    }

    fn keyword_query(keyword: &str) -> Value {
        json!({"bool": {"should": [
            {"match": {"title": {"query": keyword, "boost": 10}}},
            {"match": {"author": {"query": keyword, "boost": 5}}},
            {"match": {"category.nori": {"query": keyword, "boost": 3}}},
            {"match": {"summary": {"query": keyword, "boost": 1}}},
        ], "minimum_should_match": 1}})
    }

    pub fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(max_result_count);
        log::debug!("search_by_keyword(keyword='{keyword}', max_result_count={max_result_count})");
        self.search(&Self::keyword_query(keyword), None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 이름과 추출한 저자만 사용해 유사 파일을 찾는다.
    pub fn search_similar_names(&self, name: &str, author: &str, max_result_count: i64) -> Result<Vec<Hit>, EsError> {
        let wildcard_name = name.replace('\\', "\\\\").replace('*', "\\*").replace('?', "\\?");
        let mut should = vec![json!({"match": {"title": {"query": name, "fuzziness": "AUTO", "boost": 8}}}), json!({"wildcard": {"file_path": {"value": format!("*{wildcard_name}*"), "boost": 6}}})];
        if !author.is_empty() {
            should.push(json!({"match": {"author": {"query": author, "fuzziness": "AUTO", "boost": 10}}}));
        }
        let query = json!({"bool": {"should": should, "minimum_should_match": 1}});
        self.search(&query, None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[&str]>, category: Option<&str>) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("search_by_keyword_paged(keyword='{keyword}', size={size}, offset={offset}, exclude_categories={exclude_categories:?}, category={category:?})");
        let mut query = json!({"bool": {"must": [Self::keyword_query(keyword)]}});
        if let Some(excluded) = exclude_categories.filter(|e| !e.is_empty()) {
            query["bool"]["must_not"] = json!(excluded.iter().map(|c| json!({"prefix": {"category": c}})).collect::<Vec<_>>());
        }
        if let Some(category) = category.filter(|c| !c.is_empty()) {
            query["bool"]["filter"] = json!([{"term": {"category": category}}]);
        }
        self.search_paged(&query, None, size, offset, 0.0, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    /// 커버 형식 문서를 최신순으로 조회한다. 오류는 삼키고 빈 결과를 돌려준다.
    pub fn search_latest_docs(&self, max_result_count: i64, exclude_categories: Option<&[&str]>) -> (Vec<Hit>, i64) {
        log::debug!("search_latest_docs(max_result_count={max_result_count}, exclude_categories={exclude_categories:?})");
        let size = max_result_count.clamp(1, MAX_RESULT_WINDOW);
        let mut query = json!({"bool": {"must": [{"match_all": {}}], "filter": [Self::cover_type_filter()]}});
        let excluded = Self::exclude_category_queries(exclude_categories);
        if !excluded.is_empty() {
            query["bool"]["must_not"] = json!(excluded);
        }
        let body = json!({"query": query, "sort": Self::latest_sort(), "size": size, "track_total_hits": true, "_source": Self::LIST_SOURCE_FIELDS});
        let parsed = self.es.send(&EsRequest::search(body)).and_then(|response| {
            let hits = hits_of(&response).iter().map(|hit| Ok(Hit { id: hit_id(hit)?, source: source_of(hit), score: 0.0 })).collect::<Result<Vec<Hit>, EsError>>()?;
            Ok((hits, total_of(&response)?))
        });
        parsed.unwrap_or_else(|e| {
            log::error!("search_latest_docs error: {e}");
            (Vec::new(), 0)
        })
    }

    // ---- 유사 문서 ----

    pub fn search_similar_docs(&self, args: &SimilarDocsArgs) -> Result<Vec<Hit>, EsError> {
        let max_result_count = resolve_count(args.max_result_count);
        log::debug!("search_similar_docs(title='{}', author='{}', size={}, max_result_count={max_result_count})", args.title, args.author, args.file_size);
        let query = es_client::similar_query(&args.title, &args.author, args.file_size, &args.summary, args.exclude_id);
        self.search(&query, None, max_result_count, Some(&Self::LIST_SOURCE_FIELDS[..]))
    }

    pub fn search_similar_docs_paged(&self, args: &SimilarDocsArgs) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("search_similar_docs_paged(title='{}', author='{}', size={}, offset={})", args.title, args.author, args.size, args.offset);
        let should_clauses = es_client::similar_should_clauses(&args.title, &args.author, args.file_size, &args.summary);
        // exclude_id가 있으면 msearch로 self-score와 본 검색을 한 번의 왕복으로 실행한다.
        if let Some(exclude_id) = args.exclude_id {
            return self.search_paged_with_self_score(&should_clauses, exclude_id, args.size, args.offset);
        }
        let query = json!({"bool": {"should": should_clauses, "minimum_should_match": 1}});
        self.search_paged(&query, None, args.size, args.offset, 0.0, None)
    }

    /// msearch로 self-score 쿼리와 본 검색을 한 번의 왕복으로 실행한다. 점수는 원본 문서의 self-score 기준이다.
    pub fn search_paged_with_self_score(&self, should_clauses: &[Value], exclude_id: i64, size: i64, offset: i64) -> Result<(Vec<Hit>, i64), EsError> {
        log::debug!("_search_paged_with_self_score(exclude_id={exclude_id}, size={size}, offset={offset})");
        let size = size.min(MAX_RESULT_WINDOW);
        // self-score 쿼리: 원본 문서만 매칭한다. 점수만 쓰고 본문을 버리므로 _source를 통째로 끈다.
        let self_score_query = json!({"bool": {"should": should_clauses, "filter": [{"term": {"_id": exclude_id.to_string()}}]}});
        // 본 검색 쿼리: 원본은 제외한다.
        let search_query = json!({"bool": {"should": should_clauses, "minimum_should_match": 1, "must_not": [{"term": {"_id": exclude_id.to_string()}}]}});
        let searches = vec![
            json!({"index": self.index()}),
            json!({"size": 1, "query": self_score_query, "_source": false}),
            json!({"index": self.index()}),
            json!({"size": size, "from": offset, "query": search_query, "track_scores": true, "track_total_hits": true, "_source": Self::LIST_SOURCE_FIELDS}),
        ];
        let response = self.es.send(&EsRequest::MultiSearch { index: None, searches })?;
        let responses = response["responses"].as_array().filter(|r| r.len() >= 2).ok_or_else(|| parse_error("msearch responses가 2개 미만이다"))?;

        let base_score = if responses[0].get("error").is_some() {
            log::error!("msearch self-score query error: {}", responses[0]["error"]);
            0.0
        } else {
            hits_of(&responses[0]).first().and_then(|hit| hit["_score"].as_f64()).unwrap_or(0.0)
        };

        let search_resp = &responses[1];
        if search_resp.get("error").is_some() {
            log::error!("msearch search query error: {}", search_resp["error"]);
            return Ok((Vec::new(), 0));
        }
        let total = total_of(search_resp)?;
        if base_score <= 0.0 {
            return Ok((Vec::new(), total));
        }
        let result = hits_of(search_resp)
            .iter()
            .map(|hit| Ok(Hit { id: hit_id(hit)?, source: source_of(hit), score: normalize(hit["_score"].as_f64().unwrap_or(0.0), base_score, true) }))
            .collect::<Result<Vec<Hit>, EsError>>()?;
        Ok((result, total))
    }

    // ---- 단건 ----

    /// `_source`를 읽는다. 없으면 `None`이다.
    pub(super) fn get_source(&self, id: &str) -> Result<Option<Value>, EsError> {
        match self.es.send(&EsRequest::Get { index: None, id: id.to_string() }) {
            Ok(response) => Ok(Some(source_of(&response))),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 문서 하나의 `_source`. 없으면 `{}`를 반환하고, 그 밖의 ES 오류는 전달한다.
    pub fn search_by_id(&self, doc_id: i64) -> Result<Value, EsError> {
        log::debug!("search_by_id(doc_id={doc_id})");
        self.get_source(&doc_id.to_string()).map(|source| source.unwrap_or_else(|| json!({})))
    }

    /// 카테고리별 문서 수.
    pub fn search_and_aggregate_by_category(&self) -> Result<BTreeMap<String, i64>, EsError> {
        log::debug!("search_and_aggregate_by_category()");
        let body = json!({"size": 1, "aggs": {"unique_values": {"terms": {"field": "category", "size": 10000}}}});
        let result = self.es.send(&EsRequest::search(body))?;
        let buckets = result["aggregations"]["unique_values"]["buckets"].as_array().ok_or_else(|| parse_error("aggregations.unique_values.buckets가 없다"))?;
        let mut counts = BTreeMap::new();
        for bucket in buckets {
            let key = bucket["key"].as_str().map(str::to_string).unwrap_or_else(|| bucket["key"].to_string());
            counts.insert(key, bucket["doc_count"].as_i64().unwrap_or(0));
        }
        Ok(counts)
    }

    fn pdf_stats_of(pdfs: &Value) -> PdfStats {
        let int = |value: &Value| value["value"].as_f64().unwrap_or(0.0) as i64;
        PdfStats { file_count: pdfs["doc_count"].as_i64().unwrap_or(0), page_count: int(&pdfs["total_pages"]), total_file_size: int(&pdfs["total_file_size"]) }
    }

    fn pdf_aggs() -> Value {
        json!({"filter": {"term": {"file_type": "pdf"}}, "aggs": {"total_pages": {"sum": {"field": "page_count"}}, "total_file_size": {"sum": {"field": "file_size"}}}})
    }

    /// 카테고리와 하위 카테고리의 PDF 수, 페이지 합계, 파일 크기를 계산한다.
    pub fn search_category_pdf_stats(&self, category: &str) -> Result<PdfStats, EsError> {
        let query = json!({"bool": {"filter": [category_or_children(category)]}});
        let response = self.es.send(&EsRequest::search(json!({"query": query, "size": 0, "aggs": {"pdfs": Self::pdf_aggs()}})))?;
        Ok(Self::pdf_stats_of(&response["aggregations"]["pdfs"]))
    }

    /// 여러 카테고리와 하위 경로의 PDF 수·페이지 합계를 한 번에 집계한다.
    pub fn search_categories_pdf_stats(&self, categories: &[&str]) -> Result<BTreeMap<String, PdfStats>, EsError> {
        if categories.is_empty() {
            return Ok(BTreeMap::new());
        }
        let filters: Map<String, Value> = categories.iter().map(|c| (c.to_string(), category_or_children(c))).collect();
        let aggs = json!({"by_category": {"filters": {"filters": filters}, "aggs": {"pdfs": Self::pdf_aggs()}}});
        let response = self.es.send(&EsRequest::search(json!({"query": {"match_all": {}}, "size": 0, "aggs": aggs})))?;
        let buckets = response["aggregations"]["by_category"]["buckets"].as_object().ok_or_else(|| parse_error("aggregations.by_category.buckets가 없다"))?;
        Ok(buckets.iter().map(|(category, bucket)| (category.clone(), Self::pdf_stats_of(&bucket["pdfs"]))).collect())
    }

    /// 전체 문서를 (category, file_path, 문서 수)로 훑는다. 검색이 아니라 열거라서 composite 집계를 쓴다.
    /// 같은 경로를 가리키는 중복 문서는 문서 수로 센다.
    pub fn iter_all_category_file_paths(&self, batch_size: usize) -> CategoryFilePaths<'_> {
        log::debug!("iter_all_category_file_paths(batch_size={batch_size})");
        CategoryFilePaths { manager: self, batch_size, after: None, buffer: VecDeque::new(), done: false }
    }
}

/// `iter_all_category_file_paths`가 돌려주는 반복자. 한 번에 한 쪽(batch)씩 읽는다. 오류가 나면 그 오류를 한 번 내고 끝난다.
pub struct CategoryFilePaths<'a> {
    manager: &'a ESManager,
    batch_size: usize,
    after: Option<Value>,
    buffer: VecDeque<(String, String, i64)>,
    done: bool,
}

impl CategoryFilePaths<'_> {
    fn fetch(&mut self) -> Result<(), EsError> {
        let mut composite = json!({
            "size": self.batch_size,
            // missing_bucket을 켜야 category나 file_path가 빠진 문서가 조용히 사라지지 않는다.
            "sources": [{"category": {"terms": {"field": "category", "missing_bucket": true}}}, {"file_path": {"terms": {"field": "file_path", "missing_bucket": true}}}],
        });
        if let Some(after) = &self.after {
            composite["after"] = after.clone();
        }
        let response = self.manager.es.send(&EsRequest::search(json!({"size": 0, "aggs": {"paths": {"composite": composite}}})))?;
        let aggregation = &response["aggregations"]["paths"];
        let buckets = aggregation["buckets"].as_array().ok_or_else(|| parse_error("aggregations.paths.buckets가 없다"))?;
        if buckets.is_empty() {
            self.done = true;
            return Ok(());
        }
        for bucket in buckets {
            let key = &bucket["key"];
            self.buffer.push_back((key["category"].as_str().unwrap_or("").to_string(), key["file_path"].as_str().unwrap_or("").to_string(), bucket["doc_count"].as_i64().unwrap_or(0)));
        }
        self.after = aggregation.get("after_key").filter(|k| has_content(Some(k))).cloned();
        self.done = self.after.is_none();
        Ok(())
    }
}

impl Iterator for CategoryFilePaths<'_> {
    type Item = Result<(String, String, i64), EsError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(row) = self.buffer.pop_front() {
                return Some(Ok(row));
            }
            if self.done {
                return None;
            }
            if let Err(e) = self.fetch() {
                self.done = true;
                return Some(Err(e));
            }
        }
    }
}

/// 한 번에 받는 열거 쪽 크기와 일괄 조회 크기.
const ENUMERATE_PAGE_SIZE: usize = 5000;
const MGET_CHUNK_SIZE: usize = 1000;

fn id_and_path(hit: &Value) -> Option<(i64, String)> {
    let id = hit["_id"].as_str()?.parse().ok()?;
    Some((id, hit["_source"]["file_path"].as_str()?.to_string()))
}

impl ESManager {
    /// 전체 문서의 `(inode, file_path)`. 분류기 학습의 모집단이다. `_id`가 정수가 아니거나 경로가 없는 문서는 건너뛴다.
    pub fn list_ids_and_paths(&self) -> Result<Vec<(i64, String)>, EsError> {
        let mut rows = Vec::new();
        let mut scroll_id: Option<String> = None;
        let outcome = (|| -> Result<(), EsError> {
            let body = json!({"query": {"match_all": {}}, "size": ENUMERATE_PAGE_SIZE, "_source": ["file_path"], "sort": ["_doc"]});
            let mut response = self.es.send(&EsRequest::scroll_search(body, "10m"))?;
            loop {
                scroll_id = response["_scroll_id"].as_str().map(str::to_owned).or(scroll_id.take());
                let hits = hits_of(&response);
                if hits.is_empty() {
                    return Ok(());
                }
                rows.extend(hits.iter().filter_map(id_and_path));
                response = self.es.send(&EsRequest::Scroll { scroll_id: scroll_id.clone().unwrap_or_default(), scroll: "10m".to_string() })?;
            }
        })();
        if let Some(id) = scroll_id {
            let _ = self.es.send(&EsRequest::ClearScroll { scroll_id: id });
        }
        outcome.map(|()| rows)
    }

    /// inode로 문서를 일괄 조회한다. 없는 문서는 뺀다.
    pub fn mget_sources(&self, ids: &[i64], fields: &[&str]) -> Result<Vec<(i64, Value)>, EsError> {
        let mut rows = Vec::new();
        for chunk in ids.chunks(MGET_CHUNK_SIZE) {
            let docs = chunk.iter().map(|id| json!({"_index": self.es.index_name(), "_id": id.to_string()})).collect();
            let response = self.es.send(&EsRequest::MultiGet { docs, source: Some(fields.iter().map(|f| (*f).to_string()).collect()) })?;
            for doc in response["docs"].as_array().into_iter().flatten() {
                if doc["found"].as_bool() == Some(true)
                    && let Some(id) = doc["_id"].as_str().and_then(|s| s.parse().ok())
                {
                    rows.push((id, doc["_source"].clone()));
                }
            }
        }
        Ok(rows)
    }

    /// 같은 ISBN 문서의 `(inode, file_path)`. 최대 `limit`건.
    pub fn search_by_isbn(&self, isbn: &str, limit: usize) -> Result<Vec<(i64, String)>, EsError> {
        let response = self.es.send(&EsRequest::search(json!({"query": {"term": {"isbn": isbn}}, "size": limit, "_source": ["file_path"]})))?;
        Ok(hits_of(&response).iter().filter_map(id_and_path).collect())
    }
}
