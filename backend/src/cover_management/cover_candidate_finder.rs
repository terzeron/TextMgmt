//! 서점 검색으로 표지 후보를 찾고, 후보 정보와 이미지를 캐시에서 읽는다.

use super::*;

impl CoverManagement {
    fn candidate_file(&self, candidate_id: &str) -> Result<PathBuf, CoverError> {
        let valid = candidate_id.len() == 64 && candidate_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(CoverError::new("표지 후보 ID가 올바르지 않습니다"));
        }
        let grouped = self.cache.join(format!("book-{}", &candidate_id[..32])).join(format!("candidate-{candidate_id}.json"));
        Ok(if grouped.is_file() { grouped } else { self.cache.join(format!("candidate-{candidate_id}.json")) })
    }

    /// 후보 기록 파일 경로. ID가 올바르지 않으면 어떤 후보 파일과도 겹치지 않는 경로를 돌려준다.
    pub fn candidate_path(&self, candidate_id: &str) -> PathBuf {
        self.candidate_file(candidate_id).unwrap_or_else(|_| self.cache.join("candidate-invalid.json"))
    }

    pub fn candidate(&self, candidate_id: &str) -> Result<Value, CoverError> {
        self.read_json(&self.candidate_file(candidate_id)?).filter(has_entries).ok_or_else(|| CoverError::new("표지 후보를 찾을 수 없습니다. 후보를 다시 검색하세요"))
    }

    pub(super) fn image_file(&self, candidate_id: &str) -> Result<PathBuf, CoverError> {
        Ok(self.candidate_file(candidate_id)?.parent().unwrap_or(&self.cache).join(format!("image-{candidate_id}.jpg")))
    }

    fn image_info(&self, candidate_id: &str, candidate: &mut Value, data: Vec<u8>) -> Result<Vec<u8>, CoverError> {
        if !has_image_info(candidate) {
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .and_then(|reader| reader.into_dimensions().map_err(std::io::Error::other))
                .map_err(|_| CoverError::io("이미지 형식을 읽을 수 없습니다"))?;
            merge(candidate, &json!({"width": width, "height": height, "byte_size": data.len()}));
            self.try_write(&self.candidate_file(candidate_id)?, &serde_json::to_vec(candidate).map_err(|e| CoverError::io(&e.to_string()))?)?;
        }
        Ok(data)
    }

    /// 캐시한 검색 결과. 성공 결과는 후보 기록이 모두 남아 있어야 쓴다. 오류는 2분, 그 밖은 1시간 뒤 버린다.
    fn cached_search(&self, store: &str, relative: &str, path: &Path) -> Result<Option<Value>, CoverError> {
        if let Some(f) = &self.hooks.cached_search {
            return Ok(f(store, relative));
        }
        let cached = self.read_json(path).filter(has_entries);
        if let Some(mut cached) = cached.clone().filter(|c| c.get("status").and_then(Value::as_str) == Some("success")) {
            let ids: Vec<String> = cached.get("candidates").and_then(Value::as_array).map(|list| list.iter().map(|c| str_field(c, "candidate_id").to_string()).collect()).unwrap_or_default();
            let mut merged: Vec<(usize, Value)> = Vec::new();
            for (index, id) in ids.iter().enumerate() {
                let record_path = self.candidate_file(id)?;
                let Some(mut record) = self.read_json(&record_path).filter(has_entries) else { return Ok(None) };
                let image_path = record_path.parent().unwrap_or(&self.cache).join(format!("image-{id}.jpg"));
                if image_path.is_file() && !has_image_info(&record) {
                    self.image_info(id, &mut record, std::fs::read(&image_path)?)?;
                }
                merged.push((index, record));
            }
            if let Some(list) = cached.get_mut("candidates").and_then(Value::as_array_mut) {
                for (index, record) in merged {
                    for key in IMAGE_INFO_KEYS {
                        if let Some(value) = record.get(key) {
                            list[index][key] = value.clone();
                        }
                    }
                }
            }
            return Ok(Some(cached));
        }
        let is_error = cached.as_ref().is_some_and(|c| c.get("status").and_then(Value::as_str) == Some("error"));
        Ok(self.read_json_within(path, Some(if is_error { 120.0 } else { 3600.0 })).filter(has_entries))
    }

    fn search(&self, store: &str, metadata: &Value) -> Result<Value, SearchError> {
        match &self.hooks.search {
            Some(f) => f(store, metadata),
            None => search_candidates_with(store, metadata, &self.hooks.search_hooks),
        }
    }

    fn fetch_store_bytes(&self, store: &str, url: &str) -> Result<Vec<u8>, CoverError> {
        match &self.hooks.fetch_bytes {
            Some(f) => f(store, url).map_err(|message| CoverError::io(&message)),
            None => self.hooks.search_hooks.fetch_bytes(store, url, Some(MAX_IMAGE_BYTES)).map_err(|err| match err {
                SearchError::Cancelled => CoverError::cancelled(),
                SearchError::Value(message) => CoverError::io(&message),
                other => CoverError::io(&other.to_string()),
            }),
        }
    }

    fn blocked_result(&self, store: &str, metadata: &Value, retry_after: f64) -> Value {
        let minutes = ((retry_after / 60.0).ceil() as i64).max(1);
        json!({
            "status": "error",
            "blocked": true,
            "retry_after": retry_after.trunc() as i64,
            "error": format!("서점이 일시적으로 요청을 제한하고 있습니다. 약 {minutes}분 뒤 다시 시도하세요"),
            "search_url": build_search_url(store, str_field(metadata, "title")),
            "candidates": [],
        })
    }

    pub fn candidates(&self, store: &str, relative: &str, include_covered: bool, refresh: bool) -> Result<Value, CoverError> {
        if !STORES.contains(&store) {
            return Err(CoverError::new("지원하지 않는 서점입니다"));
        }
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _book = self.book_lock(&relative, false)?;
        let inspection = EpubInspection { title: filename_title(&path), ..self.inspect(&path)? };
        if inspection.has_cover && !include_covered {
            return Err(CoverError::new(ALREADY_COVERED));
        }
        let metadata = serde_json::to_value(&inspection).map_err(|e| CoverError::io(&e.to_string()))?;
        let cache_key = sha256_hex(&format!("{store}:{relative}:{}:filename-v1", str_field(&metadata, "revision")));
        let book_cache = self.book_cache(&relative);
        let result_path = book_cache.join(format!("search-{cache_key}.json"));
        let flat_path = self.cache.join(format!("search-{cache_key}.json"));
        if refresh {
            let _ = std::fs::remove_file(&result_path);
            let _ = std::fs::remove_file(&flat_path);
        }
        if let Some(cached) = self.cached_search(store, &relative, &result_path)? {
            return Ok(cached);
        }
        // 이전 배포의 flat cache도 재사용한다.
        if let Some(cached) = self.cached_search(store, &relative, &flat_path)? {
            return Ok(cached);
        }
        let _store = self.acquire_store_lock(store)?;
        if let Some(cached) = self.cached_search(store, &relative, &result_path)? {
            return Ok(cached);
        }
        let gate = StoreGate::with_clock(&self.cache, store, self.hooks.clock()).with_cancel(Arc::new(is_cancelled));
        match gate.before_request() {
            Ok(()) => {}
            Err(GateError::Cancelled) => return Err(CoverError::cancelled()),
            Err(GateError::Blocked(blocked)) => return Ok(self.blocked_result(store, &metadata, blocked.retry_after)),
        }
        let error_result = |metadata: &Value| {
            json!({
                "status": "error",
                "error": "서점에 접근하지 못했습니다. 잠시 후 다시 시도하세요",
                "search_url": build_search_url(store, str_field(metadata, "title")),
                "candidates": [],
            })
        };
        let result = match self.search(store, &metadata) {
            Ok(mut result) => {
                gate.record_success();
                match self.store_candidates(store, &relative, &cache_key, &book_cache, &metadata, &mut result) {
                    Ok(()) => result,
                    Err(err) => {
                        log::warn!("Cover candidate search failed ({store}): {err}");
                        error_result(&metadata)
                    }
                }
            }
            Err(SearchError::Cancelled) => return Err(CoverError::cancelled()),
            Err(err) if blocked_status(&err) => {
                let retry_after = gate.record_blocked();
                let status = if let SearchError::Http { status } = err { status } else { 0 };
                log::warn!("Cover store rate-limited ({store}): HTTP {status}, pausing {}s", retry_after.trunc());
                return Ok(self.blocked_result(store, &metadata, retry_after));
            }
            Err(err) => {
                match &err {
                    SearchError::Http { status } => log::warn!("Cover candidate search failed ({store}): HTTPError HTTP {status}"),
                    SearchError::Value(_) => log::warn!("Cover candidate search failed ({store}): ValueError"),
                    _ => log::warn!("Cover candidate search failed ({store}): RequestException"),
                }
                error_result(&metadata)
            }
        };
        self.try_write(&result_path, &serde_json::to_vec(&result).map_err(|e| CoverError::io(&e.to_string()))?)?;
        Ok(result)
    }

    /// 검색 후보마다 기록 파일을 쓰고 응답에는 `candidate_id`만 남긴다(`image_url`은 서버만 안다).
    fn store_candidates(&self, store: &str, relative: &str, cache_key: &str, book_cache: &Path, metadata: &Value, result: &mut Value) -> Result<(), CoverError> {
        let Some(list) = result.get_mut("candidates").and_then(Value::as_array_mut) else { return Ok(()) };
        for candidate in list.iter_mut() {
            let image_url = candidate.get("image_url").and_then(Value::as_str).ok_or_else(|| CoverError::new("후보에 image_url이 없습니다"))?.to_string();
            let suffix = &sha256_hex(&format!("{cache_key}:{image_url}"))[..32];
            let candidate_id = format!("{}{suffix}", book_key(relative));
            let mut record = candidate.clone();
            merge(&mut record, &json!({"store": store, "file_path": relative, "revision": str_field(metadata, "revision")}));
            self.try_write(&book_cache.join(format!("candidate-{candidate_id}.json")), &serde_json::to_vec(&record).map_err(|e| CoverError::io(&e.to_string()))?)?;
            candidate["candidate_id"] = json!(candidate_id);
            if let Some(object) = candidate.as_object_mut() {
                object.remove("image_url");
            }
        }
        Ok(())
    }

    /// 후보 이미지를 받아 캐시에 둔다. 이미 있으면 그대로 돌려준다.
    pub fn image(&self, candidate_id: &str) -> Result<Vec<u8>, CoverError> {
        let candidate = self.candidate(candidate_id)?;
        let _book = self.book_lock(str_field(&candidate, "file_path"), false)?;
        let mut candidate = self.candidate(candidate_id)?;
        if self.revision_of(&self.resolve(str_field(&candidate, "file_path"), false)?) != str_field(&candidate, "revision") {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        let image_path = self.image_file(candidate_id)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let store = str_field(&candidate, "store").to_string();
        let _store = self.acquire_store_lock(&store)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let raw = self.fetch_store_bytes(&store, str_field(&candidate, "image_url"))?;
        let data = epub_cover::normalize_image_with(&raw, None, self.epub_hooks())?;
        self.try_write(&image_path, &data)?;
        self.image_info(candidate_id, &mut candidate, data)
    }
}
