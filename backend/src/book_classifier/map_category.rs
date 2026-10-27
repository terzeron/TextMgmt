// 생성 파일이라 Python 원본과 줄 단위로 대응시킨다. 중첩 if 를 합치지 않는다.
#![allow(clippy::collapsible_if)]
//! 생성 파일: `backend/book_classifier.py` 의 `map_category` 를 줄 단위로 옮겼다. 검사 순서가 곧 우선순위다.

fn any(hay: &str, words: &[&str]) -> bool {
    words.iter().any(|w| hay.contains(w))
}

/// 서점 카테고리 문자열(과 제목·저자)을 내부 카테고리 하나로 옮긴다. 규칙에 안 걸리면 `None`.
pub fn map_category(cat_str: &str, raw_title: &str, raw_author: &str) -> Option<String> {
    if cat_str.is_empty() && raw_title.is_empty() && raw_author.is_empty() {
        return None;
    }
    let cat = cat_str;
    let parts: Vec<&str> = if cat.is_empty() { Vec::new() } else { cat.split('>').map(str::trim).collect() };
    let leaf_owned = if parts.len() > 1 { parts[1..].join(" > ") } else { cat.to_string() };
    let leaf_cat = leaf_owned.as_str();
    // 1. 저자/시리즈 고유 규칙
    if raw_author.contains("히가시노 게이고") || raw_author.contains("히가시노게이고") {
        return Some("2_소설일본게이고".to_string());
    }
    if raw_author.contains("무라카미 하루키") || raw_author.contains("무라카미하루키") {
        return Some("2_소설일본하루키".to_string());
    }
    if raw_title.contains("을유세계문학") || cat.contains("을유세계문학") {
        return Some("2_을유세계문학전집".to_string());
    }
    if raw_title.contains("열린책들 세계문학") || raw_title.contains("열린책들세계문학") || raw_title.contains("열린책들") {
        return Some("2_열린책들세계문학".to_string());
    }
    if raw_title.contains("문예세계문학선") {
        return Some("2_문예세계문학선".to_string());
    }
    if raw_title.contains("살림지식총서") {
        return Some("4_살림지식총서".to_string());
    }
    if raw_title.contains("시공디스커버리") {
        return Some("4_시공디스커버리".to_string());
    }
    if raw_title.contains("올재 클래식스") || raw_title.contains("(올재") || raw_title.contains("올재") {
        return Some("1_올재".to_string());
    }
    if raw_title.contains("누워서 읽는 법학") || raw_title.contains("누워서읽는법학") {
        return Some("4_누워서읽는법학".to_string());
    }
    if raw_title.contains("이지사이언스") {
        return Some("5_이지사이언스".to_string());
    }
    if raw_title.contains("그림으로 읽는") || raw_title.contains("그림으로읽는") {
        return Some("5_그림으로_읽는".to_string());
    }
    if any(raw_title, &["손자병법", "육도, 삼략", "육도삼략", "한비자", "장자", "채근담", "논어", "맹자"]) {
        return Some("1_동양고전".to_string());
        // 2. 카테고리 매핑 규칙
    }
    if any(cat, &["라이트노벨", "라이트 노벨", "라노벨"]) {
        return Some("3_라이트노벨".to_string());
    }
    if any(cat, &["BL", "GL", "야오이", "백합", "BL/GL", "퀴어"]) {
        return Some("9_BLGL".to_string());
    }
    if any(cat, &["그래픽노블", "그래픽 노블", "만화/코믹", "코믹스", "웹툰", "만화"]) {
        return Some("3_그래픽노블".to_string());
    }
    if any(cat, &["추리", "미스터리", "스릴러", "서스펜스", "공포소설", "호러", "하드보일드"]) {
        return Some("3_스릴러".to_string());
    }
    if any(cat, &["과학소설", "사이버펑크"]) || (cat.contains("SF") && leaf_cat.contains("SF")) {
        return Some("3_SF".to_string());
    }
    if any(cat, &["판타지소설", "판타지", "현대판타지", "게임소설", "퓨전판타지", "현대", "퓨전"]) {
        if !cat.contains("SF") && !cat.contains("로맨스") && !cat.contains("로맨틱판타지") {
            return Some("3_판타지".to_string());
        }
    }
    if any(cat, &["무협", "신무협", "무협소설"]) {
        return Some("3_무협".to_string());
    }
    if any(cat, &["로맨스", "로판", "로맨스판타지", "로맨틱판타지", "사랑소설", "연애소설", "칙릿"]) {
        return Some("3_여성향".to_string());
    }
    if any(cat, &["에세이", "수필", "일기", "서간", "산문집"]) {
        return Some("2_수필서간일기".to_string());
    }
    if any(cat, &["한국소설", "한국단편", "한국장편", "한국 문학", "한국문학"]) {
        return Some("2_소설한국".to_string());
    }
    if any(cat, &["일본소설", "일본문학", "일본장편", "일본단편"]) {
        return Some("2_소설일본".to_string());
    }
    if any(cat, &["중국소설", "중국문학", "대만소설"]) {
        return Some("2_소설중국".to_string());
    }
    if any(cat, &["영미소설", "영국소설", "미국소설", "프랑스소설", "독일소설", "러시아소설", "유럽소설", "스페인소설", "남미소설", "외국소설", "세계의 소설", "각국소설", "북유럽소설", "고전문학"])
    {
        return Some("2_소설외국".to_string());
    }
    if any(cat, &["역사소설", "대하역사소설", "대하소설"]) {
        return Some("2_소설역사".to_string());
    }
    if any(cat, &["시집", "한국시", "외국시", "현대시", "희곡", "시/희곡"]) {
        if !cat.contains("소설") && !cat.contains("에세이") {
            return Some("2_시".to_string());
        }
    }
    if any(cat, &["독서 에세이", "글쓰기", "작법", "문학비평", "문학이론", "서평", "책읽기", "문학의 이해", "독서법"]) {
        return Some("2_문학일반서평작법독서".to_string());
    }
    if any(cat, &["문헌정보", "서지학", "기록관리", "문헌학"]) {
        return Some("1_문헌서지".to_string());
    }
    if any(cat, &["동양고전", "사서삼경", "제자백가", "한문학", "한문"]) {
        return Some("1_동양고전".to_string());
    }
    if any(cat, &["서양고전", "그리스로마", "그리스 로마"]) {
        return Some("1_서양고전".to_string());
    }
    if any(cat, &["투자", "재테크", "주식", "증권", "부동산", "경매", "가상화폐", "비트코인", "자산관리", "금융상품", "펀드", "연금", "환테크", "청약"]) {
        return Some("6_재테크".to_string());
    }
    if any(leaf_cat, &["경제학", "경제일반", "경제전망", "경제사", "각국 경제", "금융/화폐", "국제경제", "화폐", "거시경제", "미시경제", "행동경제학", "경제"])
    {
        return Some("4_경제".to_string());
    }
    if any(cat, &["경제학", "경제사", "각국 경제", "거시경제", "미시경제", "행동경제학"]) {
        return Some("4_경제".to_string());
    }
    if any(cat, &["마케팅", "브랜딩", "광고", "창업", "스타트업", "경영전략", "경영일반", "비즈니스", "조직관리", "e-비즈니스", "기획", "세일즈", "CEO"]) {
        return Some("4_경영마케팅".to_string());
    }
    if any(leaf_cat, &["경영"]) {
        return Some("4_경영마케팅".to_string());
    }
    if any(cat, &["자기계발", "성공/처세", "처세술", "인간관계", "화술", "대화법", "시간관리", "습관", "동기부여", "창의성", "성공학", "마인드셋", "리더십"])
    {
        return Some("6_처세술리더십창의성".to_string());
    }
    if any(cat, &["심리", "심리학", "정신분석", "뇌과학", "상담심리", "임상심리", "인지심리"]) {
        return Some("4_심리학뇌과학".to_string());
    }
    if any(cat, &["철학", "서양철학", "동양철학", "윤리학", "사상", "현대철학"]) {
        return Some("4_철학윤리".to_string());
    }
    if any(cat, &["한국사", "동양사", "서양사", "세계사", "역사인물", "역사학", "평전", "전기", "조선사", "고려사", "근현대사", "역사", "조선시대"]) {
        return Some("4_역사인물".to_string());
    }
    if any(cat, &["정치", "외교", "군사", "국제정치", "안보", "통일", "국방", "정치학"]) {
        return Some("4_정치외교군사".to_string());
    }
    if any(cat, &["법학", "법률", "헌법", "형법", "민법", "소송", "행정/법률"]) {
        return Some("4_법".to_string());
    }
    if any(cat, &["사회학", "사회문제", "언론", "미디어", "인류학", "문화인류", "여성학", "페미니즘", "노동", "사회과학"]) {
        return Some("4_사회인류".to_string());
    }
    if any(cat, &["종교", "기독교", "불교", "가톨릭", "천주교", "이슬람", "성경", "신앙", "신화"]) {
        return Some("4_종교신화".to_string());
    }
    if any(cat, &["인문학", "교양인문", "인문일반", "인문", "논픽션"]) {
        return Some("4_인문일반논픽션".to_string());
    }
    if any(cat, &["수학", "물리학", "화학", "생물학", "생명과학", "지구과학", "천문학", "자연과학", "교양과학", "기초과학", "과학"]) {
        return Some("5_수학과학일반".to_string());
        // 구체적인 복합 카테고리를 먼저 판정한다. 아래의 broad keyword(회화/국어/교육/의학/영어)
        // 보다 늦게 검사하면 substring match 때문에 다른 분야로 오분류된다.
    }
    if any(cat, &["영어원서", "영문원서", "영한대역"]) {
        return Some("7_영문일반".to_string());
    }
    if any(cat, &["영어회화", "토익", "토플", "수능영어", "영단어"]) {
        return Some("7_영어교육".to_string());
    }
    if any(cat, &["중국어", "HSK", "한자"]) {
        return Some("7_중어한자교육".to_string());
    }
    if any(cat, &["성교육", "성의학"]) {
        return Some("8_성".to_string());
    }
    if any(cat, &["미술", "예술", "건축", "디자인", "공예", "조각", "회화", "도예"]) {
        return Some("5_미술예술건축".to_string());
    }
    if any(cat, &["음악", "클래식음악", "대중음악", "작곡", "악기", "가요", "뮤지컬"]) {
        return Some("5_음악".to_string());
    }
    if any(cat, &["영화", "시나리오", "각본", "영화사", "드라마 대본"]) {
        return Some("5_영화".to_string());
    }
    if any(cat, &["사진", "영상제작", "카메라", "영상편집"]) {
        return Some("5_사진영상".to_string());
    }
    if any(cat, &["스포츠", "골프", "축구", "야구", "마라톤", "헬스", "피트니스", "수영", "등산", "운동"]) {
        return Some("5_스포츠".to_string());
    }
    if any(cat, &["서브컬쳐", "캐릭터 드로잉", "일러스트", "만화작법"]) {
        return Some("5_서브컬쳐".to_string());
    }
    if any(cat, &["교육학", "학습법", "공부법", "독서지도", "교육"]) {
        return Some("7_교육일반".to_string());
    }
    if any(cat, &["국어", "한국어", "맞춤법", "어휘"]) {
        return Some("7_국어교육".to_string());
    }
    if any(cat, &["영어", "영문법"]) {
        return Some("7_영어교육".to_string());
    }
    if any(cat, &["일본어", "일어", "JLPT"]) {
        return Some("7_일어교육".to_string());
    }
    if any(cat, &["외국어", "프랑스어", "독일어", "스페인어", "러시아어"]) {
        return Some("7_외국어교육".to_string());
    }
    if any(cat, &["언어학", "번역", "통역"]) {
        return Some("7_언어일반".to_string());
    }
    if any(cat, &["컴퓨터", "IT", "프로그래밍", "소프트웨어", "코딩", "인공지능", "AI", "빅데이터", "네트워크", "모바일/태블릿", "웹개발", "데이터베이스"]) {
        return Some("8_IT".to_string());
    }
    if any(cat, &["건강", "의학", "질병", "치료", "한의학", "다이어트", "영양", "수면", "면역", "당뇨", "치매"]) {
        return Some("8_건강일반".to_string());
    }
    if any(cat, &["요리", "베이킹", "레시피", "와인", "커피", "음료", "디저트", "주류"]) {
        return Some("8_요리음료".to_string());
    }
    if any(cat, &["여행", "국내여행", "해외여행", "여행에세이", "가이드북"]) {
        return Some("8_여행".to_string());
    }
    if any(cat, &["공인중개사"]) {
        return Some("8_공인중개사".to_string());
    }
    if any(cat, &["프라모델", "모형", "모델링"]) {
        return Some("8_모델링".to_string());
    }
    if any(cat, &["밀리터리", "무기", "군사무기"]) {
        return Some("8_밀리터리".to_string());
    }
    if any(cat, &["살림", "인테리어", "수예", "뜨개질", "원예", "반려동물", "가정살림", "가사"]) {
        return Some("8_실용의학회계".to_string());
    }
    if any(cat, &["악보", "스코어", "Songbook"]) {
        return Some("8_악보".to_string());
    }
    if any(cat, &["어린이", "유아", "그림책", "동화책", "동화", "육아", "자녀교육", "부모"]) {
        return Some("9_어린이육아".to_string());
    }
    if any(cat, &["청소년", "청소년문학", "청소년교양", "청소년 인문"]) {
        return Some("9_청소년".to_string());
    }
    if any(cat, &["사주", "타로", "점성술", "풍수", "해몽", "퍼즐", "스도쿠", "바둑"]) {
        return Some("9_역학해몽퍼즐".to_string());
    }
    if any(cat, &["유머", "개그", "만담"]) {
        return Some("9_유머".to_string());
    }
    if any(cat, &["명언", "격언", "아포리즘"]) {
        return Some("9_격언명언".to_string());
    }
    if cat.contains("경제") {
        return Some("4_경제".to_string());
    }
    None
}
