//! 하이브리드 장르 클러스터링 테스트 (tests/test_cluster_hybrid_genres.py 이식 +
//! tests/test_book_classifier.py 중 이 도구가 쓰는 함수의 테스트).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use cluster_hybrid_genres::book_classifier::{clean_empty_parent_dirs, extract_explicit_genre, get_effective_filename};
use cluster_hybrid_genres::*;
use serde_json::Value;

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

fn make_zip(path: &Path, docs: &[(&str, String)]) {
    let mut z = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, body) in docs {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

// ---- 본문 추출 ----

#[test]
fn extract_head_tail_words_compat_wrapper() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("test.txt");
    write(&f, &"무협 강호의 소림사 화산파 내공 진기 무림맹 ".repeat(50));
    let (h, t) = extract_head_tail_words(&f, 10, 10);
    assert!(!h.is_empty());
    assert_eq!(t, "");
}

#[test]
fn extract_content_head_tail_words_joins_both_ends() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("hybrid_test.txt");
    let head = format!("{}\n", "화산파 제자가 무공을 수련하며 강호를 방랑했다. ".repeat(30));
    let mid = format!("{}\n", "긴 여행의 중간 내용... ".repeat(100));
    let tail = "결국 그는 이계로 차원이동하여 드래곤과 마법사를 마주하고 마나를 각성했다. ".repeat(30);
    write(&f, &format!("{head}{mid}{tail}"));
    let combined = extract_content_head_tail_words(&f, 50, 50);
    assert!(combined.contains("화산파"));
    assert!(combined.contains("드래곤") || combined.contains("마법사"));
}

#[test]
fn txt_head_keeps_first_words_in_order() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("book.txt");
    write(&f, &"무협 강호의 소림사 화산파 내공 진기 무림맹 ".repeat(50));
    let text = extract_content_head_tail_words(&f, 10, 10);
    assert_eq!(text.split_whitespace().take(3).collect::<Vec<_>>(), ["무협", "강호의", "소림사"]);
    assert_eq!(extract_content_first_1000_words(&f), extract_content_head_tail_words(&f, 500, 500));
}

#[test]
fn txt_head_falls_back_to_cp949_when_utf8_has_no_hangul_run() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("old.txt");
    let text = "무협 강호의 소림사 화산파 내공 진기 무림맹 ".repeat(50);
    let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(&text);
    assert!(!had_errors);
    fs::write(&f, bytes).unwrap();
    assert!(extract_content_head_tail_words(&f, 10, 10).contains("소림사"));
}

#[test]
fn epub_head_and_tail_are_read_from_first_and_last_two_documents() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("sample.epub");
    let ch = |t: &str| format!("<html><body><p>{}</p></body></html>", t.repeat(20));
    make_zip(
        &p,
        &[
            ("mimetype", "application/epub+zip".into()),
            ("text/ch1.xhtml", ch("화산파 매화검법 수련 시작 ")),
            ("text/ch2.xhtml", ch("단전의 내공을 운기조식하다 ")),
            ("text/ch3.xhtml", ch("드래곤과 마주쳐 마나를 각성 ")),
            ("text/ch4.xhtml", ch("이세계 판타지 모험의 끝 ")),
        ],
    );
    let res = extract_content_head_tail_words(&p, 50, 50);
    assert!(res.contains("화산파"));
    assert!(res.contains("이세계"));
}

#[test]
fn broken_epub_and_empty_txt_give_empty_text() {
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("bad.epub");
    fs::write(&bad, b"not a zip file").unwrap();
    assert_eq!(extract_content_head_tail_words(&bad, 500, 500), "");
    let empty = d.path().join("empty.txt");
    fs::write(&empty, b"").unwrap();
    assert_eq!(extract_content_head_tail_words(&empty, 500, 500), "");
}

#[test]
fn epub_with_only_cover_like_names_falls_back_to_all_html() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("book.epub");
    let body = format!("<p>{}</p>", "소림사 화산파 내공 진기 무림맹 ".repeat(20));
    make_zip(&p, &[("OEBPS/cover.xhtml", body.clone()), ("OEBPS/nav.xhtml", body)]);
    assert!(extract_content_head_tail_words(&p, 10, 10).contains("소림사"));
}

