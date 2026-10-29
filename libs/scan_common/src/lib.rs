//! 파일 스캐너(`epubscan`, `pdfscan`)가 함께 쓰는 공용 코드.
//!
//! 포맷을 해석하는 규칙(Scanner, 지표, JSON 모양)은 각 도구에 둔다. 여기에는 포맷과 무관한 것만 둔다.
//!
//! - [`files`]: 입력 경로에서 대상 파일을 모은다(정렬, 중복 제거, 읽을 수 없는 경로는 버리지 않고 남긴다).
//! - [`stream`]: 보고서를 창 단위로 검사하고 바로 내보낸 뒤 버린다. 파일 수가 아니라 창 크기에 비례해 메모리를 쓴다.
//! - [`workers`]: 병렬 풀, 앞에서 N건만 처리, 메모리로 제한하는 워커 수.
//! - [`finding`]: bookdoctor의 지표·파일 보고서 모델. [`sniff`]: 매직바이트 판별. [`names`]: Windows 예약 이름.
//! - [`args`]: 명령행 옵션 읽기. [`text`]: 사람이 읽는 출력용 문자열 정리. [`severity`]: 심각도.
//!
//! `corpus_common`과 별도로 둔 것은 이 코드가 코퍼스와 무관한 파일 스캐너의 것이기 때문이다. 의존성도 `rayon` 하나다.

pub mod args;
pub mod files;
pub mod finding;
pub mod names;
pub mod severity;
pub mod sniff;
pub mod stream;
pub mod text;
pub mod workers;

pub use finding::{Category, FileReport, Finding, Format, ViewerBlocked};
pub use severity::Severity;
pub use stream::{Report, Summary, scan_streaming};
pub use workers::{DEFAULT_WORKERS, limited, pool, workers_within_memory};
