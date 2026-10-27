//! Python PDF 편집 계약: 원본 페이지 번호, 백업 보존, 원자적 교체, 재색인.
mod common;

use backend::book_manager::{BookManager, BookManagerDeps};
use base64::Engine;
use common::{
    es_fake::FakeEsManager,
    faulty_fs::FaultyFs,
    pdf::{make_pdf, widths},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};

struct Env {
    bm: BookManager,
    es: Arc<FakeEsManager>,
    fs: Arc<FaultyFs>,
    pdf: PathBuf,
    root: tempfile::TempDir,
}

fn env() -> Env {
    common::setup_env();
    let root = tempfile::tempdir().unwrap();
    let pdf = root.path().join("cat/book.pdf");
    make_pdf(&pdf, 5);
    let es = FakeEsManager::with_doc(
        json!({"category":"cat", "title":"Title", "author":"Author", "file_path":"cat/book.pdf", "file_type":"pdf", "file_size": 0, "page_count":5, "updated_time":"2024-01-01T00:00:00.000000"}),
    );
    let fs = FaultyFs::new();
    let bm = BookManager::with_deps(BookManagerDeps::new(root.path().to_path_buf(), es.clone()).with_fs(fs.clone()));
    Env { bm, es, fs, pdf, root }
}

impl Env {
    async fn edit(&self, pages: &[i64], order: Option<&[i64]>) -> (Option<Value>, Option<String>) {
        self.bm.delete_pdf_pages(1, pages, None, None, None, order).await
    }
    fn backup(&self) -> PathBuf {
        self.root.path().join("0_trash/book.pdf")
    }
    fn assert_no_parts(&self) {
        for dir in [self.pdf.parent().unwrap().to_path_buf(), self.root.path().join("0_trash")] {
            if let Ok(entries) = fs::read_dir(dir) {
                assert!(entries.flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
            }
        }
    }
}

#[tokio::test]
async fn removes_original_page_numbers_and_reindexes_same_id() {
    let e = env();
    let original = fs::read(&e.pdf).unwrap();
    let (result, error) = e.edit(&[2, 4, 2], None).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap(), json!({"remaining_pages":3,"cropped_pages":0,"split_pages":0,"inserted_pages":0,"backup":"0_trash/book.pdf","backup_created":true}));
    assert_eq!(widths(&e.pdf), [100, 102, 104]);
    assert_eq!(fs::read(e.backup()).unwrap(), original);
    e.es.with(|s| {
        assert_eq!(s.deleted, [1]);
        let doc = &s.inserted[0][&1];
        assert_eq!(doc["page_count"], 3);
        assert_eq!(doc["file_size"], fs::metadata(&e.pdf).unwrap().len());
        for (key, value) in [("category", "cat"), ("title", "Title"), ("author", "Author"), ("file_path", "cat/book.pdf"), ("file_type", "pdf")] {
            assert_eq!(doc[key], value);
        }
    });
    e.assert_no_parts();
}

#[tokio::test]
async fn reorder_prefix_preserves_tail_and_uses_original_numbers_for_deletion() {
    let e = env();
    let (result, error) = e.edit(&[2], Some(&[3, 1, 2])).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["remaining_pages"], 4);
    assert_eq!(widths(&e.pdf), [102, 100, 103, 104]);
}

#[tokio::test]
async fn repeated_edit_never_overwrites_first_backup() {
    let e = env();
    let original = fs::read(&e.pdf).unwrap();
    assert_eq!(e.edit(&[2], None).await.1, None);
    let (result, error) = e.edit(&[2], None).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["backup_created"], false);
    assert_eq!(widths(&e.pdf), [100, 103, 104]);
    assert_eq!(fs::read(e.backup()).unwrap(), original);
}