// ---- 어휘 스코어링 ----

#[test]
fn calculate_genre_frequencies_compat_wrapper() {
    let res = calculate_genre_frequencies("화산귀환", "화산파 매화검법 내공", "소림사 무림맹");
    assert!(res.valid);
    assert!(res.scores["3_무협"] > 0);
}

#[test]
fn all_hybrid_clusters() {
    let cases: Vec<(&str, String, &str, Option<&str>)> = vec![
        ("무림속", format!("{} 던전 마나", "소림사 화산파 무당파 내공 진기 마교 천마 ".repeat(10)), "하이브리드_무협_판타지", Some("3_판타지")),
        ("판타지로맨스", format!("{}{}", "던전 몬스터 헌터 각성 ".repeat(5), " 영애 황태자 남주 여주 ".repeat(5)), "하이브리드_판타지_로판", None),
        ("판타지성인", format!("{}{}", "던전 몬스터 헌터 각성 ".repeat(5), " 섹스 자위 정액 클리토리스 음란 ".repeat(5)), "하이브리드_판타지_성인", None),
        ("로맨스성인", format!("{}{}", "로맨스 남주 여주 황태자 ".repeat(5), " 섹스 자위 정액 클리토리스 음란 ".repeat(5)), "하이브리드_로맨스_성인", None),
        ("무협성인", format!("{}{}", "소림사 화산파 내공 진기 ".repeat(5), " 섹스 자위 정액 클리토리스 음란 ".repeat(5)), "하이브리드_무협_성인", None),
        ("BL성인", format!("{}{}", "미인공 미남공 다정공 미인수 단정수 ".repeat(5), " 섹스 자위 정액 클리토리스 음란 ".repeat(5)), "하이브리드_BL_성인", None),
        ("BL판타지", format!("{}{}", "미인공 미남공 다정공 미인수 단정수 ".repeat(5), " 던전 몬스터 헌터 각성 ".repeat(5)), "하이브리드_BL_판타지", None),
        ("준확실", "로맨스 남주 여주 화산파 소림사 ".repeat(10), "준확실_3_여성향", Some("3_여성향")),
        ("무림판타지", format!("{}던전 마나", "화산파 소림사 무당파 내공 진기 마교 천마 ".repeat(5)), "하이브리드_무협_판타지", Some("3_판타지")),
    ];
    for (title, text, cluster, target) in cases {
        let res = calculate_5_genre_scores_and_ratios(title, &text);
        assert_eq!(res.cluster, cluster, "{title}");
        assert_eq!(res.target_cat.as_deref(), target, "{title}");
    }
}

#[test]
fn unclassified_clusters() {
    let en = calculate_5_genre_scores_and_ratios("English Book", "This is an english book without any hangul text.");
    assert_eq!((en.cluster.as_str(), en.valid, en.reason.as_deref()), ("미분류_비문학영문", false, Some("not_enough_hangul")));
    let zero = calculate_5_genre_scores_and_ratios("일반소설", "아침에 일어나서 밥을 먹고 학교에 갔다. 평범한 하루였다.");
    assert_eq!((zero.cluster.as_str(), zero.reason.as_deref()), ("미분류_키워드부족", Some("zero_score")));
    assert!(zero.ratios.values().all(|v| *v == 0.0));
}

#[test]
fn pure_cluster_for_dominant_genre() {
    let text = "소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 ".repeat(20);
    let res = calculate_5_genre_scores_and_ratios("정통무협", &text);
    assert_eq!((res.cluster.as_str(), res.target_cat.as_deref()), ("확실한_3_무협", Some("3_무협")));
}

