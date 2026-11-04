//! 표지 저장·삭제·삽화 넣기와 책 캐시 정리.

use super::*;

impl CoverManagement {
    fn delete_book_cache(&self, relative: &str, expected_revision: &str) -> std::io::Result<()> {
        let book_cache = self.book_cache(relative);
        if let Some(f) = &self.hooks.delete_book_cache {
            return f(&book_cache);
        }
        if book_cache.exists() {
            std::fs::remove_dir_all(&book_cache)?;
        }
        // 이전 flat cache의 모든 서점·revision 후보도 해당 책만 정리한다.
        let mut revisions: BTreeSet<String> = BTreeSet::from([expected_revision.to_string()]);
        let mut aliases: BTreeSet<String> = BTreeSet::from([relative.to_string()]);
        if let Ok(entries) = std::fs::read_dir(&self.cache) {
            let mut names: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            names.sort();
            for path in names {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !(name.starts_with("candidate-") && name.ends_with(".json")) {
                    continue;
                }
                let Some(candidate) = self.read_json(&path).filter(has_entries) else { continue };
                if norm_rel(str_field(&candidate, "file_path")) != norm_rel(relative) {
                    continue;
                }
                revisions.insert(str_field(&candidate, "revision").to_string());
                aliases.insert(str_field(&candidate, "file_path").to_string());
                let candidate_id = name.trim_start_matches("candidate-").trim_end_matches(".json");
                remove_if_exists(&self.cache.join(format!("image-{candidate_id}.jpg")))?;
                remove_if_exists(&path)?;
            }
        }
        for store in STORES {
            for alias in &aliases {
                for old_revision in &revisions {
                    remove_if_exists(&self.cache.join(format!("search-{}.json", sha256_hex(&format!("{store}:{alias}:{old_revision}")))))?;
                }
            }
        }
        Ok(())
    }

