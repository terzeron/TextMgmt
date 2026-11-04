//! `classifier::book_classifier::map_category` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::path::Path;
use classifier::book_classifier::*;
use serde_json::Value;
use tempfile::TempDir;

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/book_classifier_parity.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn test_map_category_rules() {
    // (cat_str, raw_title, raw_author, expected)
    let cases: &[(&str, &str, &str, Option<&str>)] = &[
        ("", "", "", None),
        ("", "평범한 제목", "히가시노 게이고", Some("2_소설일본게이고")),
        ("", "평범한 제목", "무라카미 하루키", Some("2_소설일본하루키")),
        ("", "을유세계문학 고전", "", Some("2_을유세계문학전집")),
        ("", "열린책들 세계문학", "", Some("2_열린책들세계문학")),
        ("", "문예세계문학선", "", Some("2_문예세계문학선")),
        ("", "살림지식총서 001", "", Some("4_살림지식총서")),
        ("", "시공디스커버리 총서", "", Some("4_시공디스커버리")),
        ("", "올재 클래식스", "", Some("1_올재")),
        ("", "누워서 읽는 법학", "", Some("4_누워서읽는법학")),
        ("", "이지사이언스", "", Some("5_이지사이언스")),
        ("", "그림으로 읽는 과학", "", Some("5_그림으로_읽는")),
        ("", "손자병법", "", Some("1_동양고전")),
        ("라이트노벨", "", "", Some("3_라이트노벨")),
        ("BL/GL", "", "", Some("9_BLGL")),
        ("그래픽노블", "", "", Some("3_그래픽노블")),
        ("미스터리", "", "", Some("3_스릴러")),
        ("과학소설 > SF", "", "", Some("3_SF")),
        ("판타지소설", "", "", Some("3_판타지")),
        ("무협소설", "", "", Some("3_무협")),
        ("로맨스판타지", "", "", Some("3_여성향")),
        ("에세이", "", "", Some("2_수필서간일기")),
        ("한국소설", "", "", Some("2_소설한국")),
        ("일본소설", "", "", Some("2_소설일본")),
        ("중국소설", "", "", Some("2_소설중국")),
        ("영미소설", "", "", Some("2_소설외국")),
        ("역사소설", "", "", Some("2_소설역사")),
        ("시집", "", "", Some("2_시")),
        ("글쓰기", "", "", Some("2_문학일반서평작법독서")),
        ("문헌정보", "", "", Some("1_문헌서지")),
        ("동양고전", "", "", Some("1_동양고전")),
        ("서양고전", "", "", Some("1_서양고전")),
        ("투자", "", "", Some("6_재테크")),
        ("국내도서 > 경제일반", "", "", Some("4_경제")),
        ("마케팅", "", "", Some("4_경영마케팅")),
        ("국내도서 > 경영", "", "", Some("4_경영마케팅")),
        ("자기계발", "", "", Some("6_처세술리더십창의성")),
        ("심리학", "", "", Some("4_심리학뇌과학")),
        ("철학", "", "", Some("4_철학윤리")),
        ("한국사", "", "", Some("4_역사인물")),
        ("정치학", "", "", Some("4_정치외교군사")),
        ("법학", "", "", Some("4_법")),
        ("사회과학", "", "", Some("4_사회인류")),
        ("종교", "", "", Some("4_종교신화")),
        ("인문학", "", "", Some("4_인문일반논픽션")),
        ("교양과학", "", "", Some("5_수학과학일반")),
        ("미술", "", "", Some("5_미술예술건축")),
        ("음악", "", "", Some("5_음악")),
        ("영화", "", "", Some("5_영화")),
        ("사진", "", "", Some("5_사진영상")),
        ("스포츠", "", "", Some("5_스포츠")),
        ("서브컬쳐", "", "", Some("5_서브컬쳐")),
        ("교육학", "", "", Some("7_교육일반")),
        ("국어", "", "", Some("7_국어교육")),
        ("영어회화", "", "", Some("7_영어교육")),
        ("토익", "", "", Some("7_영어교육")),
        ("일본어", "", "", Some("7_일어교육")),
        ("중국어", "", "", Some("7_중어한자교육")),
        ("HSK", "", "", Some("7_중어한자교육")),
        ("프랑스어", "", "", Some("7_외국어교육")),
        ("언어학", "", "", Some("7_언어일반")),
        ("영어원서", "", "", Some("7_영문일반")),
        ("영문원서", "", "", Some("7_영문일반")),
        ("프로그래밍", "", "", Some("8_IT")),
        ("건강", "", "", Some("8_건강일반")),
        ("요리", "", "", Some("8_요리음료")),
        ("여행", "", "", Some("8_여행")),
        ("공인중개사", "", "", Some("8_공인중개사")),
        ("프라모델", "", "", Some("8_모델링")),
        ("밀리터리", "", "", Some("8_밀리터리")),
        ("성교육", "", "", Some("8_성")),
        ("성의학", "", "", Some("8_성")),
        ("살림", "", "", Some("8_실용의학회계")),
        ("악보", "", "", Some("8_악보")),
        ("어린이", "", "", Some("9_어린이육아")),
        ("청소년문학", "", "", Some("9_청소년")),
        ("사주", "", "", Some("9_역학해몽퍼즐")),
        ("유머", "", "", Some("9_유머")),
        ("명언", "", "", Some("9_격언명언")),
    ];
    for (cat_str, raw_title, raw_author, expected) in cases {
        assert_eq!(map_category(cat_str, raw_title, raw_author).as_deref(), *expected, "case: ({cat_str:?}, {raw_title:?}, {raw_author:?})");
    }
}

#[test]
fn test_map_category_and_effective_filename_branches() {
    // 936: cat_str="거시경제 > 총론" -> leaf_cat="총론" (933 패스, 936 매칭)
    assert_eq!(map_category("거시경제 > 총론", "제목", "저자").as_deref(), Some("4_경제"));
    // 994: 영어, 영문법 (969의 영어회화 등이 없는 카테고리)
    assert_eq!(map_category("기타어학 > 기초 영문법", "제목", "저자").as_deref(), Some("7_영어교육"));
    // 1034: cat_str="지역경제 > 총론" -> 933/936 패스 후 1034 경제 폴백 매칭
    assert_eq!(map_category("지역경제 > 총론", "제목", "저자").as_deref(), Some("4_경제"));

    let tmp = TempDir::new().unwrap();
    let parent_dir = tmp.path().join("작품");
    fs::create_dir_all(&parent_dir).unwrap();
    let f = parent_dir.join("[저자] 제목 1권.txt");
    fs::write(&f, "").unwrap();
    assert_eq!(get_effective_filename(&f, tmp.path()), "[저자] 제목 1권.txt");
}

#[test]
fn map_category_does_not_classify_a_poetry_label_that_also_names_fiction() {
    assert_eq!(map_category("한국시 소설", "", ""), None);
}

#[test]
fn map_category_matches_python() {
    let f = fixture();
    let mut bad = Vec::new();
    for c in f["map_category"].as_array().unwrap() {
        let got = map_category(s(&c["cat"]), s(&c["title"]), s(&c["author"]));
        if got.as_deref() != c["expected"].as_str() {
            bad.push(format!("{:?} -> rust {got:?} python {}", c["cat"], c["expected"]));
        }
    }
    assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
}