#[test]
fn explicit_genre_titles_boost_and_zero_competitors() {
    let wuxia = "소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 ".repeat(5);
    let plain = calculate_5_genre_scores_and_ratios("제목", &wuxia).scores;
    let tagged = calculate_5_genre_scores_and_ratios("[무협] 제목", &wuxia).scores;
    assert!(tagged["3_무협"] >= plain["3_무협"] + 5);
    assert!(tagged["9_BLGL"] == 0 && tagged["3_여성향"] == 0);

    let fantasy = "던전 상태창 마나 스킬 몬스터 레이드 ".repeat(5);
    let plain = calculate_5_genre_scores_and_ratios("제목", &fantasy).scores;
    let tagged = calculate_5_genre_scores_and_ratios("[판타지] 제목", &fantasy).scores;
    assert!(tagged["3_판타지"] >= plain["3_판타지"] + 5);
    assert!(tagged["9_BLGL"] == 0 && tagged["3_여성향"] == 0);

    let other = "무림맹 화산파 ".repeat(3);
    let plain = calculate_5_genre_scores_and_ratios("제목", &other).scores;
    let tagged = calculate_5_genre_scores_and_ratios("[로맨스] 제목", &other).scores;
    assert!(tagged["3_여성향"] >= plain["3_여성향"] + 5);
    assert_eq!(tagged["3_무협"], plain["3_무협"]);
}

#[test]
fn ratios_round_like_python_and_keep_key_order() {
    let res = calculate_5_genre_scores_and_ratios("화산귀환", "화산파 매화검법 내공 소림사 무림맹 던전");
    assert_eq!(res.scores.keys().collect::<Vec<_>>(), ["3_무협", "3_판타지", "3_여성향", "9_BLGL", "9_성인"]);
    assert_eq!(res.ratios.keys().collect::<Vec<_>>(), ["3_무협", "3_판타지", "3_여성향", "9_BLGL", "9_성인"]);
    for r in res.ratios.values() {
        assert_eq!(*r, (r * 10.0).round() / 10.0);
    }
}

#[test]
fn clean_disclaimer_strips_legal_and_colophon_lines() {
    assert_eq!(clean_disclaimer_and_colophon(""), "");
    // Python 출력과 같다. 무단전재 문구는 줄 끝까지(본문 끝 포함), 판권 줄은 발행인부터 지운다.
    assert_eq!(clean_disclaimer_and_colophon("본문 시작 무단전재 및 재배포 금지 본문 끝\n발행인 홍길동\n본문"), "본문 시작  \n ");
}

#[test]
fn keyword_patterns_respect_lookaround_rules() {
    // 단전: '무단전'은 제외한다. 조사 목록에 '을'은 없어 "단전을"은 세지 않는다(원본과 같다).
    assert_eq!(count_genre_word_patterns("무단전 사용", &["단전"]), 0);
    assert_eq!(count_genre_word_patterns("그는 단전이 막혔다", &["단전"]), 1);
    assert_eq!(count_genre_word_patterns("단전을 모았다", &["단전"]), 0);
    // 진기: '민진기'와 '진기한'은 제외.
    assert_eq!(count_genre_word_patterns("민진기 진기한 이야기", &["진기"]), 0);
    assert_eq!(count_genre_word_patterns("진기를 운용", &["진기"]), 1);
    // 강수: 앞에 한글·영문이 붙으면 제외.
    assert_eq!(count_genre_word_patterns("김강수 강수가 왔다", &["강수"]), 1);
    // 마나: 앞뒤가 단어 경계일 때만.
    assert_eq!(count_genre_word_patterns("마나를 쓴다 마나리 ", &["마나"]), 1);
    // bl/gl: 독립 단어만.
    assert_eq!(count_genre_word_patterns("bl 소설 table gl", &["bl", "gl"]), 2);
    // 일반 키워드는 겹치지 않게 센다.
    assert_eq!(count_genre_word_patterns("무림 무림 무림맹", &["무림"]), 3);
}

// ---- backend/book_classifier.py에서 옮긴 유틸 ----

#[test]
fn extract_explicit_genre_table() {
    for (name, cat) in [
        ("[로판] 악녀는 오늘도", Some("3_여성향")),
        ("(무협) 광마회귀 1-100", Some("3_무협")),
        ("[판타지] 달빛조각사 01권", Some("3_판타지")),
        ("[BL] 사랑의 형태", Some("9_BLGL")),
        ("[SF] 은하영웅전설", Some("3_SF")),
        ("[라노벨] 소드아트온라인", Some("3_라이트노벨")),
        ("일반 소설 제목.txt", None),
    ] {
        assert_eq!(extract_explicit_genre(name), cat, "{name}");
    }
    // URL 인코딩된 파일명도 푼다.
    assert_eq!(extract_explicit_genre("%5B%EB%AC%B4%ED%98%91%5D%20%EC%A0%9C%EB%AA%A9"), Some("3_무협"));
}

