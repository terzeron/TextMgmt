//! `hwp3_parser::tables` 단위 테스트. 원본 테스트 파일마다 하위 모듈로 나눠 둔다.

// `tests/rust/hwp3_parser/tables.rs`에서 옮긴 테스트.
mod tables {
    //! 매핑 표 테스트: Python 원본(`utils/hwp3_tables.py`)과 같은 값인지 확인한다.
    //! 기대 체크섬은 원본 모듈에서 같은 FNV-1a 계산으로 얻었다.

    use hwp3_parser::tables::*;

    fn fnv(values: impl IntoIterator<Item = u64>) -> u64 {
        values.into_iter().fold(1469598103934665603u64, |h, v| (h ^ v).wrapping_mul(1099511628211))
    }

    #[test]
    fn small_tables_match_the_python_module() {
        let tables: [(&str, &[u16], u64); 9] = [
            ("L_MAP", &L_MAP[..], 4912895135183160142u64),
            ("V_MAP", &V_MAP[..], 12646895591070372532u64),
            ("T_MAP", &T_MAP[..], 3322249348969978857u64),
            ("HNC_L1", &HNC_L1[..], 5046002326284863935u64),
            ("HNC_V1", &HNC_V1[..], 5665011597208160034u64),
            ("HNC_T1", &HNC_T1[..], 15522155207017611168u64),
            ("HNC_L2", &HNC_L2[..], 9096962167701101926u64),
            ("HNC_V2", &HNC_V2[..], 9935408977178958829u64),
            ("HNC_T2", &HNC_T2[..], 729911041499360139u64),
        ];
        for (name, table, expected) in tables {
            assert_eq!(table.len(), 32, "{name}");
            assert_eq!(fnv(table.iter().map(|v| u64::from(*v))), expected, "{name}");
        }
    }

    #[test]
    fn ksc5601_table_matches_the_python_module() {
        assert_eq!(KSC5601_TO_UNI.len(), 4888);
        assert_eq!(fnv(KSC5601_TO_UNI.iter().map(|v| u64::from(*v))), 13439108652521159260u64);
        assert_eq!(KSC5601_TO_UNI[0], 0x4f3d);
        assert_eq!(KSC5601_TO_UNI[4887], 0x8a70);
    }

    #[test]
    fn hnc2uni_matches_the_python_module_and_is_sorted_for_binary_search() {
        assert_eq!(HNC2UNI.len(), 11979);
        assert!(HNC2UNI.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(fnv(HNC2UNI.iter().flat_map(|(k, v)| [u64::from(*k), u64::from(*v)])), 2780894431007483675u64);
        assert_eq!(hnc2uni(0x007f), Some(0xa1));
        assert_eq!(hnc2uni(0x7f02), Some(0x813f));
        assert_eq!(hnc2uni(0), None);
        assert_eq!(hnc2uni(0xffff), None);
    }

    #[test]
    fn markers_have_the_python_values() {
        assert_eq!(NONE, 0xFFFD);
        assert_eq!(FILL, 0xFF);
    }
}

// `tests/rust/hwp3_parser/hwp3.rs`에서 옮긴 테스트.
mod hwp3 {
    //! HWP V3.00/V2.10/V2.00 텍스트 추출기 테스트 (tests/test_hwp3_parser.py 이식).

    use hwp3_parser::*;
    use hwp3_parser::tables::{HNC2UNI, KSC5601_TO_UNI};

    fn codes(text: &str) -> Vec<u16> {
        text.chars().map(|c| c as u16).collect()
    }

    fn pack(codes: &[u16]) -> Vec<u8> {
        codes.iter().flat_map(|c| c.to_le_bytes()).collect()
    }

    #[test]
    fn table_ranges_use_ksc5601_and_hnc2uni() {
        let known = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x7F..=0x3FFF).contains(k)).unwrap();
        assert!(hnc_to_unicode(known).chars().count() <= 1);
        let high = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x5318..=0x7FFF).contains(k)).unwrap();
        assert!(hnc_to_unicode(high).chars().count() <= 1);
        let unknown = (0x80u16..0x200).find(|c| HNC2UNI.binary_search_by_key(c, |(k, _)| *k).is_err()).unwrap();
        assert_eq!(hnc_to_unicode(unknown), "");
        if let Some(i) = KSC5601_TO_UNI.iter().position(|v| *v == 0) {
            assert_eq!(hnc_to_unicode(0x4000 + i as u16), "");
        }
        let mapped = KSC5601_TO_UNI.iter().position(|v| *v != 0).unwrap();
        assert_eq!(hnc_to_unicode(0x4000 + mapped as u16), char::from_u32(u32::from(KSC5601_TO_UNI[mapped])).unwrap().to_string());
    }

    #[test]
    fn bruteforce_flushes_runs_at_unknown_and_control_codes() {
        let mut a = codes("Hello World Test");
        let unknown = (0x100u16..0x400).find(|c| HNC2UNI.binary_search_by_key(c, |(k, _)| *k).is_err()).unwrap();
        a.push(unknown);
        a.extend(codes("Second Readable Run"));
        let out = extract_text_bruteforce(&pack(&a));
        assert_eq!(out, "Hello World Test\nSecond Readable Run");
        let mut b = codes("Hello World Test");
        b.push(4);
        b.extend(codes("After Control Code"));
        assert_eq!(extract_text_bruteforce(&pack(&b)), "Hello World Test\nAfter Control Code");
    }
}
