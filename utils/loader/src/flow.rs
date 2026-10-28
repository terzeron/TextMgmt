//! 적재 흐름: 배치로 파일을 읽어 ES에 넣는다. 원본: `main()` 안의 `process_file_iter`와 인자 처리.

use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use corpus_layout::is_ignored_segment;
use doc_reader::pdf::StageRunner;
use es_client::EsError;
use es_client::indexing::IndexStore;
use serde_json::Value;
use stat::Stat;

use crate::config::Config;
use crate::record::{ReadOutcome, detect_file_type, read_file};

pub type BatchReadHook<'a> = dyn Fn(&Path, Option<&Metadata>, bool) -> ReadOutcome + Sync + 'a;
pub type BatchStatHook<'a> = dyn Fn(&Path) -> std::io::Result<Metadata> + Sync + 'a;
#[derive(Default)]
pub struct ProcessHooks<'a> {
    pub read_file: Option<&'a BatchReadHook<'a>>,
    pub stat: Option<&'a BatchStatHook<'a>>,
    pub on_worker_spawn: Option<&'a (dyn Fn() + Sync + 'a)>,
}

/// 배치 하나에 모으는 파일 수.
pub const BATCH_SIZE: usize = 100;
/// 파일 인자를 모아서 처리하는 단위.
pub const FILE_ARGUMENT_BATCH_SIZE: usize = 100;
/// 읽기 병렬도. /mnt/data는 회전 디스크라 늘려도 I/O로 막힌다.
pub const WORKER_COUNT: usize = 2;

/// 문제가 있을 때만 `* 파일경로`와 그 아래에 사유를 쓴다. 문제 여부를 돌려준다.
pub fn report_problems(out: &mut dyn Write, file_path: &Path, messages: &[String], indexed: bool) -> bool {
    // 경로와 예외를 각각 로깅하므로 경로만 있는 줄은 제거한다.
    let shown = file_path.display().to_string();
    let mut problems: Vec<String> = messages.iter().filter(|m| **m != shown).cloned().collect();
    if !indexed {
        problems.push(if file_path.is_file() { format!("적재 제외: 지원하지 않는 파일 형식 '{}'", suffix_with_dot(file_path)) } else { "적재 제외: 파일이 사라짐".to_owned() });
    }
    if problems.is_empty() {
        return false;
    }
    let _ = writeln!(out, "* {shown}");
    for m in problems {
        let _ = writeln!(out, "    {m}");
    }
    true
}

fn suffix_with_dot(path: &Path) -> String {
    path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default()
}

/// 이미 색인된 문서의 `updated_time`(로컬 시각 ISO)을 유닉스 시각으로 바꾼다.
fn iso_to_timestamp(text: &str) -> Option<f64> {
    use chrono::{Local, NaiveDateTime, TimeZone};
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    let local = Local.from_local_datetime(&naive).earliest()?;
    Some(local.timestamp() as f64 + f64::from(local.timestamp_subsec_nanos()) / 1e9)
}

/// 적재 도구. ES와 PDF 단계 실행기를 바꿔 끼울 수 있다.
pub struct Loader<'a> {
    pub cfg: &'a Config,
    pub es: &'a dyn IndexStore,
    pub runner: &'a (dyn StageRunner + Sync),
    pub stat: &'a Stat,
}

/// 배치 처리 결과: (적재한 수, 건너뛴 수, 경로 동기화 수).
pub type BatchCounts = (usize, usize, usize);