#[test]
fn get_effective_filename_rules() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("library");
    let series = root.join("[전민희] 룬의 아이들 윈터러");
    fs::create_dir_all(&series).unwrap();
    let vol = series.join("01권.txt");
    write(&vol, "content");
    assert_eq!(get_effective_filename(&vol, &root), "[전민희] 룬의 아이들 윈터러 01권.txt");

    let direct = root.join("제목.txt");
    write(&direct, "content");
    assert_eq!(get_effective_filename(&direct, &root), "제목.txt");

    let s2 = root.join("[작가] 시리즈");
    fs::create_dir_all(&s2).unwrap();
    let chapter = s2.join("긴 제목.txt");
    write(&chapter, "content");
    assert_eq!(get_effective_filename(&chapter, &root), "[작가] 긴 제목.txt");
}

#[test]
fn clean_empty_parent_dirs_rules() {
    let d = tempfile::tempdir().unwrap();
    let stop = d.path().join("library");
    let empty_child = stop.join("series/volume");
    fs::create_dir_all(&empty_child).unwrap();
    clean_empty_parent_dirs(&empty_child, &stop);
    assert!(stop.exists() && !stop.join("series").exists());

    let child = stop.join("series");
    fs::create_dir_all(&child).unwrap();
    write(&child.join("keep.txt"), "c");
    clean_empty_parent_dirs(&child, &stop);
    assert!(child.exists());

    // rmdir은 숨김 파일까지 봐야 성공한다.
    fs::remove_file(child.join("keep.txt")).unwrap();
    write(&child.join(".nomedia"), "");
    clean_empty_parent_dirs(&child, &stop);
    assert!(child.exists());
}

#[test]
fn clean_empty_parent_dirs_never_climbs_past_unnormalized_stop_dir() {
    let d = tempfile::tempdir().unwrap();
    let stop = d.path().join("library");
    let child = stop.join("series");
    fs::create_dir_all(&child).unwrap();
    clean_empty_parent_dirs(&child, &stop.join("series").join(".."));
    assert!(stop.exists() && d.path().exists());
}

// ---- CLI ----

struct Run {
    code: i32,
    out: String,
}

