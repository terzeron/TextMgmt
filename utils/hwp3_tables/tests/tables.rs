//! 매핑 표 테스트: Python 원본(`utils/hwp3_tables.py`)과 같은 값인지 확인한다.
//! 기대 체크섬은 원본 모듈에서 같은 FNV-1a 계산으로 얻었다.

use hwp3_tables::*;

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
