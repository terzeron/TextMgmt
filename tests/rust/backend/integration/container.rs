//! All backend tests that need MySQL or Elasticsearch share one process-scoped
//! Testcontainers fixture, matching pytest's session-scoped containers.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/elasticsearch_fixture.rs"]
mod elasticsearch_fixture;
#[path = "../common/mysql_fixture.rs"]
mod mysql_fixture;

#[path = "container/book_manager_1.rs"]
mod book_manager_1;
#[path = "container/classify_proposal_items_db.rs"]
mod classify_proposal_items_db;
#[path = "container/classify_proposal_status_db.rs"]
mod classify_proposal_status_db;
#[path = "container/cover_status.rs"]
mod cover_status;
#[path = "container/db.rs"]
mod db;
#[path = "container/main.rs"]
mod main_api;
#[path = "container/refresh_token_store.rs"]
mod refresh_token_store;
#[path = "container/view_history_api.rs"]
mod view_history_api;
#[path = "container/view_history_store.rs"]
mod view_history_store;