fn run_cli(argv: &[&str]) -> Run {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = cli::run(&args, &mut out);
    Run { code, out: String::from_utf8(out).unwrap() }
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

struct Dirs {
    _d: tempfile::TempDir,
    src: PathBuf,
    lib: PathBuf,
    report: PathBuf,
}

fn dirs() -> Dirs {
    // 원본은 경로 구성요소가 '.'으로 시작하면 파일을 뺀다. tempfile의 기본 이름(.tmpXXXX)을 피한다.
    let d = tempfile::Builder::new().prefix("cluster").tempdir().unwrap();
    let (src, lib) = (d.path().join("source"), d.path().join("library"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&lib).unwrap();
    let report = d.path().join("report.json");
    Dirs { _d: d, src, lib, report }
}

#[test]
fn cli_workflow_reports_clusters_and_progress() {
    let x = dirs();
    let samples = [
        ("정통무협", "소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 ".repeat(20)),
        ("무림속던전", "화산파 내공 검법 던전 상태창 마나 스킬 몬스터 레이드 ".repeat(20)),
        ("준확실로맨스", "로맨스 남주 여주 화산파 소림사 ".repeat(10)),
        ("기타문서", "가나다라마바사 아무런 키워드가 없는 일반적인 글입니다. ".repeat(15)),
    ];
    for i in 0..250 {
        for (name, text) in &samples {
            write(&x.src.join(format!("{name}{i}.txt")), text);
        }
    }
    let r = run_cli(&["--source-dir", p(&x.src), "--library-root", p(&x.lib), "--report", p(&x.report)]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("클러스터링 분석 완료") && r.out.contains("[1000/1000] 분석 진행 중..."));
    let report: Value = serde_json::from_str(&fs::read_to_string(&x.report).unwrap()).unwrap();
    assert_eq!(report["total_files"], 1000);
    assert_eq!(report["cluster_counts"]["확실한_3_무협"], 250);
    assert_eq!(report["cluster_counts"]["하이브리드_무협_판타지"], 250);
    assert!(report["clusters"]["확실한_3_무협"][0]["ratios"].is_object());
    // 이동 옵션이 없으면 원본은 그대로다.
    assert!(x.src.join("정통무협0.txt").exists());
}

#[test]
fn cli_auto_move_pure_moves_new_and_cleans_duplicates() {
    let x = dirs();
    let dest = x.lib.join("3_무협");
    fs::create_dir_all(&dest).unwrap();
    write(&dest.join("중복무협.txt"), "기존 파일");
    let text = "소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 ".repeat(20);
    write(&x.src.join("중복무협.txt"), &text);
    write(&x.src.join("신규무협.txt"), &text);
    let r = run_cli(&["--source-dir", p(&x.src), "--library-root", p(&x.lib), "--auto-move-pure", "--clean-existing", "--limit", "2", "--report", p(&x.report)]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("자동 이동 결과") && r.out.contains("신규 이동: 1권, 중복 정리: 1권"));
    assert!(!x.src.join("중복무협.txt").exists() && !x.src.join("신규무협.txt").exists());
    assert!(dest.join("신규무협.txt").exists());
    assert_eq!(fs::read_to_string(dest.join("중복무협.txt")).unwrap(), "기존 파일");
}

#[test]
fn cli_move_hybrids_to_target_category() {
    let x = dirs();
    let hybrid = "화산파 내공 검법 던전 상태창 마나 스킬 몬스터 레이드 ".repeat(20);
    let dest = x.lib.join("3_판타지");
    fs::create_dir_all(&dest).unwrap();
    write(&dest.join("중복하이브리드.txt"), "기존 파일");
    write(&x.src.join("중복하이브리드.txt"), &hybrid);
    write(&x.src.join("신규하이브리드.txt"), &hybrid);
    let r = run_cli(&["--source-dir", p(&x.src), "--library-root", p(&x.lib), "--move-hybrids-to", "3_판타지", "--clean-existing", "--limit", "2", "--report", p(&x.report)]);
    assert_eq!(r.code, 0);
    assert!(!x.src.join("중복하이브리드.txt").exists() && !x.src.join("신규하이브리드.txt").exists());
    assert!(dest.join("신규하이브리드.txt").exists());
}

#[test]
fn cli_single_file_with_defaults_writes_report() {
    let x = dirs();
    write(&x.src.join("test.txt"), "무협 소림사");
    let r = run_cli(&["--source-dir", p(&x.src), "--report", p(&x.report), "--limit", "1"]);
    assert_eq!(r.code, 0);
    assert!(x.report.exists());
}

#[test]
fn cli_ignores_hidden_directories_and_other_extensions() {
    let x = dirs();
    fs::create_dir_all(x.src.join(".cache")).unwrap();
    write(&x.src.join(".cache/a.txt"), "무협 소림사");
    write(&x.src.join("b.docx"), "x");
    write(&x.src.join("c.txt"), "무협 소림사");
    let r = run_cli(&["--source-dir", p(&x.src), "--report", p(&x.report)]);
    assert!(r.out.contains("총 1권"));
}

#[test]
fn cli_empty_source_does_not_divide_by_zero() {
    let x = dirs();
    let r = run_cli(&["--source-dir", p(&x.src), "--report", p(&x.report)]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("총 0권"));
}

#[test]
fn cli_bad_arguments_exit_2_and_unwritable_report_exits_1() {
    let x = dirs();
    assert_eq!(run_cli(&["--bogus"]).code, 2);
    assert_eq!(run_cli(&["--limit", "x"]).code, 2);
    assert_eq!(run_cli(&["--help"]).code, 0);
    assert_eq!(run_cli(&["--source-dir", p(&x.src), "--report", "/nonexistent-dir/r.json"]).code, 1);
}

#[test]
fn py_float_matches_python_repr() {
    assert_eq!(py_float(100.0), "100.0");
    assert_eq!(py_float(66.7), "66.7");
    assert_eq!(py_float(0.0), "0.0");
}
