//! Python 3.13 `str.lower()` compatibility for Unicode 15.1.

use crate::py_is_assigned;

pub fn py_lowercase(value: &str) -> String {
    let mut lowered = String::with_capacity(value.len());
    let mut assigned = String::new();
    for character in value.chars() {
        if py_is_assigned(character) {
            assigned.push(character);
        } else {
            lowered.push_str(&assigned.to_lowercase());
            assigned.clear();
            lowered.push(character);
        }
    }
    lowered.push_str(&assigned.to_lowercase());
    lowered
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::py_lowercase;

    #[test]
    fn lowercase_matches_python_313_unicode_151_for_every_codepoint() {
        let mappings: HashMap<u32, String> = include_str!("../../../tests/fixtures/python_lower_mappings_15_1.txt")
            .lines()
            .map(|line| {
                let (codepoint, lowered) = line.split_once(';').unwrap();
                let lowered = lowered.split(',').map(|cp| char::from_u32(u32::from_str_radix(cp, 16).unwrap()).unwrap()).collect();
                (u32::from_str_radix(codepoint, 16).unwrap(), lowered)
            })
            .collect();
        for_every_codepoint(|codepoint, character| {
            let input = character.to_string();
            let expected = mappings.get(&codepoint).map_or(input.as_str(), String::as_str);
            assert_eq!(py_lowercase(&input), expected, "U+{codepoint:04X}");
        });
    }

    /// U+0000~U+10FFFF를 코어 수만큼 나눠 검사한다. 디버그 빌드에서 한 스레드로 돌면 1초 안팎이 걸린다.
    fn for_every_codepoint(check: impl Fn(u32, char) + Sync) {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
        let chunk = 0x11_0000u32.div_ceil(threads);
        std::thread::scope(|scope| {
            for start in (0..0x11_0000u32).step_by(chunk as usize) {
                let check = &check;
                scope.spawn(move || (start..(start + chunk).min(0x11_0000)).for_each(|codepoint| char::from_u32(codepoint).into_iter().for_each(|character| check(codepoint, character))));
            }
        });
    }

    #[test]
    fn unassigned_codepoint_ends_final_sigma_context() {
        assert_eq!(py_lowercase("ΟΣ\u{1c89}A"), "ος\u{1c89}a");
    }
}
