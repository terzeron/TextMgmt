# Test and Coverage 100% Plan

## Goal

- Keep all Python, Rust, and frontend tests passing, including Rust tests currently marked ignored.
- Raise source line coverage to 100% for backend/utils Python, the Rust workspace, and frontend `src`.
- Do not hide uncovered code with coverage exclusions. Remove dead code only when callers and behavior confirm it is unreachable.
- Treat classifier model quality and corpus parity as a separate review from code coverage.

## Baseline (2026-10-27)

| Area | Tests | Line coverage | Result |
| --- | ---: | ---: | --- |
| Python backend/utils | 3,051 | 94.34% (12,829/13,598) | Passed |
| Rust workspace | 3,213 active + 103 ignored | 85.92% (36,354/42,313) | Active and ignored suites passed |
| Frontend | 2,470 | 99.88% (6,068/6,075) | Passed |

The coverage figures are separate per language. Python coverage includes legacy modules
that Rust replaces; those files remain in the current configured target set.

## Work Slices

1. **Baseline and scope:** run every suite, including ignored Rust integration tests; retain machine-readable coverage reports.
2. **Frontend:** cover remaining viewer/admin/helper paths or remove unreachable defensive branches after caller review.
3. **Python:** cover the remaining classifier exporter, EPUB cover, book classifier, and legacy utility lines.
4. **Rust:** close uncovered paths by crate, starting with `pyferrite`, backend routes, `loader`, and `classify_cli`.
5. **Gates:** configure line-coverage thresholds only after all three areas reach 100%; rerun every suite and record final reports.

## Current Progress

- **Latest verified snapshot (2026-10-27):**

  | Area | Latest line coverage | Latest test run |
  | --- | ---: | --- |
  | Python backend/utils | 100% (13,597/13,597) | 3,017 full-suite + 1 focused passed; 105 skipped |
  | Rust workspace | 91.88% (40,661/44,254) | 3,464 active + 104 ignored passed; latest backend classifier batch passed |
  | Frontend `src` | 100% (6,072/6,072) | 2,473 passed |

  Python report: `/var/tmp/tm-python-coverage-current-20261027.json`.
  Rust report: `/var/tmp/tm-rust-coverage-workspace-merged-after-export-format-20261027.json`
  (accumulated profile from the complete active/ignored workspace run plus the
  later backend package batch; integration test files are excluded from the
  source-file report).
  Frontend: `npm test` summary (Vite script already enables coverage).
- Python line coverage is complete. New utility tests cover the Python CLI
  behavior; no coverage exclusions were added. A provably unreachable empty-key
  guard in `infer_identity` was removed because `parse_filename` guarantees a
  non-empty alphanumeric key.
- The merged Rust snapshot before the focused CLI tests listed
  `utils/detect_mojibake/src/cli.rs` (87),
  `backend/src/epub_cover/edit.rs` (55),
  `backend/src/book_manager/preview.rs` (55), `backend/src/bookstore.rs` (52),
  and `libs/pyferrite/src/formats/hdf5/msg.rs` (49). The latest merged report
  counts 3,648 uncovered lines across 43,910 executable lines.
  Added five grouped CLI unit tests for argument parsing, help and status codes,
  JSON output, report directory creation, and filesystem/writer errors. The
  focused `detect_mojibake` suite passes 5 unit, 3 binary, and 32 integration
  tests. `cli.rs` now covers 104/105 lines (99.05%); its remaining uncovered
  line is the propagated directory-creation error expression. Focused report:
  `/var/tmp/tm-detect-mojibake-cli-cov-20261027.json`.
- Added seven `epub_cover::edit` unit tests for page encoding and lookup errors,
  masked image tags, crop rectangle and image size validation, alpha flattening,
  SVG sizing, and invalid edit requests. Added three integration cases for tool
  cover refusal, non-spine pages, and repeated image references. The backend
  lib suite passes 91 tests and `epub_image_edit` passes 54 tests. Focused
  `edit.rs` line coverage is 588/620 (94.84%), up from 565/620. Remaining
  misses include cache/output size caps, SVG split sizing, and manifest cleanup.
  This package-only report is not a refreshed workspace total:
  `/var/tmp/tm-epub-cover-edit-cov-20261027.json`.
