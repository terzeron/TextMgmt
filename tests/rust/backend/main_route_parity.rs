//! `backend/main.py` OpenAPI route set versus the Rust route registry.

use std::collections::HashSet;

use backend::main::route_table;

fn normalize_path(method: &str, path: &str) -> String {
    let path = path.replace("{*", "{");
    if method != "GET" {
        return path;
    }
    ["/categories/rename", "/categories/delete"].into_iter().find_map(|reserved| path.strip_suffix(reserved).map(|prefix| format!("{prefix}/categories/{{category}}"))).unwrap_or(path)
}

#[test]
fn rust_registers_every_python_openapi_route_and_method() {
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/python_openapi_routes.json"))).unwrap();
    let actual: HashSet<(String, String, Vec<String>)> = route_table()
        .into_iter()
        .flat_map(|route| {
            route.methods.into_iter().map(move |method| {
                let dependencies = route.dependencies.clone();
                (method.clone(), normalize_path(&method, &route.path), dependencies)
            })
        })
        .collect();
    let expected: HashSet<_> = expected.into_iter().collect();
    let missing: Vec<_> = expected.difference(&actual).collect();
    let extra: Vec<_> = actual.difference(&expected).collect();
    assert!(missing.is_empty() && extra.is_empty(), "Rust missing routes: {missing:?}; Rust-only routes: {extra:?}");
}
