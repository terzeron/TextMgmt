//! 카테고리 변경 적용 테스트의 옵션(`ApplyOpts`)과 실행 helper. 여러 테스트 파일의 같은 사본을 모았다.
#![allow(dead_code)]

use super::shared_fixtures::outcome;
use backend::book_manager::{ApplyOptions, BookManager};
use serde_json::Value;
use std::collections::BTreeSet;

/// `apply_category_changes`의 인자 묶음(테스트 로컬). 구현의 `ApplyOptions`로 바꿔 부른다.
pub struct ApplyOpts {
    pub allowed: BTreeSet<String>,
    pub on_item_done: Option<Box<dyn Fn(Value) + Send + Sync>>,
    pub should_continue: Option<Box<dyn Fn() -> bool + Send + Sync>>,
}

impl ApplyOpts {
    pub fn new(allowed: BTreeSet<String>) -> Self {
        Self { allowed, on_item_done: None, should_continue: None }
    }

    pub fn on_item_done(mut self, f: Box<dyn Fn(Value) + Send + Sync>) -> Self {
        self.on_item_done = Some(f);
        self
    }

    pub fn should_continue(mut self, f: Box<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.should_continue = Some(f);
        self
    }
}

/// 적용 결과를 `(JSON, 오류)`로 펼친다. JSON은 상태 문서에 저장하는 모양이다.
pub async fn apply_opts(manager: &BookManager, items: &[Value], opts: ApplyOpts) -> (Value, Option<String>) {
    let allowed: std::collections::HashSet<String> = opts.allowed.iter().cloned().collect();
    outcome(
        manager
            .apply_category_changes_with(items, &allowed, ApplyOptions { on_item_done: opts.on_item_done.as_deref(), should_continue: opts.should_continue.as_deref(), ..ApplyOptions::default() })
            .await
            .map(|result| serde_json::to_value(result).unwrap()),
    )
}