#[tokio::test]
async fn invalid_edits_leave_file_backup_and_index_untouched() {
    for (pages, order, expected) in [
        (vec![], None, "변경할 쪽을 선택하세요"),
        (vec![0], None, "페이지 범위가 잘못되었습니다 (1~5)"),
        (vec![6], None, "페이지 범위가 잘못되었습니다 (1~5)"),
        (vec![1, 2, 3, 4, 5], None, "모든 페이지를 삭제할 수 없습니다"),
        (vec![], Some(vec![2, 2]), "쪽 순서가 잘못되었습니다"),
        (vec![], Some(vec![1, 3]), "쪽 순서가 잘못되었습니다"),
        (vec![], Some(vec![1, 2, 3, 4, 5, 6]), "쪽 순서가 잘못되었습니다"),
    ] {
        let e = env();
        let original = fs::read(&e.pdf).unwrap();
        let (result, error) = e.edit(&pages, order.as_deref()).await;
        assert_eq!(result, None);
        assert_eq!(error.as_deref(), Some(expected));
        assert_eq!(fs::read(&e.pdf).unwrap(), original);
        assert!(!e.backup().exists());
        e.es.with(|s| {
            assert!(s.deleted.is_empty());
            assert!(s.inserted.is_empty());
        });
        e.assert_no_parts();
    }
}

#[tokio::test]
async fn replace_failure_keeps_original_and_backup_and_cleans_parts() {
    let e = env();
    let original = fs::read(&e.pdf).unwrap();
    let path = e.pdf.clone();
    e.fs.set_rename(move |_, to| (to == path).then(|| std::io::Error::other("replace failed")));
    let (result, error) = e.edit(&[2], None).await;
    assert_eq!(result, None);
    assert!(error.unwrap().contains("replace failed"));
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
    assert_eq!(fs::read(e.backup()).unwrap(), original);
    e.es.with(|s| assert!(s.deleted.is_empty()));
    e.assert_no_parts();
}

#[tokio::test]
async fn reindex_failure_reports_backup_and_invalidates_only_this_cover() {
    let e = env();
    let cache = e.root.path().join(".cover_cache");
    fs::create_dir_all(&cache).unwrap();
    for name in ["1.v1.jpg", "1.v2.jpg", "2.v1.jpg"] {
        fs::write(cache.join(name), b"old").unwrap();
    }
    let previews = e.root.path().join(".preview_cache");
    fs::create_dir_all(&previews).unwrap();
    for name in ["1.pdf", "1_p1-2.pdf", "2.pdf", "2_p1-2.pdf"] {
        fs::write(previews.join(name), b"old").unwrap();
    }
    e.es.fail("insert", "offline");
    let (result, error) = e.edit(&[2], None).await;
    assert_eq!(result, None);
    let error = error.unwrap();
    assert!(error.contains("색인 갱신에 실패했습니다"), "{error}");
    assert!(error.contains("0_trash/book.pdf"));
    assert!(error.contains("offline"));
    assert_eq!(widths(&e.pdf), [100, 102, 103, 104]);
    assert!(!cache.join("1.v1.jpg").exists());
    assert!(!cache.join("1.v2.jpg").exists());
    assert!(cache.join("2.v1.jpg").exists());
    assert!(!previews.join("1.pdf").exists());
    assert!(!previews.join("1_p1-2.pdf").exists());
    assert!(previews.join("2.pdf").exists());
    assert!(previews.join("2_p1-2.pdf").exists());
}