impl Loader<'_> {
    /// PDF는 멈출 수 있어 메인 스레드에서 읽고, 나머지는 작업 스레드에서 읽는다.
    fn can_parse_in_worker(path: &Path) -> bool {
        let declared = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if declared == "pdf" {
            return false;
        }
        detect_file_type(path, &declared).as_deref() != Some("pdf")
    }

    /// 파일들을 배치로 처리해 ES에 저장한다.
    pub fn process_file_iter(
        &self,
        out: &mut dyn Write,
        files: impl IntoIterator<Item = PathBuf>,
        skip_check: bool,
        skip_text: bool,
        seen_inodes: Option<&mut HashSet<i64>>,
    ) -> Result<BatchCounts, EsError> {
        self.process_file_iter_with_hooks(out, files, skip_check, skip_text, seen_inodes, &ProcessHooks::default())
    }

    pub fn process_file_iter_with_hooks(
        &self,
        out: &mut dyn Write,
        files: impl IntoIterator<Item = PathBuf>,
        skip_check: bool,
        skip_text: bool,
        mut seen_inodes: Option<&mut HashSet<i64>>,
        hooks: &ProcessHooks<'_>,
    ) -> Result<BatchCounts, EsError> {
        let (mut skipped_count, mut processed_count, mut synced_count) = (0, 0, 0);
        let mut files = files.into_iter().peekable();
        while files.peek().is_some() {
            let batch_files: Vec<PathBuf> = files.by_ref().take(BATCH_SIZE).collect();

            // stat을 한 번만 하고 inode와 함께 둔다. 같은 inode(하드링크)는 나중 것이 이긴다.
            let mut order: Vec<i64> = Vec::new();
            let mut stat_map: HashMap<i64, (PathBuf, Metadata)> = HashMap::new();
            for path in batch_files {
                let st = match hooks.stat {
                    Some(get) => get(&path),
                    None => std::fs::metadata(&path),
                };
                let Ok(st) = st else { continue };
                let inode = st.ino() as i64;
                if !stat_map.contains_key(&inode) {
                    order.push(inode);
                }
                stat_map.insert(inode, (path, st));
            }
            // 디스크에 있는(live) inode: 고아 판정 기준이다. 파싱 성공 여부와 무관하다.
            if let Some(seen) = seen_inodes.as_deref_mut() {
                seen.extend(order.iter().copied());
            }

            let mut existing_len = 0;
            let mut path_changed: HashSet<i64> = HashSet::new();
            let new_inodes: Vec<i64> = if skip_check {
                order.clone()
            } else {
                let existing = self.es.get_existing_paths(&order)?;
                existing_len = existing.len();
                for inode in &order {
                    let Some(es_path) = existing.get(inode) else { continue };
                    let (path, _) = &stat_map[inode];
                    let current = path.strip_prefix(self.cfg.path_prefix(path)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    if current != *es_path {
                        path_changed.insert(*inode);
                        let _ = writeln!(out, "  [경로 변경 감지] inode={inode}: {es_path} → {current}");
                    }
                }
                skipped_count += existing_len - path_changed.len();
                synced_count += path_changed.len();
                order.iter().copied().filter(|i| !existing.contains_key(i) || path_changed.contains(i)).collect()
            };

            // 파일 읽기. 문제가 있는 파일만 경로와 사유를 출력한다.
            let (workers, main_thread): (Vec<i64>, Vec<i64>) = new_inodes.iter().partition(|i| Self::can_parse_in_worker(&stat_map[i].0));
            let results: Mutex<HashMap<i64, ReadOutcome>> = Mutex::new(HashMap::new());
            let next = AtomicUsize::new(0);
            // 작업 스레드는 ES를 쓰지 않으므로 필요한 것만 빌려 준다.
            let (cfg, runner, stat) = (self.cfg, self.runner, self.stat);
            let parse = |path: &Path, st: &Metadata| match hooks.read_file {
                Some(read) => read(path, Some(st), skip_text),
                None => read_file(cfg, path, Some(st), skip_text, None, runner, stat),
            };
            std::thread::scope(|scope| {
                if !workers.is_empty() {
                    for _ in 0..WORKER_COUNT.min(workers.len()) {
                        if let Some(on_spawn) = hooks.on_worker_spawn {
                            on_spawn();
                        }
                        scope.spawn(|| {
                            loop {
                                let i = next.fetch_add(1, Ordering::SeqCst);
                                let Some(inode) = workers.get(i) else { break };
                                let (path, st) = &stat_map[inode];
                                let outcome = parse(path, st);
                                results.lock().unwrap_or_else(|e| e.into_inner()).insert(*inode, outcome);
                            }
                        });
                    }
                }
                for inode in &main_thread {
                    let (path, st) = &stat_map[inode];
                    let outcome = parse(path, st);
                    results.lock().unwrap_or_else(|e| e.into_inner()).insert(*inode, outcome);
                }
            });
            let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());

            let mut batch_data: Vec<(i64, Value)> = Vec::new();
            for inode in &new_inodes {
                let outcome = results.remove(inode).unwrap_or_default();
                report_problems(out, &stat_map[inode].0, &outcome.messages, outcome.item.is_some());
                if let Some((id, doc)) = outcome.item {
                    // 같은 inode가 두 번 나오면 나중 것이 이긴다(dict.update).
                    if let Some(slot) = batch_data.iter_mut().find(|(i, _)| *i == id) {
                        slot.1 = doc;
                    } else {
                        batch_data.push((id, doc));
                    }
                }
            }

            // 저장: 중복 file_path 문서를 먼저 지우고 넣는다.
            if !batch_data.is_empty() {
                let ids: Vec<i64> = batch_data.iter().map(|(i, _)| *i).collect();
                let metadata = self.es.get_existing_created_time_metadata(&ids)?;
                for (inode, doc) in &mut batch_data {
                    let Some(meta) = metadata.get(inode) else { continue };
                    if let Some(v) = meta.get("created_time") {
                        doc["created_time"] = Value::String(v.clone());
                    }
                    if let Some(v) = meta.get("created_time_source") {
                        doc["created_time_source"] = Value::String(v.clone());
                    }
                    // 파일이 마지막 적재 이후 바뀌지 않았으면 updated_time을 유지한다(최신 책 탭에 재적재분이 끼지 않도록).
                    if let (Some(stored), Some((_, st))) = (meta.get("updated_time"), stat_map.get(inode))
                        && let Some(ts) = iso_to_timestamp(stored)
                        && st.mtime() as f64 + st.mtime_nsec() as f64 / 1e9 <= ts
                    {
                        doc["updated_time"] = Value::String(stored.clone());
                    }
                }
                let new_paths: Vec<String> = batch_data.iter().filter_map(|(_, d)| d["file_path"].as_str().map(str::to_owned)).collect();
                let cleaned = self.es.delete_by_file_paths(&new_paths, &ids);
                if cleaned > 0 {
                    let _ = writeln!(out, "  [중복 제거: {cleaned}개 기존 문서 삭제]");
                }
                self.es.insert(&batch_data)?;
                processed_count += batch_data.len();
                let _ = self.stat.add("index_count", batch_data.len() as f64);
                if skip_check {
                    let _ = writeln!(out, "  [배치 저장: {}개]", batch_data.len());
                } else {
                    let synced_msg = if path_changed.is_empty() { String::new() } else { format!(", 경로변경 재적재: {}개", path_changed.len()) };
                    let _ = writeln!(out, "  [배치 저장: {}개, 건너뜀: {}개{synced_msg}]", batch_data.len(), existing_len - path_changed.len());
                }
            }
        }
        Ok((processed_count, skipped_count, synced_count))
    }
}

