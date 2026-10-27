//! 색인·파싱 통계 카운터. 원본: `utils/stat.py`.
//!
//! Python은 클래스 속성을 전역 카운터로 쓴다. 여기서는 스레드 안전한 `Stat` 값으로 두고,
//! 프로세스 전체가 공유하는 `Stat::global()`도 제공한다. 항목 이름은 Python 속성 이름 그대로다.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, OnceLock};

/// 파일 종류별 `<이름>_count`와 `<이름>_total_time`, 그리고 UTF-8 재인코딩 건수.
const FIELDS: [&str; 23] = [
    "text_count",
    "text_total_time",
    "text_reencoded_count",
    "normal_epub_count",
    "normal_epub_total_time",
    "zipped_epub_count",
    "zipped_epub_total_time",
    "pdf_count",
    "pdf_total_time",
    "html_count",
    "html_total_time",
    "docx_count",
    "docx_total_time",
    "doc_count",
    "doc_total_time",
    "hwp_count",
    "hwp_total_time",
    "rtf_count",
    "rtf_total_time",
    "image_count",
    "image_total_time",
    "index_count",
    "index_total_time",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatError {
    UnknownField(String),
}

impl fmt::Display for StatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatError::UnknownField(name) => write!(f, "알 수 없는 통계 항목: {name}"),
        }
    }
}

impl std::error::Error for StatError {}

#[derive(Default)]
struct Inner {
    values: HashMap<String, f64>,
    stage_count: BTreeMap<String, u64>,
    stage_total_time: BTreeMap<String, f64>,
    stage_outcome_count: BTreeMap<(String, String), u64>,
}

pub struct Stat {
    inner: Mutex<Inner>,
}

impl Default for Stat {
    fn default() -> Self {
        Self::new()
    }
}

fn field(name: &str) -> Result<&'static str, StatError> {
    FIELDS.iter().find(|f| **f == name).copied().ok_or_else(|| StatError::UnknownField(name.to_owned()))
}

impl Stat {
    pub fn new() -> Self {
        Stat { inner: Mutex::new(Inner::default()) }
    }

    /// 프로세스 전체가 공유하는 인스턴스. Python의 클래스 속성에 해당한다.
    pub fn global() -> &'static Arc<Stat> {
        static GLOBAL: OnceLock<Arc<Stat>> = OnceLock::new();
        GLOBAL.get_or_init(|| Arc::new(Stat::new()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // 다른 스레드가 panic해도 통계는 계속 쓴다.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn divide(a: f64, b: f64) -> f64 {
        if b != 0.0 { a / b } else { 0.0 }
    }

    pub fn add(&self, name: &str, value: f64) -> Result<(), StatError> {
        let mut inner = self.lock();
        if let Ok(key) = field(name) {
            *inner.values.entry(key.to_owned()).or_insert(0.0) += value;
            return Ok(());
        }
        let Some(current) = inner.values.get_mut(name) else { return Err(StatError::UnknownField(name.to_owned())) };
        *current += value;
        Ok(())
    }

    pub fn set_value(&self, name: &str, value: f64) -> Result<(), StatError> {
        self.lock().values.insert(name.to_owned(), value);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<f64, StatError> {
        let inner = self.lock();
        if let Ok(key) = field(name) {
            return Ok(inner.values.get(key).copied().unwrap_or(0.0));
        }
        inner.values.get(name).copied().ok_or_else(|| StatError::UnknownField(name.to_owned()))
    }

    pub fn record_pdf_stage(&self, stage: &str, elapsed: f64, outcome: &str) {
        let mut g = self.lock();
        *g.stage_count.entry(stage.to_owned()).or_insert(0) += 1;
        *g.stage_total_time.entry(stage.to_owned()).or_insert(0.0) += elapsed;
        *g.stage_outcome_count.entry((stage.to_owned(), outcome.to_owned())).or_insert(0) += 1;
    }

    pub fn pdf_stage_count(&self, stage: &str) -> u64 {
        self.lock().stage_count.get(stage).copied().unwrap_or(0)
    }

    pub fn pdf_stage_total_time(&self, stage: &str) -> f64 {
        self.lock().stage_total_time.get(stage).copied().unwrap_or(0.0)
    }

    pub fn pdf_stage_outcome_count(&self, stage: &str, outcome: &str) -> u64 {
        self.lock().stage_outcome_count.get(&(stage.to_owned(), outcome.to_owned())).copied().unwrap_or(0)
    }

    /// 요약을 쓴다. 형식은 Python `print()`와 같다: 건수는 7자리 0 채움, 평균 시간은 소수 4자리.
    pub fn print(&self, out: &mut dyn Write) -> io::Result<()> {
        let g = self.lock();
        let v = |name: &str| g.values.get(name).copied().unwrap_or(0.0);
        let row = |out: &mut dyn Write, label: &str, kind: &str| -> io::Result<()> {
            let count = v(&format!("{kind}_count"));
            writeln!(out, "{label} {:07} / {:.4}", count as i64, Self::divide(v(&format!("{kind}_total_time")), count))
        };

        writeln!(out, "[Stat]")?;
        row(out, "text:   ", "text")?;
        if v("text_reencoded_count") != 0.0 {
            writeln!(out, "  └ UTF-8 재인코딩: {}건", v("text_reencoded_count") as i64)?;
        }
        row(out, "epub:   ", "normal_epub")?;
        row(out, "epub(z):", "zipped_epub")?;
        row(out, "pdf:    ", "pdf")?;
        for (stage, count) in &g.stage_count {
            let outcome = |name: &str| g.stage_outcome_count.get(&(stage.clone(), name.to_owned())).copied().unwrap_or(0);
            let avg = Self::divide(g.stage_total_time.get(stage).copied().unwrap_or(0.0), *count as f64);
            writeln!(
                out,
                "  - {stage:<24} {count:07} / {avg:.4} / ok={} empty={} damaged={} timeout={} error={}",
                outcome("ok"),
                outcome("empty"),
                outcome("damaged"),
                outcome("timeout"),
                outcome("error")
            )?;
        }
        for (label, kind) in [("html:   ", "html"), ("docx:   ", "docx"), ("doc:    ", "doc"), ("hwp:    ", "hwp"), ("rtf:    ", "rtf"), ("image:  ", "image"), ("total:  ", "index")] {
            row(out, label, kind)?;
        }
        Ok(())
    }
}