#[tokio::test]
async fn reorder_keeps_metadata_and_inherited_page_attributes() {
    let e = env();
    let mut pdf = lopdf::Document::load(&e.pdf).unwrap();
    let info = pdf.add_object(lopdf::dictionary! {"Title" => lopdf::Object::string_literal("Example")});
    pdf.trailer.set("Info", info);
    let first = pdf.get_pages()[&1];
    let parent = pdf.get_object(first).unwrap().as_dict().unwrap().get(b"Parent").unwrap().as_reference().unwrap();
    pdf.get_object_mut(parent).unwrap().as_dict_mut().unwrap().set("Rotate", 90);
    pdf.get_object_mut(parent).unwrap().as_dict_mut().unwrap().set("CropBox", vec![0.into(), 0.into(), 50.into(), 60.into()]);
    pdf.save(&e.pdf).unwrap();
    assert_eq!(e.edit(&[], Some(&[2, 1])).await.1, None);
    let pdf = lopdf::Document::load(&e.pdf).unwrap();
    assert_eq!(pdf.get_dictionary(info).unwrap().get(b"Title").unwrap().as_str().unwrap(), b"Example");
    for id in pdf.get_pages().values() {
        let page = pdf.get_dictionary(*id).unwrap();
        assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
        assert_eq!(page.get(b"CropBox").unwrap().as_array().unwrap().len(), 4);
    }
}

#[tokio::test]
async fn crop_removes_pixels_and_resizes_only_selected_page() {
    let e = env();
    let img = image::RgbImage::from_fn(100, 60, |x, y| image::Rgb([x as u8, y as u8, 0]));
    common::pdf::write_image_pdf(&e.pdf, &[img.clone(), img]);
    let crops = [json!({"page":1,"x0":0.2,"y0":0.1,"x1":0.8,"y1":0.9})];
    let (result, error) = e.bm.delete_pdf_pages(1, &[], Some(&crops), None, None, None).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["cropped_pages"], 1);
    assert_eq!(common::pdf::mediabox_size(&e.pdf, 1), (60.0, 48.0));
    assert_eq!(common::pdf::mediabox_size(&e.pdf, 2), (100.0, 60.0));
    let doc = common::pdf::load(&e.pdf);
    let stream = common::pdf::first_xobject(&doc, 1);
    let data = stream.get_plain_content().unwrap();
    assert_eq!(&data[..3], &[20, 6, 0]);
    assert_eq!(data.len(), 60 * 48 * 3);
    let second = common::pdf::first_xobject(&doc, 2).get_plain_content().unwrap();
    assert_eq!(&second[..3], &[0, 0, 0]);
}

#[tokio::test]
async fn split_keeps_both_halves_together_when_reordered() {
    let e = env();
    let img = image::RgbImage::from_fn(100, 60, |x, y| image::Rgb([x as u8, y as u8, 0]));
    common::pdf::write_image_pdf(&e.pdf, &[img.clone(), img]);
    let splits = [json!({"page":1,"first":{"x0":0.0,"y0":0.0,"x1":0.4,"y1":1.0},"second":{"x0":0.4,"y0":0.0,"x1":1.0,"y1":1.0}})];
    let (result, error) = e.bm.delete_pdf_pages(1, &[], None, Some(&splits), None, Some(&[2, 1])).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["split_pages"], 1);
    assert_eq!(widths(&e.pdf), [100, 40, 60]);
    let doc = common::pdf::load(&e.pdf);
    assert_eq!(&common::pdf::first_xobject(&doc, 3).get_plain_content().unwrap()[..3], &[40, 0, 0]);
}

#[tokio::test]
async fn inserted_page_survives_removal_of_its_reference_page() {
    let e = env();
    let image = common::image::png_solid(80, 160, [10, 20, 30]);
    let inserts = [json!({"page":2,"image_data":base64::engine::general_purpose::STANDARD.encode(image)})];
    let (result, error) = e.bm.delete_pdf_pages(1, &[2], None, None, Some(&inserts), Some(&[2, 1])).await;
    assert_eq!(error, None);
    let result = result.unwrap();
    assert_eq!(result["remaining_pages"], 5);
    assert_eq!(result["inserted_pages"], 1);
    assert_eq!(widths(&e.pdf), [101, 100, 102, 103, 104]);
    assert_eq!(common::pdf::mediabox_size(&e.pdf, 1), (101.0, 202.0));
}