- Added unit tests for `PreviewFailure` conversions, `file_response` read
  success/failure, and NCX filtering with included, missing, empty, and absent
  targets. Added preview error tests for ES lookup and cache-directory creation.
  Backend lib passed 94 tests and `book_preview` passed 114 tests. The merged
  Rust workspace report covers 40,291/43,910 lines (91.76%).
  `book_manager/preview.rs` covers 514/561 lines (91.62%); remaining misses
  include duplicate manifest entries, missing NCX entries, default image
  downscaling, and PDF page extraction/storage errors. Report:
  `/var/tmp/tm-rust-preview-cache-merged-20261027.json`.
  `utils/epubscan/src/security.rs` covers 655/664 lines
  (98.64%). The classifier target passes 52 tests; its source coverage is
  879/926 lines (94.92%).
  The ignored classifier parity tests used a fresh Python 3.13 environment
  synced from `uv.lock`; its scikit-learn 1.9.1 generated 266 Python golden
  cases. A focused `pyferrite` values run also passed 13 tests.
- Frontend line coverage is complete. Branch coverage remains 99.74% and is
  recorded separately because the selected threshold is line coverage.
- Added exporter rejection-path tests, EPUB metadata/image helper tests,
  malformed insert/encoding fallback regressions, and normalization race
  coverage. The latest full Python suite passed at 100% line coverage.
- Added loader binary dispatch tests for PDF and ISBN worker commands, help,
  missing ES configuration, and a retried connection against a local mock ES.
  `loader`'s `main.rs` reached 92.5% line coverage in its focused report.
- Added `ChildRunner` tests for successful worker JSON, nonzero exit, and
  malformed JSON. Added Toptoon catalog tests for incomplete metadata,
  author/category fallbacks, and catalog transport/JSON failures.
- Activated the abstract bookstore method tests and covered all factory names,
  default trait behavior, unavailable HTTP, Ridi headers/redirect handling,
  URL joining, and title parsing. The unreachable multiple-child check in
  `title_string` was removed after scraper tests showed `<title>` is one raw
  text node, including markup and comments.
- Added exhaustive `EsManagerApi` default-method tests and an adapter test for
  `ESManager`. The adapter tests check query methods, error normalization,
  category stats, path iteration, cover aggregations, and paging. Its combined
  line coverage rose to 95.18%.
- Added `joblib_import` unit coverage for pickle metadata conversion, NumPy
  arrays, vectorizer validation, and malformed model sections. It exposed and
  fixed empty-dimension shape conversion (`(2, 0)` now becomes `[[], []]`).
  Removed a proven unreachable vocabulary-gap check: range validation rejects
  out-of-range indices, and duplicate rejection plus `n` unique in-range
  indices guarantees every one of the `n` slots is populated.
- An earlier combined report showed `loader::cli` at 87.43% and `bookstore`
  with 118 uncovered lines; these counts are historical.
- An earlier combined Rust snapshot was 90.90%; the ignored pass used
  Docker-backed Elasticsearch and MySQL tests plus classifier golden fixtures.
- Added a regression for EPUB structure report-directory creation failure;
  the targeted CLI suite passed 14 tests. Its remaining reported line is a
  non-executable closing brace, which rustfmt keeps on its own line.
- Rust-only feature candidates identified separately: `pdfscan` and `epubscan` security scanning.
- Classifier `pyferrite` is migration support for reading existing joblib models; it is not treated as an unneeded feature.
- Added a fallback-path test for `epubImageRect`, an omitted-angle test for `cropRect`, and an EPUB crop-drag integration test.
- Added `cover_sources` unit tests for bookstore base URLs, supported host lists,
  search URL generation, Ridi and Naver Series parsing, ISBN capability, and
  public error display messages.
- Added `bookstore` tests for unoverridden trait defaults, HTML cache round trips,
  and Yes24, Aladin, and Ridibooks ISBN extraction fallbacks. Its focused line
  coverage reached 94.29% (1,239/1,314).
- Added Rust array encoding tests for numeric types, complex and fixed-width
  values, x87, endianness, streaming, and non-contiguous arrays. The encoder
  reached 94.17% (97/103); six lines remain, including defensive dtype fallbacks
  that normal `Array` construction cannot reach.
- Added `Value` accessor/type-name tests for every variant and nested array walks
  through frames and all `PyObject` fields. The `value/mod.rs` line coverage is
  85.59% (95/111) in the latest combined Rust report; 16 lines remain uncovered.
- Added public-reader pickle integration tests for Torch storage dtype mapping,
  storage offsets, strided tensors, out-of-range storage, and allocation caps.
  `torch_rebuild.rs` reached 92.80% (116/125); nine source lines remain.
- Re-ran all Python tests (3,117 passed; 13,597/13,597 lines) and frontend tests
  (78 files, 2,473 tests; 6,072/6,072 lines). The Rust clean-target active suite
  and all 103 ignored tests also passed; that earlier report was 91.24%.