/// 디렉토리 밑의 파일들. 숨김 디렉토리와 파이프라인 중간 산출물 디렉토리는 뺀다(`rglob`처럼 심볼릭 링크 디렉토리는 따라가지 않는다).
pub fn recursive_files(root: &Path) -> Vec<PathBuf> {
    recursive_files_with_unreadable(root).0
}

/// `recursive_files`와 같고, 끝까지 읽지 못한 디렉토리 목록을 함께 돌려준다.
/// 목록이 비어 있지 않으면 파일 목록이 불완전하다. orphan 판정에 쓰면 안 된다.
pub fn recursive_files_with_unreadable(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, unreadable: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            unreadable.push(dir.to_path_buf());
            return;
        };
        let Ok(mut entries) = entries.collect::<std::io::Result<Vec<_>>>() else {
            unreadable.push(dir.to_path_buf());
            return;
        };
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_ignored_segment(&name) {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&path, out, unreadable),
                Ok(_) if path.is_file() => out.push(path),
                Ok(_) => {}
                Err(_) => unreadable.push(path),
            }
        }
    }
    let mut out = Vec::new();
    let mut unreadable = Vec::new();
    walk(root, &mut out, &mut unreadable);
    (out, unreadable)
}

/// 직접 속한 일반 파일들(숨김 제외).
pub fn direct_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    files.sort();
    files
}

/// 하위 디렉토리마다 첫 파일 하나(숨김·파이프라인 디렉토리 제외). 파일이 없는 디렉토리는 `None`.
pub fn sample_files(dir: &Path) -> Vec<(String, Option<PathBuf>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|p| !p.file_name().is_some_and(|n| is_ignored_segment(&n.to_string_lossy())))
        .collect();
    subdirs.sort();
    subdirs
        .into_iter()
        .map(|d| {
            let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (name, direct_files(&d).into_iter().next())
        })
        .collect()
}

/// Python `Loader.get_file_list` 대응. 비재귀 모드는 각 하위 디렉터리의 첫 파일과 직속 파일을 반환한다.
pub fn get_file_list(path: &Path, recursive: bool, num_files: Option<usize>) -> Vec<PathBuf> {
    let mut files = if path.is_dir() {
        if recursive {
            recursive_files(path)
        } else {
            let mut files: Vec<PathBuf> = sample_files(path).into_iter().filter_map(|(_, file)| file).collect();
            files.extend(direct_files(path));
            files
        }
    } else {
        vec![path.to_path_buf()]
    };
    if let Some(limit) = num_files {
        files.truncate(limit);
    }
    files
}

/// 파일들을 읽어 inode 키의 문서 맵을 만든다. 테스트와 하위 호환성용이다.
pub fn read_files(cfg: &Config, path: &Path, runner: &(dyn StageRunner + Sync), stat: &Stat) -> HashMap<i64, Value> {
    let mut data = HashMap::new();
    for file in get_file_list(path, false, None) {
        let outcome = crate::record::read_file(cfg, &file, None, false, None, runner, stat);
        if let Some((inode, doc)) = outcome.item {
            data.insert(inode, doc);
        }
    }
    data
}