#[tokio::test]
async fn unsupported_crop_and_bad_image_preserve_original_before_backup() {
    for (crops, inserts, expected) in [
        (Some(vec![json!({"page":1,"x0":0.0,"y0":0.0,"x1":1.0,"y1":1.0})]), None, "크롭할 수 없습니다"),
        (None, Some(vec![json!({"page":1,"image_data":"@@@@"})]), "이미지 데이터를 읽을 수 없습니다"),
        (None, Some(vec![json!({"page":1,"image_data":"YWJj"})]), "이미지 형식을 읽을 수 없습니다"),
    ] {
        let e = env();
        let original = fs::read(&e.pdf).unwrap();
        let (result, error) = e.bm.delete_pdf_pages(1, &[], crops.as_deref(), None, inserts.as_deref(), None).await;
        assert_eq!(result, None);
        assert!(error.as_deref().unwrap_or("").contains(expected), "{error:?}");
        assert_eq!(fs::read(&e.pdf).unwrap(), original);
        assert!(!e.backup().exists());
        e.assert_no_parts();
    }
}

#[tokio::test]
async fn http_edits_books_and_comics_with_python_response_contract() {
    use backend::main::{AppDeps, AuthMode, app_with};
    for prefix in ["", "/comics"] {
        let e = env();
        let mut deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"})));
        if prefix.is_empty() {
            deps = deps.with_book_manager(Arc::new(e.bm));
        } else {
            deps = deps.with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()))));
        }
        let client = common::TestClient::with_app(app_with(deps));
        let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":[2,4]})).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.json()["status"], "success");
        assert_eq!(response.json()["result"]["remaining_pages"], 3);
        assert_eq!(widths(&e.pdf), [100, 102, 104]);
        let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":[99]})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["status"], "failure");
    }
}

#[tokio::test]
async fn http_rejects_non_admin_before_editing_or_body_validation() {
    use backend::main::{AppDeps, AuthMode, app_with};
    for prefix in ["", "/comics"] {
        for viewer in [false, true] {
            let e = env();
            let original = fs::read(&e.pdf).unwrap();
            let bm = Arc::new(e.bm);
            let mut client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(bm.clone()).with_comics_manager(bm)));
            if viewer {
                let jwt = backend::auth::create_jwt_token("viewer@test.com", "viewer", None, None);
                client = client.cookie("tm_access_token", &jwt);
            }
            let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":"invalid"})).await;
            assert_eq!(response.status, if viewer { 403 } else { 401 }, "{}", response.text());
            assert_eq!(fs::read(&e.pdf).unwrap(), original);
        }
    }
}

#[tokio::test]
async fn http_validates_edit_shapes_before_opening_pdf() {
    use backend::main::{AppDeps, AuthMode, app_with};
    let e = env();
    let original = fs::read(&e.pdf).unwrap();
    let client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(Arc::new(e.bm))));
    for body in [
        json!({"pages":"1"}),
        json!({"crops":[{"page":1,"x0":0.0}]}),
        json!({"splits":[{"page":1}]}),
        json!({"inserts":[{"page":1}]}),
        json!({"inserts":[{"page":1,"url":"https://example.com/x","image_data":"AAAA"}]}),
    ] {
        let response = client.post_json("/pdf-pages/1/delete", &body).await;
        assert_eq!(response.status, 422, "{body}: {}", response.text());
    }
    assert_eq!(client.post_json("/pdf-pages/no-id/delete", &json!({})).await.status, 422);
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
}

