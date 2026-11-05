#[path = "../shared/tls_server.rs"]
mod tls_server;

use es_client::EsClient;
use es_client::indexing::{IndexStore, index_mappings, index_settings, path_prefix_query};
use serde_json::json;

#[test]
fn settings_and_mappings_describe_the_nori_index() {
    let settings = index_settings();
    assert_eq!(settings["analysis"]["analyzer"]["nori_analyzer"]["tokenizer"], "nori_tokenizer");
    assert_eq!(settings["analysis"]["filter"]["nori_posfilter"]["stoptags"].as_array().unwrap().len(), 26);
    assert_eq!(settings["index"]["similarity"]["default"]["type"], "BM25");
    let props = &index_mappings()["properties"];
    assert_eq!(props["file_path"]["type"], "keyword");
    assert_eq!(props["file_size"]["type"], "unsigned_long");
    assert_eq!(props["category"]["fields"]["nori"]["analyzer"], "nori_analyzer");
    assert_eq!(props["publisher"]["fields"]["keyword"]["type"], "keyword");
    assert_eq!(props.as_object().unwrap().len(), 14);
}

#[test]
fn path_prefix_query_matches_the_path_itself_and_everything_below() {
    assert_eq!(path_prefix_query(""), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("."), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("/"), json!({"match_all": {}}));
    assert_eq!(path_prefix_query("/3_판타지/"), json!({"bool": {"should": [{"term": {"file_path": "3_판타지"}}, {"prefix": {"file_path": "3_판타지/"}}], "minimum_should_match": 1}}));
}

#[test]
fn bulk_insert_uses_verified_https_with_configured_ca() {
    let (url, server) = tls_server::serve_tls_once();
    let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/shared/tls/local-es.crt");
    let client = EsClient::new_with_ca(&url, "u", "p", "books", Some(&ca)).unwrap();
    client.insert(&[(1, json!({"title": "책"}))]).unwrap();
    server.join().unwrap();
}
