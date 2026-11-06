//! 서점 검색으로 표지 후보를 찾고, 후보 정보와 이미지를 캐시에서 읽는다.

use super::*;

const SEARCH_FAILED: &str = "서점에 접근하지 못했습니다. 잠시 후 다시 시도하세요";

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

    pub fn candidate(&self, candidate_id: &str) -> Result<CandidateRecord, CoverError> {
        self.read_json(&self.candidate_file(candidate_id)?).ok_or_else(|| CoverError::new("표지 후보를 찾을 수 없습니다. 후보를 다시 검색하세요"))
    }

    pub(super) fn image_file(&self, candidate_id: &str) -> Result<PathBuf, CoverError> {
        Ok(self.candidate_file(candidate_id)?.parent().unwrap_or(&self.cache).join(format!("image-{candidate_id}.jpg")))
    }

    /// 후보 기록에 이미지 크기가 없으면 `data`에서 읽어 기록에 남긴다.
    fn image_info(&self, candidate_id: &str, candidate: &mut CandidateRecord, data: Vec<u8>) -> Result<Vec<u8>, CoverError> {
        if candidate.image.is_none() {
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .and_then(|reader| reader.into_dimensions().map_err(std::io::Error::other))
                .map_err(|_| CoverError::io("이미지 형식을 읽을 수 없습니다"))?;
            candidate.image = Some(CoverImageInfo { byte_size: data.len() as u64, width: Some(width), height: Some(height) });
            self.try_write(&self.candidate_file(candidate_id)?, &to_json_bytes(candidate)?)?;
        }
        Ok(data)
    }

    /// 캐시한 검색 결과. 성공 결과는 후보 기록이 모두 남아 있어야 쓴다. 오류는 2분, 그 밖은 1시간 뒤 버린다.
    fn cached_search(&self, store: &str, relative: &str, path: &Path) -> Result<Option<CandidateSearch>, CoverError> {
        if let Some(f) = &self.hooks.cached_search {
            return Ok(f(store, relative));
        }
        let cached: Option<CandidateSearch> = self.read_json(path);
        if let Some(mut cached) = cached.clone().filter(|c| c.status == SearchStatus::Success) {
            for view in &mut cached.candidates {
                let record_path = self.candidate_file(&view.candidate_id)?;
                let Some(mut record) = self.read_json::<CandidateRecord>(&record_path) else { return Ok(None) };
                let image_path = record_path.parent().unwrap_or(&self.cache).join(format!("image-{}.jpg", view.candidate_id));
                if image_path.is_file() && record.image.is_none() {
                    self.image_info(&view.candidate_id, &mut record, std::fs::read(&image_path)?)?;
                }
                if record.image.is_some() {
                    view.image = record.image;
                }
            }
            return Ok(Some(cached));
        }
        let is_error = cached.as_ref().is_some_and(|c| c.status == SearchStatus::Error);
        Ok(self.read_json_within(path, Some(if is_error { 120.0 } else { 3600.0 })))
    }

    fn search(&self, store: &str, metadata: &EpubInspection) -> Result<StoreSearch, SearchError> {
        match &self.hooks.search {
            Some(f) => f(store, metadata),
            None => search_candidates_with(store, &crate::cover_sources::CoverQuery { title: metadata.title.clone(), isbn: metadata.isbn.clone() }, &self.hooks.search_hooks),
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

    fn blocked_result(&self, store: &str, title: &str, retry_after: f64) -> CandidateSearch {
        let minutes = ((retry_after / 60.0).ceil() as i64).max(1);
        CandidateSearch {
            blocked: true,
            retry_after: Some(retry_after.trunc() as i64),
            ..CandidateSearch::failed(build_search_url(store, title), format!("서점이 일시적으로 요청을 제한하고 있습니다. 약 {minutes}분 뒤 다시 시도하세요"))
        }
    }

    pub fn candidates(&self, store: &str, relative: &str, include_covered: bool, refresh: bool) -> Result<CandidateSearch, CoverError> {
        if !STORES.contains(&store) {
            return Err(CoverError::new("지원하지 않는 서점입니다"));
        }
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _book = self.book_lock(&relative, false)?;
        let metadata = EpubInspection { title: filename_title(&path), ..self.inspect(&path)? };
        if metadata.has_cover && !include_covered {
            return Err(CoverError::new(ALREADY_COVERED));
        }
        let cache_key = sha256_hex(&format!("{store}:{relative}:{}:filename-v1", metadata.revision));
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
            Err(GateError::Blocked(blocked)) => return Ok(self.blocked_result(store, &metadata.title, blocked.retry_after)),
        }
        let search_failed = || CandidateSearch::failed(build_search_url(store, &metadata.title), SEARCH_FAILED.to_string());
        let result = match self.search(store, &metadata) {
            Ok(found) => {
                gate.record_success();
                match self.store_candidates(store, &relative, &cache_key, &book_cache, &metadata.revision, found) {
                    Ok(result) => result,
                    Err(err) => {
                        log::warn!("Cover candidate search failed ({store}): {err}");
                        search_failed()
                    }
                }
            }
            Err(SearchError::Cancelled) => return Err(CoverError::cancelled()),
            Err(err) if blocked_status(&err) => {
                let retry_after = gate.record_blocked();
                let status = if let SearchError::Http { status } = err { status } else { 0 };
                log::warn!("Cover store rate-limited ({store}): HTTP {status}, pausing {}s", retry_after.trunc());
                return Ok(self.blocked_result(store, &metadata.title, retry_after));
            }
            Err(err) => {
                match &err {
                    SearchError::Http { status } => log::warn!("Cover candidate search failed ({store}): HTTPError HTTP {status}"),
                    SearchError::Value(_) => log::warn!("Cover candidate search failed ({store}): ValueError"),
                    _ => log::warn!("Cover candidate search failed ({store}): RequestException"),
                }
                search_failed()
            }
        };
        self.try_write(&result_path, &to_json_bytes(&result)?)?;
        Ok(result)
    }

    /// 검색 후보마다 기록 파일을 쓰고 응답에는 `candidate_id`만 남긴다(`image_url`은 서버만 안다).
    fn store_candidates(&self, store: &str, relative: &str, cache_key: &str, book_cache: &Path, revision: &str, found: StoreSearch) -> Result<CandidateSearch, CoverError> {
        let mut views = Vec::with_capacity(found.candidates.len());
        for candidate in found.candidates {
            if candidate.image_url.is_empty() {
                return Err(CoverError::new("후보에 image_url이 없습니다"));
            }
            let suffix = &sha256_hex(&format!("{cache_key}:{}", candidate.image_url))[..32];
            let candidate_id = format!("{}{suffix}", book_key(relative));
            let view = CandidateView { title: candidate.title.clone(), author: candidate.author.clone(), detail_url: candidate.detail_url.clone(), candidate_id: candidate_id.clone(), image: None };
            let record = CandidateRecord { candidate, store: store.to_string(), file_path: relative.to_string(), revision: revision.to_string(), image: None };
            self.try_write(&book_cache.join(format!("candidate-{candidate_id}.json")), &to_json_bytes(&record)?)?;
            views.push(view);
        }
        Ok(CandidateSearch { status: found.status, search_url: found.search_url, candidates: views, ..CandidateSearch::default() })
    }

    /// 후보 이미지를 받아 캐시에 둔다. 이미 있으면 그대로 돌려준다.
    pub fn image(&self, candidate_id: &str) -> Result<Vec<u8>, CoverError> {
        let candidate = self.candidate(candidate_id)?;
        let _book = self.book_lock(&candidate.file_path, false)?;
        let mut candidate = self.candidate(candidate_id)?;
        if self.revision_of(&self.resolve(&candidate.file_path, false)?) != candidate.revision {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        let image_path = self.image_file(candidate_id)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let store = candidate.store.clone();
        let _store = self.acquire_store_lock(&store)?;
        if image_path.exists() {
            return self.image_info(candidate_id, &mut candidate, std::fs::read(&image_path)?);
        }
        let raw = self.fetch_store_bytes(&store, &candidate.candidate.image_url)?;
        let data = epub_cover::normalize_image_with(&raw, None, self.epub_hooks())?;
        self.try_write(&image_path, &data)?;
        self.image_info(candidate_id, &mut candidate, data)
    }
}

fn to_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, CoverError> {
    serde_json::to_vec(value).map_err(|e| CoverError::io(&e.to_string()))
}