#[tokio::test]
async fn crop_preserves_lossless_pixels_and_one_bit_row_padding() {
    for kind in ["rgb", "rgb_png", "gray", "bw", "palette"] {
        let e = env();
        common::pdf::make_pixel_pdf(&e.pdf, &[kind]);
        let crops = [json!({"page":1,"x0":0.25,"y0":0.0,"x1":0.75,"y1":1.0})];
        let (_, error) = e.bm.delete_pdf_pages(1, &[], Some(&crops), None, None, None).await;
        assert_eq!(error, None, "{kind}");
        let (actual, info) = common::pdf::pixel_image(&e.pdf, 1);
        let expected = common::pdf::flate_source(kind).crop(25, 0, 75, 60);
        assert_eq!(actual.rgb, expected.rgb, "{kind}");
        assert_eq!(info.bpc, if kind == "bw" { 1 } else { 8 });
        assert_eq!(info.filter, "/FlateDecode");
    }
}

#[tokio::test]
async fn crop_keeps_matching_icc_profile() {
    let e = env();
    common::pdf::make_pixel_pdf(&e.pdf, &["rgb"]);
    common::pdf::edit_pdf(&e.pdf, |pdf| {
        let icc = pdf.add_object(lopdf::Stream::new(lopdf::dictionary! {"N"=>3}, common::pdf::srgb_profile()));
        common::pdf::first_xobject_mut(pdf, 1).dict.set("ColorSpace", vec![lopdf::Object::Name(b"ICCBased".to_vec()), icc.into()]);
    });
    let crops = [json!({"page":1,"x0":0.5,"y0":0.0,"x1":1.0,"y1":1.0})];
    let (_, error) = e.bm.delete_pdf_pages(1, &[], Some(&crops), None, None, None).await;
    assert_eq!(error, None);
    let pdf = common::pdf::load(&e.pdf);
    let image = common::pdf::first_xobject(&pdf, 1);
    let space = image.dict.get(b"ColorSpace").unwrap().as_array().unwrap();
    assert_eq!(space[0].as_name().unwrap(), b"ICCBased");
    let profile = pdf.get_object(space[1].as_reference().unwrap()).unwrap().as_stream().unwrap();
    assert_eq!(profile.content, common::pdf::srgb_profile());
}

#[tokio::test]
async fn upload_preview_applies_exif_and_reuses_already_normalized_jpeg() {
    let e = env();
    for orientation in 1..=8 {
        let source = common::image::jpeg_with_orientation(orientation, 200, 100);
        let (jpeg, error) = e.bm.preview_insert_upload(&source).await;
        assert_eq!(error, None);
        let jpeg = jpeg.unwrap();
        assert_eq!(common::image::image_size(&jpeg), if orientation <= 4 { (200, 100) } else { (100, 200) });
        assert_eq!(e.bm.preview_insert_upload(&jpeg).await, (Some(jpeg), None));
    }
    for data in [b"".as_slice(), b"not an image"] {
        let (jpeg, error) = e.bm.preview_insert_upload(data).await;
        assert!(jpeg.is_none());
        assert!(error.is_some());
    }
}

#[tokio::test]
async fn raw_uncompressed_image_can_be_cropped() {
    let e = env();
    common::pdf::make_pixel_pdf(&e.pdf, &["rgb"]);
    common::pdf::edit_pdf(&e.pdf, |pdf| {
        let image = common::pdf::first_xobject_mut(pdf, 1);
        image.content = image.get_plain_content().unwrap();
        image.dict.remove(b"Filter");
        image.dict.remove(b"DecodeParms");
    });
    let crops = [json!({"page":1,"x0":0.5,"y0":0.0,"x1":1.0,"y1":1.0})];
    let (_, error) = e.bm.delete_pdf_pages(1, &[], Some(&crops), None, None, None).await;
    assert_eq!(error, None);
    assert_eq!(common::pdf::pixel_image(&e.pdf, 1).0.pixel(10, 10), [0, 0, 255]);
}

#[tokio::test]
async fn custom_trash_directory_is_used() {
    let e = env();
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_trash_dirname("backup"));
    let (result, error) = bm.delete_pdf_pages(1, &[1], None, None, None, None).await;
    assert_eq!(error, None);
    assert_eq!(result.unwrap()["backup"], "backup/book.pdf");
    assert!(e.root.path().join("backup/book.pdf").is_file());
}

