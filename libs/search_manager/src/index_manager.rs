//! 색인 만들기와 refresh. 색인 스키마와 기존 색인의 필드 보강은 loader와 같은 `es_client` 구현 한 벌을 쓴다.

use super::*;
use es_client::EsClient;
use es_client::indexing::IndexStore;

impl ESManager {
    /// 이 전송 위의 `es_client` 색인 연산.
    pub(super) fn index_store(&self) -> EsClient {
        EsClient::from_transport(Arc::clone(&self.es))
    }

    /// 색인이 없으면 만들고, 있으면 빠진 필드(category.nori, created_time 계열, publisher)를 더한다.
    pub fn create_index(&self) -> Result<Value, EsError> {
        log::debug!("create_index()");
        self.index_store().create_index().map(|()| json!({"acknowledged": true}))
    }

    /// 인덱스를 refresh해 최근 변경사항을 검색 가능하게 한다.
    pub fn refresh(&self) -> Result<(), EsError> {
        log::debug!("refresh()");
        self.index_store().refresh()
    }
}