- Added focused `admin.rs` unit tests for proposal failure state read/write
  errors and success/failure result shapes. The backend library suite passed;
  `admin.rs` reached 91.12% (780/856) in the latest clean combined report.
- Added malformed OPF recovery cases for unterminated comments, CDATA, mismatched
  and unmatched closing tags, plus TXT CRLF/lone-CR and non-Hangul decoder
  fallback cases. Added default `BookstoreDecider` delegation and service getter
  tests. The classifier target passes 52 tests.
- Added EPUB image editing tests for UTF-8 signature, UTF-16 BOM, Latin-1
  round-trip, unsupported encoding, and `data-src` preservation. The target
  passes 51 tests; `edit.rs` is 565/620 lines (91.13%). Rust coverage is 91.36%.
- Added bookstore truthiness cases for Python scalar/container values, the
  default HTTP session path, direct ISBN rejection for stores without ISBN
  support, and a truncated HTTP response body. The bookstore target passes 124
  tests; its source is 1,276/1,340 lines (95.22%). Rust coverage is 91.388%.
- Added `admin.rs` unit tests for both buffered proposal flush paths. Both pass;
  the file now covers 794/870 executable lines (91.26%). Ran all eight ignored
  MySQL `db.rs` integration tests and activated both operational classifier
  golden tests by supplying existing export/import directories. The complete
  active and ignored Rust workspace runs passed. Combined Rust coverage is
  39,917/43,670 lines (91.41%).
- Extended `db.rs` coverage for unknown SQL percent tokens, both public
  database error display forms, every remaining MySQL scalar column type, and
  transaction calls on a closed connection. A unit test now covers Diesel's
  non-database error fallback. The backend db target passes 16 active tests and
  all 10 ignored MySQL tests; `db.rs` covers 212/214 lines (99.07%).
- Added a bulk-rename regression proving an Elasticsearch refresh failure does
  not undo successful renames, plus a catastrophic-backtracking error test.
  All 19 `main_bulk_rename` tests pass.
- Added classifier tests for case-sensitive tokenization and a poetry label
  containing a fiction word. Both targeted tests pass. The aggregate line total
  remains unchanged. `cargo llvm-cov report --show-missing-lines` reports one
  missed line in each source file but omits their locations from its detailed
  missing-line list; identify the source-map entries before classifying them as
  unreachable.
- Added a unit test for the admin `mapping` adapter's success, HTTP error, and
  internal error paths. The focused test, complete active workspace, and all
  ignored Rust tests passed. The refreshed combined Rust report is
  39,943/43,693 lines (91.42%); 3,750 lines remain uncovered. Report:
  `/var/tmp/tm-rust-coverage-current-mapping-final.json`.
- Added four `run_classify_apply_job` unit scenarios: successful completion,
  callback failure, manager failure, and replaced/unreadable apply status. All
  four passed. Fresh clean active and ignored workspace runs passed. Combined
  coverage is 40,040/43,787 lines (91.44%); 3,747 lines remain uncovered.
  `admin.rs` now covers 890/960 lines (92.71%), with 70 misses. Report:
  `/var/tmp/tm-rust-coverage-current-clean-apply-final.json`.
- Added KakaoWebtoon and Lezhin non-verbose API failure tests, Kakao successful
  non-verbose response coverage, and NaverShopping soup title fallbacks. The
  full bookstore integration target passes 126 tests. Added malformed URL-safe
  Base64 cursor padding cases to the Elasticsearch manager tests; that focused
  test passes. Added Elasticsearch mapping tests for incomplete mapping shapes
  and failed mapping updates; all 95 `es_manager` integration tests pass.
  Added insert tests for injected-clock retries, nonretryable HTTP errors, and
  bulk item errors, plus truthiness, empty metadata, missing-index deletion,
  and nonretryable connection cases. All 100 `es_manager` tests pass. Current
  merged Rust line coverage is 40,071/43,787 (91.51%), with 3,716 lines
  uncovered. `bookstore.rs` covers 1,277/1,333 lines and `es_manager.rs`
  covers 1,286/1,327. All 104 Elasticsearch manager tests pass. Report:
  `/var/tmp/tm-rust-coverage-current-clean-es-iter-final.json`.
- Added `cover_management` tests for service paths and cache-key normalization,
  the EPUB thumbnailer file-type branches, and failed atomic cache writes that
  must clean temporary files. All 72 integration tests pass. Combined Rust
  coverage is 40,084/43,787 (91.54%); `cover_management.rs` covers 841/885
  lines. Report:
  `/var/tmp/tm-rust-coverage-current-clean-cover-management-final.json`.