#[tokio::test]
async fn conflicting_or_duplicate_edits_do_not_save() {
    let e = env();
    let original = fs::read(&e.pdf).unwrap();
    let crop = json!({"page":1,"x0":0.0,"y0":0.0,"x1":1.0,"y1":1.0});
    let crops = [crop.clone()];
    let splits = [json!({"page":1,"first":crop,"second":crop})];
    for (pages, crops, splits, expected) in [
        (vec![1], Some(crops.to_vec()), None, "같은 쪽"),
        (vec![1], None, Some(splits.to_vec()), "같은 쪽"),
        (vec![], Some(crops.to_vec()), Some(splits.to_vec()), "같은 쪽"),
        (vec![], Some(vec![crop.clone(), crop]), None, "둘 이상"),
    ] {
        let (result, error) = e.bm.delete_pdf_pages(1, &pages, crops.as_deref(), splits.as_deref(), None, None).await;
        assert_eq!(result, None);
        assert!(error.unwrap().contains(expected));
    }
    assert_eq!(fs::read(&e.pdf).unwrap(), original);
    assert!(!e.backup().exists());
}

/// JPEG 헤더의 DQT와 SOF를 읽는다. 엔트로피 데이터는 첫 SOS에서 멈춘다.
fn jpeg_header_segments(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    let mut pos = 2;
    let mut result = Vec::new();
    loop {
        assert_eq!(bytes[pos], 0xff);
        let marker = bytes[pos + 1];
        if marker == 0xda {
            break;
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if [0xdb, 0xc0, 0xc1, 0xc2].contains(&marker) {
            result.push((marker, bytes[pos + 4..pos + 2 + len].to_vec()));
        }
        pos += 2 + len;
    }
    result
}

#[tokio::test]
async fn jpeg_crop_preserves_quantization_sampling_progressive_and_grayscale() {
    use jpeg_encoder::{ColorType, Encoder, QuantizationTableType, SamplingFactor};
    for (gray, progressive, sampling) in
        [(false, false, SamplingFactor::F_1_1), (false, false, SamplingFactor::F_2_1), (false, true, SamplingFactor::F_2_2), (true, false, SamplingFactor::F_1_1), (true, true, SamplingFactor::F_1_1)]
    {
        let e = env();
        let source = image::RgbImage::from_fn(100, 60, |x, _| if x < 50 { image::Rgb([180, 180, 180]) } else { image::Rgb([40, 40, 40]) });
        let pixels = if gray { image::DynamicImage::ImageRgb8(source.clone()).to_luma8().into_raw() } else { source.into_raw() };
        let mut jpeg = Vec::new();
        let mut encoder = Encoder::new(&mut jpeg, 75);
        encoder.set_progressive(progressive);
        encoder.set_sampling_factor(sampling);
        encoder
            .set_quantization_tables(QuantizationTableType::Custom(Box::new(std::array::from_fn(|i| 7 + i as u16))), QuantizationTableType::Custom(Box::new(std::array::from_fn(|i| 90 - i as u16))));
        encoder.encode(&pixels, 100, 60, if gray { ColorType::Luma } else { ColorType::Rgb }).unwrap();
        let expected = jpeg_header_segments(&jpeg);
        common::pdf::make_pixel_pdf(&e.pdf, &["jpeg"]);
        common::pdf::edit_pdf(&e.pdf, |pdf| {
            let stream = common::pdf::first_xobject_mut(pdf, 1);
            stream.content = jpeg.clone();
            stream.dict.set("ColorSpace", if gray { "DeviceGray" } else { "DeviceRGB" });
        });
        let crops = [json!({"page":1,"x0":0.5,"y0":0.0,"x1":1.0,"y1":1.0})];
        let (_, error) = e.bm.delete_pdf_pages(1, &[], Some(&crops), None, None, None).await;
        assert_eq!(error, None);
        let doc = common::pdf::load(&e.pdf);
        let stream = common::pdf::first_xobject(&doc, 1);
        assert_eq!(stream.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        let actual = jpeg_header_segments(&stream.content);
        assert_eq!(actual.iter().filter(|(m, _)| *m == 0xdb).collect::<Vec<_>>(), expected.iter().filter(|(m, _)| *m == 0xdb).collect::<Vec<_>>());
        let actual_frame = actual.iter().find(|(m, _)| *m != 0xdb).unwrap();
        let expected_frame = expected.iter().find(|(m, _)| *m != 0xdb).unwrap();
        assert_eq!(actual_frame.0, expected_frame.0);
        assert_eq!(&actual_frame.1[5..], &expected_frame.1[5..]);
        let decoded = image::load_from_memory(&stream.content).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (50, 60));
        assert!(decoded.to_luma8().get_pixel(25, 30)[0] < 50);
        assert_eq!(stream.dict.get(b"ColorSpace").unwrap().as_name().unwrap(), if gray { b"DeviceGray".as_slice() } else { b"DeviceRGB".as_slice() });
    }
}

struct FetchImage(Result<Vec<u8>, backend::direct_cover_url::FetchError>);
impl backend::direct_cover_url::ImageFetcher for FetchImage {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, backend::direct_cover_url::FetchError> {
        assert_eq!(url, "https://example.com/image.png");
        self.0.clone()
    }
}

#[tokio::test]
async fn url_preview_is_the_same_jpeg_that_gets_inserted() {
    let e = env();
    let fetcher = Arc::new(FetchImage(Ok(common::image::png_solid(80, 160, [10, 20, 30]))));
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(fetcher));
    let (preview, error) = bm.preview_insert_image("  https://example.com/image.png  ").await;
    assert_eq!(error, None);
    let inserts = [json!({"page":1,"url":"https://example.com/image.png"})];
    assert_eq!(bm.delete_pdf_pages(1, &[], None, None, Some(&inserts), None).await.1, None);
    assert_eq!(common::pdf::first_xobject(&common::pdf::load(&e.pdf), 1).content, preview.unwrap());
}

