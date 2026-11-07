//! 모델 파일이 바뀌면 다시 읽는다. 이미지 재빌드나 pod 재시작 없이 모델을 교체하게 한다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::softmax_model::SoftmaxModel;

#[derive(Default)]
struct State {
    /// 마지막으로 본 파일의 (수정 시각, 크기). 파일이 없었으면 `None`.
    stamp: Option<(SystemTime, u64)>,
    model: Option<Arc<SoftmaxModel>>,
    /// 파일 없음 경고를 이미 남겼는가. 상태가 바뀔 때만 다시 남긴다.
    missing_warned: bool,
}

pub struct ModelReloader {
    path: PathBuf,
    state: Mutex<State>,
}

impl ModelReloader {
    pub fn new(path: PathBuf) -> Self {
        Self { path, state: Mutex::new(State::default()) }
    }

    /// 지금 쓸 모델. 파일이 바뀌었으면 다시 읽고, 다시 읽기에 실패하면 이전 모델을 돌려준다.
    pub fn current(&self) -> Option<Arc<SoftmaxModel>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let stamp = std::fs::metadata(&self.path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        let Some(stamp) = stamp else {
            if !state.missing_warned {
                log::warn!("분류 모델 파일이 없다: {}", self.path.display());
                state.missing_warned = true;
            }
            state.stamp = None;
            state.model = None;
            return None;
        };
        state.missing_warned = false;
        if state.stamp != Some(stamp) {
            state.stamp = Some(stamp);
            match SoftmaxModel::load(&self.path) {
                Ok(model) => {
                    log::info!("분류 모델을 읽었다: {} (카테고리 {}개)", self.path.display(), model.header.categories.len());
                    state.model = Some(Arc::new(model));
                }
                Err(e) => log::warn!("분류 모델을 다시 읽지 못해 이전 모델을 쓴다: {e}"),
            }
        }
        state.model.clone()
    }
}
