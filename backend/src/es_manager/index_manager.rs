//! 색인 존재 확인, 매핑 생성·보강, 삭제, refresh.

use super::*;

impl ESManager {
    pub fn do_exist_index(&self) -> Result<bool, EsError> {
        log::debug!("do_exist_index()");
        Ok(self.es.send(&EsRequest::IndexExists { index: None })?.as_bool() == Some(true))
    }

    /// 주어진 ID 목록의 기존 file_path를 조회한다. 반환: {inode: file_path}
    pub fn get_existing_paths(&self, doc_ids: &[i64]) -> Result<HashMap<i64, String>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        log::debug!("get_existing_paths({} ids)", doc_ids.len());
        let response = self.es.send(&EsRequest::MultiGet { docs: self.mget_docs(doc_ids), source: Some(vec!["file_path".to_string()]) })?;
        let mut result = HashMap::new();
        for doc in response["docs"].as_array().ok_or_else(|| parse_error("docs가 없다"))? {
            if doc["found"].as_bool().unwrap_or(false) {
                let path = doc["_source"]["file_path"].as_str().ok_or_else(|| parse_error("_source.file_path가 없다"))?;
                result.insert(hit_id(doc)?, path.to_string());
            }
        }
        Ok(result)
    }

    /// 주어진 ID 목록의 기존 created_time 관련 필드와 updated_time을 조회한다.
    pub fn get_existing_created_time_metadata(&self, doc_ids: &[i64]) -> Result<HashMap<i64, Value>, EsError> {
        if doc_ids.is_empty() {
            return Ok(HashMap::new());
        }
        log::debug!("get_existing_created_time_metadata({} ids)", doc_ids.len());
        let response = self.es.send(&EsRequest::MultiGet { docs: self.mget_docs(doc_ids), source: Some(["created_time", "created_time_source", "updated_time"].map(String::from).to_vec()) })?;
        let mut result = HashMap::new();
        for doc in response["docs"].as_array().ok_or_else(|| parse_error("docs가 없다"))? {
            if !doc["found"].as_bool().unwrap_or(false) {
                continue;
            }
            let mut metadata = Map::new();
            for field in ["created_time", "created_time_source", "updated_time"] {
                if let Some(value) = doc["_source"].get(field) {
                    metadata.insert(field.to_string(), value.clone());
                }
            }
            if !metadata.is_empty() {
                result.insert(hit_id(doc)?, Value::Object(metadata));
            }
        }
        Ok(result)
    }

    fn mget_docs(&self, doc_ids: &[i64]) -> Vec<Value> {
        doc_ids.iter().map(|id| json!({"_index": self.index(), "_id": id.to_string()})).collect()
    }

    pub fn create_index(&self) -> Result<Value, EsError> {
        log::debug!("create_index()");
        let settings = json!({
            "index": {"similarity": {"default": {"type": "BM25"}}},
            "analysis": {
                "tokenizer": {"nori_tokenizer": {"type": "nori_tokenizer", "decompound_mode": "discard"}},
                "filter": {"nori_posfilter": {"type": "nori_part_of_speech", "stoptags": [
                    // 어미
                    "EC", "EF", "EP", "ETM", "ETN",
                    // 조사
                    "JC", "JKB", "JKC", "JKG", "JKO", "JKQ", "JKS", "JKV", "JX",
                    // 기호
                    "SC", "SE", "SF", "SP", "SSC", "SSO", "SY",
                    // 접미사
                    "XSA", "XSN", "XSV",
                    // 기타
                    "IC", "MAJ",
                ]}},
                "analyzer": {"nori_analyzer": {"type": "custom", "tokenizer": "nori_tokenizer", "filter": ["nori_posfilter", "lowercase"]}},
            },
        });
        let mappings = json!({"properties": {
            "category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}},
            "title": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            "author": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            // EPUB의 dc:publisher. 전집 판정의 결정적 증거라 자동분류가 쓴다.
            "publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}},
            "file_path": {"type": "keyword"},
            "file_type": {"type": "keyword"},
            "file_size": {"type": "unsigned_long"},
            "line_count": {"type": "integer"},
            "page_count": {"type": "integer"},
            "isbn": {"type": "keyword"},
            "summary": {"type": "text", "analyzer": "nori_analyzer"},
            "created_time": {"type": "date"},
            "created_time_source": {"type": "keyword"},
            "updated_time": {"type": "date"},
        }});

        if self.do_exist_index()? {
            self.ensure_existing_index_mappings();
            return Ok(json!({"acknowledged": true}));
        }
        match self.es.send(&EsRequest::CreateIndex { index: None, body: json!({"settings": settings, "mappings": mappings}) }) {
            Err(EsError::Status { status: 400, body }) if body.contains("resource_already_exists_exception") => {
                log::info!("Index {} already exists, skipping creation", self.index());
                self.ensure_existing_index_mappings();
                Ok(json!({"acknowledged": true}))
            }
            other => other,
        }
    }

    /// 매핑을 한 번만 읽어 세 필드를 모두 점검한다.
    fn ensure_existing_index_mappings(&self) {
        match self.es.send(&EsRequest::GetMapping { index: None }) {
            Ok(mapping) => {
                self.apply_category_nori(&mapping);
                self.apply_created_time(&mapping);
                self.apply_publisher(&mapping);
            }
            Err(e) => log::warn!("Failed to read mapping of index {}: {e}", self.index()),
        }
    }

    fn index_mappings<'a>(&self, mapping: &'a Value) -> Option<&'a Value> {
        mapping.get(self.index())?.get("mappings")
    }

    fn put_properties(&self, properties: Value) -> Result<(), EsError> {
        self.es.send(&EsRequest::PutMapping { index: None, properties: properties.clone() }).map(|_| ())
    }

    /// 기존 인덱스에 category.nori 서브필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn ensure_category_nori_subfield(&self) {
        match self.es.send(&EsRequest::GetMapping { index: None }) {
            Ok(mapping) => self.apply_category_nori(&mapping),
            Err(e) => log::warn!("Failed to add category.nori sub-field: {e}"),
        }
    }

    fn apply_category_nori(&self, mapping: &Value) {
        let Some(properties) = self.index_mappings(mapping).and_then(|m| m.get("properties")) else {
            log::warn!("Failed to add category.nori sub-field: 매핑에 properties가 없다");
            return;
        };
        if properties["category"]["fields"].get("nori").is_some() {
            return;
        }
        log::info!("Adding category.nori sub-field to index {}", self.index());
        match self.put_properties(json!({"category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}}}})) {
            Ok(()) => log::info!("category.nori sub-field added successfully"),
            Err(e) => log::warn!("Failed to add category.nori sub-field: {e}"),
        }
    }

    /// 기존 인덱스에 created_time 관련 필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn ensure_created_time_field(&self) {
        match self.es.send(&EsRequest::GetMapping { index: None }) {
            Ok(mapping) => self.apply_created_time(&mapping),
            Err(e) => log::warn!("Failed to add created_time fields: {e}"),
        }
    }

    fn apply_created_time(&self, mapping: &Value) {
        let Some(mappings) = self.index_mappings(mapping) else {
            log::warn!("Failed to add created_time fields: 매핑이 없다");
            return;
        };
        let properties = mappings.get("properties");
        let has = |name: &str| properties.is_some_and(|p| p.get(name).is_some());
        let mut missing = Map::new();
        if !has("created_time") {
            missing.insert("created_time".into(), json!({"type": "date"}));
        }
        if !has("created_time_source") {
            missing.insert("created_time_source".into(), json!({"type": "keyword"}));
        }
        if missing.is_empty() {
            return;
        }
        log::info!("Adding created_time fields to index {}: {:?}", self.index(), missing.keys().collect::<Vec<_>>());
        match self.put_properties(Value::Object(missing)) {
            Ok(()) => log::info!("created_time fields added successfully"),
            Err(e) => log::warn!("Failed to add created_time fields: {e}"),
        }
    }

    /// 기존 인덱스에 publisher 필드가 없으면 추가한다. 오류는 삼킨다.
    pub fn ensure_publisher_field(&self) {
        match self.es.send(&EsRequest::GetMapping { index: None }) {
            Ok(mapping) => self.apply_publisher(&mapping),
            Err(e) => log::warn!("Failed to add publisher field: {e}"),
        }
    }

    fn apply_publisher(&self, mapping: &Value) {
        let Some(mappings) = self.index_mappings(mapping) else {
            log::warn!("Failed to add publisher field: 매핑이 없다");
            return;
        };
        if mappings.get("properties").is_some_and(|p| p.get("publisher").is_some()) {
            return;
        }
        log::info!("Adding publisher field to index {}", self.index());
        match self.put_properties(json!({"publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}}})) {
            Ok(()) => log::info!("publisher field added successfully"),
            Err(e) => log::warn!("Failed to add publisher field: {e}"),
        }
    }

    pub fn delete_index(&self) -> Result<(), EsError> {
        log::debug!("delete_index()");
        if self.do_exist_index()? {
            self.es.send(&EsRequest::DeleteIndex { index: None })?;
        }
        Ok(())
    }

    /// 인덱스를 refresh해 최근 변경사항을 검색 가능하게 한다.
    pub fn refresh(&self) -> Result<(), EsError> {
        log::debug!("refresh()");
        self.es.send(&EsRequest::RefreshIndex { index: None }).map(|_| ())
    }
}
