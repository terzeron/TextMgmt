//! All backend tests that need MySQL or Elasticsearch share one process-scoped
//! Testcontainers fixture, matching pytest's session-scoped containers.

mod common;
#[path = "common/elasticsearch_fixture.rs"]
mod elasticsearch_fixture;
#[path = "common/mysql_fixture.rs"]
mod mysql_fixture;

#[path = "book_manager_1.rs"]
mod book_manager_1;
#[path = "classify_proposal_items_db.rs"]
mod classify_proposal_items_db;
#[path = "classify_proposal_status_db.rs"]
mod classify_proposal_status_db;
#[path = "cover_status.rs"]
mod cover_status;
#[path = "db.rs"]
mod db;
#[path = "main.rs"]
mod main_api;
#[path = "refresh_token_store.rs"]
mod refresh_token_store;
#[path = "view_history_api.rs"]
mod view_history_api;
#[path = "view_history_store.rs"]
mod view_history_store;
