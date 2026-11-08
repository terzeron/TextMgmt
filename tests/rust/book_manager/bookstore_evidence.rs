//! 서점 한 곳의 표와 세 서점 합의 점수.

use std::collections::BTreeMap;

use book_manager::bookstore_evidence::{AGREEMENT_SCORES, LookupStatus, STORES, StoreResult, StoreRow, StoreVote, bookstore_scores, store_vote};

fn result(status: LookupStatus, categories: &[&str]) -> StoreResult {
    StoreResult { store: "yes24".to_string(), status, rows: categories.iter().map(|c| StoreRow { category: c.to_string(), ..StoreRow::default() }).collect() }
}

fn cats(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn not_zero(c: &str) -> bool {
    !c.starts_with("0_")
}

fn vote_of(r: &StoreResult, categories: &[&str]) -> StoreVote {
    store_vote(r, &cats(categories), &BTreeMap::new(), &not_zero)
}

fn vote(category: Option<&str>) -> StoreVote {
    StoreVote { store: String::new(), category: category.map(str::to_string), abstain: category.is_none().then(|| "blocked".to_string()) }
}

#[test]
fn constants_follow_the_design() {
    assert_eq!(STORES, ["yes24", "aladin", "kyobo"]);
    assert_eq!(AGREEMENT_SCORES, [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
}

#[test]
fn found_rows_vote_for_the_best_matching_category() {
    let v = vote_of(&result(LookupStatus::Found, &["소설/시/희곡 > SF > 한국SF"]), &["3_SF", "5_음악"]);
    assert_eq!((v.category.as_deref(), v.abstain), (Some("3_SF"), None));
    assert_eq!(v.store, "yes24");
}

#[test]
fn failed_lookups_abstain_with_their_reason() {
    for (status, reason) in [(LookupStatus::NotFound, "not_found"), (LookupStatus::Blocked, "blocked"), (LookupStatus::Error, "error")] {
        let v = vote_of(&result(status, &["SF > 한국SF"]), &["3_SF"]);
        assert_eq!((v.category, v.abstain.as_deref()), (None, Some(reason)));
    }
    let empty = vote_of(&result(LookupStatus::Found, &[]), &["3_SF"]);
    assert_eq!(empty.abstain.as_deref(), Some("not_found"));
}

#[test]
fn unmatched_store_category_abstains() {
    let v = vote_of(&result(LookupStatus::Found, &["zzz > qqq"]), &["3_SF"]);
    assert_eq!((v.category, v.abstain.as_deref()), (None, Some("no_match")));
}

#[test]
fn tied_top_categories_abstain() {
    let v = vote_of(&result(LookupStatus::Found, &["도서 > 과학"]), &["1_과학", "2_과학"]);
    assert_eq!((v.category, v.abstain.as_deref()), (None, Some("tie")));
}

#[test]
fn disallowed_top_category_abstains() {
    let v = vote_of(&result(LookupStatus::Found, &["도서 > 보관"]), &["0_보관", "3_SF"]);
    assert_eq!((v.category, v.abstain.as_deref()), (None, Some("excluded")));
}

#[test]
fn agreement_scores_follow_the_design_table() {
    let third = 1.0 / 3.0;
    assert_eq!(bookstore_scores(&[vote(Some("A")), vote(Some("A")), vote(Some("A"))]), Some(vec![("A".to_string(), 1.0)]));
    assert_eq!(bookstore_scores(&[vote(Some("A")), vote(Some("B")), vote(Some("A"))]), Some(vec![("A".to_string(), 2.0 * third), ("B".to_string(), third)]));
    assert_eq!(bookstore_scores(&[vote(Some("A")), vote(None), vote(Some("A"))]), Some(vec![("A".to_string(), 2.0 * third)]));
    assert_eq!(bookstore_scores(&[vote(Some("A")), vote(Some("B")), vote(Some("C"))]), Some(vec![("A".to_string(), third), ("B".to_string(), third), ("C".to_string(), third)]));
    assert_eq!(bookstore_scores(&[vote(None), vote(Some("B")), vote(None)]), Some(vec![("B".to_string(), third)]));
}

#[test]
fn all_abstaining_stores_give_no_evidence() {
    assert_eq!(bookstore_scores(&[vote(None), vote(None), vote(None)]), None);
    assert_eq!(bookstore_scores(&[]), None);
}