- Added a regression proving blank image-insert URLs fail before changing an
  EPUB. Added direct public-state tests for PDF reader and page-count caches;
  the focused PDF cache test passes and `pdf.rs` reaches 203/237 lines
  (85.65%). Current combined Rust coverage is 40,090/43,787 (91.56%), with
  3,697 lines uncovered. Report:
  `/var/tmp/tm-rust-coverage-current-clean-pdf-caches-final.json`.
- Added real `LopdfOpener` tests for indirect and UTF-16/UTF-8/legacy metadata,
  omitted non-string values, missing files, and invalid PDF data. All 108
  `book_preview` tests pass; `pdf.rs` now covers 219/237 lines (92.41%).
  Combined Rust coverage is 40,106/43,787 (91.59%), with 3,681 lines
  uncovered. Report:
  `/var/tmp/tm-rust-coverage-current-clean-pdf-metadata-final.json`.
- Exercised `OffsetCorrected` xref recovery and the path-based extraction
  fallback for readers without an attached document. The `pdf.rs` missing-line
  report now leaves only the wildcard for future non-exhaustive
  `lopdf::XrefRecovery` variants; the current crate exposes no constructible
  value for that arm. Latest merged coverage remains 40,106/43,787 (91.59%).
  Report: `/var/tmp/tm-rust-coverage-current-clean-pdf-all-final.json`.
- Re-ran the full Rust workspace active and ignored suites from a fresh target;
  both passed, including Docker-backed Elasticsearch/MySQL tests and classifier
  golden tests. The clean combined report is 40,237/43,909 (91.64%), with 3,672
  missed lines. Report:
  `/var/tmp/tm-rust-coverage-clean-final-20261027.json`.
- Added route coverage for clearing classify proposals and the missing mapping
  dependency response. Corrected the route mock so clearing also removes items.
  All 30 `main_gaps` tests pass; admin route coverage increased to 896/960
  (93.33%). The merged workspace report is 40,242/43,909 (91.65%), with 3,667
  missed lines. Report: `/var/tmp/tm-rust-coverage-clean-final-after-admin.json`.
- Added regression coverage for ActiveX boolean feature detection and EPUB
  directory entries, encrypted/unsupported ZIP entries, and URL/magic branches.
  The `epubscan` security integration target passes all 58 tests. A fresh
  isolated target reports `security.rs` at 652/664 lines (98.19%); this is
  target-specific coverage, not a refreshed workspace total.
  The test also exposed and fixed misclassification of unary-negation feature
  detection. Report command:
  `CARGO_TARGET_DIR=/var/tmp/tm-epubscan-cov-batch2-20261027 cargo llvm-cov -p epubscan --test security --locked -- --test-threads=1`.
- Added grouped `classifier_model` tests for error formatting, manifest guards,
  coefficient EOF handling, export dimensions, and private metadata filtering.
  The backend library suite passed 107 tests; `classifier_model` passed 35
  (2 operational model tests ignored). `export_format.rs` reports 499/558
  covered line regions (89.43%).
- Extended `features.rs` tests across sparse spill errors, source compaction,
  selected-document fitting, memory/spill assembly, and transform. The backend
  library report covers 607/629 lines (96.50%).
- Latest accumulated Rust workspace report: 40,661/44,254 lines (91.88%), with
  3,593 missed. Top backend residuals are `export_format.rs` (59),
  `bookstore.rs` (52), `cover_sources.rs` (49), `book_classifier.rs` and
  `book_manager/classify.rs` (47 each). The full workspace tests were not rerun
  after the latest package batches. Report:
  `/var/tmp/tm-rust-coverage-workspace-merged-after-export-format-20261027.json`.
- User reports that `target/debug/deps` caused the disk pressure and has since
  run `cargo clean`. Preserve this history; the repository target was cleaned.

## Resume after `cargo clean` (2026-10-27)

- User confirmed `cargo clean` removed the disk-heavy `target/debug/deps`.
  The repository `target/` is now 632 KB and `target/debug/deps` is absent;
  `/` has 541 GB free. The retained coverage target under `/var/tmp` is 57 GB.
- Reused the retained target to run all `detect_mojibake` package targets.
  All 40 tests passed (5 lib, 3 bin integration, 32 detect integration).
  Package line coverage is 244/246 (99.19%): `cli.rs` 104/105,
  `lib.rs` 128/128, and `main.rs` 12/13. JSON:
  `/var/tmp/tm-detect-mojibake-package-resume-20261027.json`.
- This package result supersedes the older workspace snapshot's apparent
  `cli.rs` 18/105 figure for this crate. The overall workspace still needs a
  fresh aggregate run after the remaining coverage batches.