    /// mtime을 보존하는 표지 편집 뒤 해당 책 ID의 썸네일과 미리보기를 지운다.
    pub fn invalidate_view_cache(&self, book_id: u64) {
        for directory in [".cover_cache", ".preview_cache"] {
            let Ok(entries) = std::fs::read_dir(self.root.join(directory)) else { continue };
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                let owned = name.starts_with(&format!("{book_id}.")) || name.starts_with(&format!("{book_id}_"));
                if owned && entry.path().is_file() {
                    let _ = remove_if_exists(&entry.path());
                }
            }
        }
    }

    // ---- 저장 ----

    pub fn apply(&self, relative: &str, candidate_id: &str, expected_revision: &str) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let candidate = self.candidate(candidate_id)?;
        if norm_rel(str_field(&candidate, "file_path")) != norm_rel(&relative) || str_field(&candidate, "revision") != expected_revision {
            return Err(CoverError::new("표지 후보와 EPUB이 일치하지 않습니다. 목록을 새로 불러오세요"));
        }
        let image_path = self.image_file(candidate_id)?;
        if !image_path.is_file() {
            return Err(CoverError::new("표지 이미지를 먼저 확인하세요"));
        }
        let data = std::fs::read(&image_path)?;
        self.save_cover(&path, &relative, &data, expected_revision, false, Backup::default())
    }

    pub fn apply_url(&self, relative: &str, url: &str, expected_revision: &str, replace_existing: bool) -> Result<Value, CoverError> {
        self.apply_url_backup(relative, url, expected_revision, replace_existing, Backup::default())
    }

    pub fn apply_url_backup(&self, relative: &str, url: &str, expected_revision: &str, replace_existing: bool, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let metadata = self.inspect(&path)?;
        if str_field(&metadata, "revision") != expected_revision {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        if truthy(metadata.get("has_cover")) && !replace_existing {
            return Err(CoverError::new(ALREADY_COVERED));
        }
        let data = self.download_image(url)?;
        self.save_cover(&path, &relative, &data, expected_revision, replace_existing, backup)
    }

    /// 공개 이미지 URL을 받는다. 서점 이미지 주소면 그 서점의 잠금 안에서 받는다.
    fn download_image(&self, url: &str) -> Result<Vec<u8>, CoverError> {
        let host = host_of(url);
        let store = STORES.iter().find(|name| store_hosts(name).iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))));
        let _guard = match store {
            Some(store) => Some(self.acquire_store_lock(store)?),
            None => None,
        };
        let fetcher: Arc<dyn ImageFetcher> = self.hooks.image_fetcher.clone().unwrap_or_else(|| Arc::new(RealImageFetcher));
        Ok(fetcher.fetch_url_image(url)?)
    }

    pub fn insert_illustration(&self, relative: &str, expected_revision: &str, before_idref: &str, image_data: Option<&str>) -> Result<Value, CoverError> {
        self.insert_illustration_backup(relative, expected_revision, before_idref, None, image_data, Backup::default())
    }

    /// `before_idref` 페이지 앞에 URL 또는 올린 이미지(base64)를 삽화로 넣는다. 표지 저장과 같은 잠금·백업·캐시 무효화를 쓴다.
    pub fn insert_illustration_backup(
        &self,
        relative: &str,
        expected_revision: &str,
        before_idref: &str,
        url: Option<&str>,
        image_data: Option<&str>,
        backup: Backup<'_>,
    ) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let decoded = match image_data.filter(|d| !d.is_empty()) {
            Some(data) => Some(decode_base64(data).map_err(|_| CoverError::new("삽화 이미지 데이터를 읽을 수 없습니다"))?),
            None => None,
        };
        let _lock = self.book_lock(&relative, true)?;
        let data = match decoded {
            Some(data) => data,
            None => self.download_image(url.unwrap_or(""))?,
        };
        let result = epub_cover::add_illustration_backup_with(&path, &data, expected_revision, &self.backup_dir(backup), before_idref, backup.name, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    /// 쪽 이미지를 한 번에 크롭·분할·삭제·삽입한다. `inserts`는 `[{before_idref, url | image_data(base64)}]`다.
    pub fn edit_images(&self, relative: &str, expected_revision: &str, edits: epub_cover::EditImages, inserts: &[Value], backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let mut decoded: Vec<(String, Option<Vec<u8>>, String)> = Vec::new();
        for entry in inserts {
            let mut data = None;
            if let Some(encoded) = entry.get("image_data").and_then(Value::as_str).filter(|d| !d.is_empty()) {
                data = Some(decode_base64(encoded).map_err(|_| CoverError::new("삽화 이미지 데이터를 읽을 수 없습니다"))?);
            }
            let url = str_field(entry, "url").trim().to_string();
            if data.is_none() && url.is_empty() {
                return Err(CoverError::new("이미지 URL이 비어 있습니다"));
            }
            decoded.push((str_field(entry, "before_idref").to_string(), data, url));
        }
        let _lock = self.book_lock(&relative, true)?;
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        for (before, data, url) in decoded {
            let bytes = match data {
                Some(data) => data,
                None => self.download_image(&url)?,
            };
            images.push((before, bytes));
        }
        let args = epub_cover::EditImages { inserts: images, backup_name: backup.name.map(str::to_string), ..edits };
        let result = epub_cover::edit_images_with(&path, expected_revision, &self.backup_dir(backup), args, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    pub fn apply_image(&self, relative: &str, image_data: &str, expected_revision: &str) -> Result<Value, CoverError> {
        self.apply_image_backup(relative, image_data, expected_revision, Backup::default())
    }

    pub fn apply_image_backup(&self, relative: &str, image_data: &str, expected_revision: &str, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let data = decode_base64(image_data).map_err(|_| CoverError::new("표지 이미지 데이터를 읽을 수 없습니다"))?;
        let _lock = self.book_lock(&relative, true)?;
        self.save_cover(&path, &relative, &data, expected_revision, true, backup)
    }

    pub fn clear_cache(&self, relative: &str, expected_revision: &str) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        if self.revision_of(&path) != expected_revision {
            return Err(CoverError::new(BOOK_CHANGED));
        }
        self.delete_book_cache(&relative, expected_revision)?;
        Ok(json!({"cleared": true, "file_path": relative}))
    }

    fn save_cover(&self, path: &Path, relative: &str, image: &[u8], expected_revision: &str, replace_existing: bool, backup: Backup<'_>) -> Result<Value, CoverError> {
        let result = epub_cover::add_cover_backup_with(path, image, expected_revision, &self.backup_dir(backup), replace_existing, backup.name, self.epub_hooks())?;
        self.finish_save(relative, result, expected_revision)
    }

    /// 이 도구가 넣은 표지를 EPUB에서 빼고, 저장 때와 같이 캐시를 비운다.
    pub fn remove(&self, relative: &str, expected_revision: &str, backup: Backup<'_>) -> Result<Value, CoverError> {
        let path = self.resolve(relative, false)?;
        let relative = self.relative_of(&path);
        let _lock = self.book_lock(&relative, true)?;
        let result = epub_cover::remove_cover_backup_with(&path, expected_revision, &self.backup_dir(backup), backup.name, self.epub_hooks())?;
        self.finish_save(&relative, result, expected_revision)
    }

    fn finish_save(&self, relative: &str, mut result: Value, expected_revision: &str) -> Result<Value, CoverError> {
        if let Err(err) = self.delete_book_cache(relative, expected_revision) {
            log::warn!("Cover saved; candidate cache cleanup failed: {:?}", err.kind());
            result["warning"] = json!("EPUB 표지를 저장했습니다. 후보 캐시를 삭제하지 못했습니다");
        }
        let book_id = result.get("book_id").and_then(Value::as_u64).unwrap_or(0);
        self.invalidate_view_cache(book_id);
        let backup_path = Path::new(str_field(&result, "backup_path"));
        let backup_relative = backup_path.strip_prefix(&self.root).map_err(|_| CoverError::io("백업 경로가 책 디렉토리 밖입니다"))?.to_string_lossy().into_owned();
        let mut response = Map::new();
        response.insert("file_path".to_string(), json!(relative));
        for key in ["revision", "book_id", "file_size", "cover_image"] {
            response.insert(key.to_string(), result.get(key).cloned().unwrap_or(Value::Null));
        }
        response.insert("backup_path".to_string(), json!(backup_relative));
        for key in ["warning", "idref", "idrefs"] {
            if truthy(result.get(key)) {
                response.insert(key.to_string(), result[key].clone());
            }
        }
        Ok(Value::Object(response))
    }
}