#[tokio::test]
async fn url_preview_reports_safe_fetch_errors() {
    use backend::direct_cover_url::FetchError;
    for (effect, expected) in [
        (FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".into()), "내부 주소의 이미지는 사용할 수 없습니다"),
        (FetchError::Os("dns error".into()), "이미지 서버 주소를 확인하지 못했습니다"),
        (FetchError::Other("private details".into()), "이미지를 가져오지 못했습니다"),
    ] {
        let e = env();
        let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(Arc::new(FetchImage(Err(effect)))));
        assert_eq!(bm.preview_insert_image("https://example.com/image.png").await, (None, Some(expected.into())));
    }
    let e = env();
    for url in ["", "  ", "http://127.0.0.1/a", "ftp://example.com/a"] {
        let (image, error) = e.bm.preview_insert_image(url).await;
        assert!(image.is_none() && error.is_some());
    }
}

#[tokio::test]
async fn http_url_preview_returns_uncached_jpeg() {
    use backend::main::{AppDeps, AuthMode, app_with};
    let e = env();
    let bm =
        Arc::new(BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(Arc::new(FetchImage(Ok(common::image::png_solid(80, 160, [10, 20, 30])))))));
    let client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(bm.clone()).with_comics_manager(bm)));
    for prefix in ["", "/comics"] {
        let path = format!("{prefix}/pdf-pages/1/insert-preview");
        let response = client.post_json(&path, &json!({"url":"https://example.com/image.png"})).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
        assert_eq!(common::image::image_size(&response.body), (80, 160));
        assert_eq!(client.post_json(&path, &json!({"url":""})).await.status, 422);
        assert_eq!(client.post_json(&path, &json!({"url":"  "})).await.status, 400);
    }
}
