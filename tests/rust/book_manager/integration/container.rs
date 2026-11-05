//! `book_manager` 컨테이너 통합 테스트. MySQL·Elasticsearch testcontainers를 한 프로세스에서 함께 쓴다.

#[path = "../common/mod.rs"]
mod common;
#[path = "../../shared/elasticsearch_fixture.rs"]
mod elasticsearch_fixture;
#[path = "../../shared/mysql_fixture.rs"]
mod mysql_fixture;

#[path = "container/book_manager.rs"]
mod manager_store;
