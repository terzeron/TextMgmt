//! Python 3.13 `str.casefold()` mapping, generated from Unicode 15.1.

#[path = "casefold_table.rs"]
mod table;

pub fn py_casefold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        match table::CASE_FOLD.binary_search_by_key(&(character as u32), |(codepoint, _)| *codepoint) {
            Ok(index) => {
                for &codepoint in table::CASE_FOLD[index].1 {
                    folded.push(char::from_u32(codepoint).expect("generated casefold data contains Unicode scalar values"));
                }
            }
            Err(_) => folded.push(character),
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::py_casefold;

    #[test]
    fn full_casefold_expands_and_normalizes_unicode_cases() {
        assert_eq!(py_casefold("Straße Σς ﬃ"), "strasse σσ ffi");
    }
}