- Added five grouped unit tests for HDF5 message parsing in `pyferrite`, covering
  dataspace versions, datatype classes, old/current layouts, filter pipelines,
  and malformed input. All 61 `pyferrite` lib tests pass. `msg.rs` line coverage
  is now 123/123 (100%). The package lib aggregate is 2,538/6,741 (37.65%), so
  substantial module coverage work remains. Report:
  `/var/tmp/tm-pyferrite-hdf5-msg-final-20261027.json`.
- Added grouped unit tests for HDF5 superblock parsing and writing, numpy dtype
  descriptors, array byte encoding, and dtype sizing/classifiers. The latest
  `pyferrite --lib` batch has 79 passing tests. HDF5 `encode.rs`, `msg.rs`,
  `superblock.rs`, `dtype/descr.rs`, and `dtype/mod.rs` each have 100% line
  coverage. `value/encode.rs` is 100/103 (97.09%);
  the remaining uncovered source lines are defensive dtype mismatch arms that
  `Array::dtype()` cannot produce. `pyferrite --lib` is 2,879/6,741 (42.71%).
  Latest report: `/var/tmp/tm-pyferrite-dtype-final-20261027.json`.
- Added grouped format-sniffing tests for NPY, HDF5, NPZ/PT zip members, joblib,
  pickle, extension fallback, and path errors. All 83 `pyferrite --lib` tests
  pass; `formats/detect.rs` is 44/44 (100%) and the package is 2,937/6,741
  (43.57%). No dependency was added. Latest report:
  `/var/tmp/tm-pyferrite-detect-final-20261027.json`.
- Added `error.rs` tests for each display variant, error sources, and standard
  conversions. All 85 `pyferrite --lib` tests pass; `error.rs` is 39/39 (100%)
  and the package is 2,953/6,741 (43.81%). Report:
  `/var/tmp/tm-pyferrite-error-resume-20261027.json`.
- Rebuilt and ran all active Rust workspace tests after `cargo clean`; the
  complete run passed. Workspace line coverage is 40,673/43,981 (92.48%). This
  aggregate predates the HDF5 group unit-test batch below. Report:
  `/var/tmp/tm-rust-workspace-resume-active-20261027.json`.
- Added seven grouped HDF5 group-parser unit tests for local heaps, B-tree
  traversal, symbol tables, and hard/soft links. All 92 `pyferrite --lib`
  tests pass. `formats/hdf5/group.rs` is 114/115 lines (99.13%); its remaining
  missed line handles an out-of-file heap data extent. Package line coverage is
  5,707/6,741 (84.66%). Report:
  `/var/tmp/tm-pyferrite-group-resume-20261027.json`.
- Closed the remaining HDF5 group B-tree recursion line, then added HDF5
  planning tests for arrays, shapes, dictionary key validation, sorting,
  sequences, and unsupported values. Added read-path tests for nested
  round-trips, dataset indexing, allocation limits, unknown headers, undefined
  links, and depth limits. All 102 `pyferrite --lib` tests pass;
  `formats/hdf5/{group,plan,read}.rs` each report 100% line coverage.
  `pyferrite` line coverage is 5,744/6,741 (85.21%). Report:
  `/var/tmp/tm-pyferrite-read-final-20261027.json`.
- Re-ran active tests and coverage for the complete Rust workspace. All 3,633
  active tests passed; 2 model-dependent tests remain ignored. Workspace line
  coverage is 40,741/43,981 (92.63%), function coverage 87.29%, and region
  coverage 90.84%. Report:
  `/var/tmp/tm-rust-workspace-active-after-pyferrite-20261027.json`.
- Re-ran `cargo test --workspace --locked` with default parallel execution,
  including doc tests. All 3,643 tests passed, 0 failed, and 2 remained ignored.
  Log: `/var/tmp/tm-rust-workspace-parallel-20261027.log`.
- Consolidated the MySQL/Elasticsearch integration sources into one
  `container_integration` Cargo test target. Their `OnceLock` fixtures now live
  in one process, so each service is started once for the 420 tests in that
  target, including the DB/ES integration cases. The full workspace run passed
  3,643 tests, with 0 failures and 2
  ignored model-dependent tests. Log:
  `/var/tmp/tm-rust-workspace-session-fixture-rerun-20261027.log`.
- Isolated `book_manager_1` delete/move integration tests in temporary book
  directories. They no longer remove checked-in EPUBs while `doc_reader` scans
  the corpus. `cargo fmt --all -- --check` and `git diff --check` passed.
