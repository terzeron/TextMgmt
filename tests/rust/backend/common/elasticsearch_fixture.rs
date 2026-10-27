//! `tests/conftest.py`의 Nori Elasticsearch 컨테이너 대응 fixture.
//! 이미지를 재사용하고, 없을 때만 Python fixture와 같은 8.15.0 + analysis-nori 이미지를 만든다.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use es_client::EsClient;
use testcontainers_modules::testcontainers::core::{ContainerPort, WaitFor};
use testcontainers_modules::testcontainers::runners::SyncRunner;
use testcontainers_modules::testcontainers::{Container, Image};

const IMAGE_NAME: &str = "elasticsearch-nori";
const IMAGE_TAG: &str = "8.15.0";
const IMAGE: &str = "elasticsearch-nori:8.15.0";
const API_PORT: ContainerPort = ContainerPort::Tcp(9200);

#[derive(Debug, Default, Clone)]
struct NoriElasticsearch;

impl Image for NoriElasticsearch {
    fn name(&self) -> &str {
        IMAGE_NAME
    }

    fn tag(&self) -> &str {
        IMAGE_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::seconds(1)]
    }

    fn env_vars(&self) -> impl IntoIterator<Item = (impl Into<std::borrow::Cow<'_, str>>, impl Into<std::borrow::Cow<'_, str>>)> {
        [("discovery.type", "single-node"), ("xpack.security.enabled", "false"), ("ES_JAVA_OPTS", "-Xms512m -Xmx512m"), ("ES_SETTING_CLUSTER_ROUTING_ALLOCATION_DISK_THRESHOLD__ENABLED", "false")]
    }

    fn expose_ports(&self) -> &[ContainerPort] {
        &[API_PORT]
    }
}

static CONTAINER: Mutex<Option<Container<NoriElasticsearch>>> = Mutex::new(None);
static READY: OnceLock<bool> = OnceLock::new();

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> i32;
}

extern "C" fn remove_container_at_exit() {
    let _ = std::panic::catch_unwind(|| {
        if let Ok(mut guard) = CONTAINER.lock() {
            guard.take();
        }
    });
}

fn ensure_nori_image() {
    let inspected = Command::new("docker").args(["image", "inspect", IMAGE]).output().expect("Docker CLI를 실행할 수 없다");
    if inspected.status.success() {
        return;
    }

    let mut build = Command::new("docker").args(["build", "--tag", IMAGE, "-"]).stdin(Stdio::piped()).spawn().expect("Nori Elasticsearch 이미지를 만들 Docker CLI를 실행할 수 없다");
    build
        .stdin
        .take()
        .expect("Docker build stdin")
        .write_all(b"FROM docker.elastic.co/elasticsearch/elasticsearch:8.15.0\nRUN bin/elasticsearch-plugin install analysis-nori --batch\n")
        .expect("Nori Elasticsearch Dockerfile을 전달하지 못했다");
    let status = build.wait().expect("Nori Elasticsearch 이미지 빌드 결과를 읽지 못했다");
    assert!(status.success(), "Nori Elasticsearch 이미지 빌드 실패");
}

/// 세션당 컨테이너 하나를 띄우고 ES 접속 환경 변수를 설정한다.
pub fn elasticsearch_container() -> String {
    READY.get_or_init(|| {
        ensure_nori_image();
        let container = NoriElasticsearch.start().expect("Nori Elasticsearch 컨테이너 시작 실패");
        let host = container.get_host().expect("Elasticsearch 컨테이너 호스트").to_string();
        let port = container.get_host_port_ipv4(9200).expect("Elasticsearch 컨테이너 포트");
        let url = format!("http://{host}:{port}");

        let client = EsClient::new(&url, "elastic", "", "test_index");
        client.connect_with_retry(60, &|_| std::thread::sleep(std::time::Duration::from_secs(1))).expect("Nori Elasticsearch 준비 대기 실패");

        // SAFETY: OnceLock 초기화가 끝나기 전 한 번만 설정한다.
        unsafe {
            // `common::setup_env()` 가 나중에 돌아도 컨테이너 값을 지우지 않게 표식을 남긴다(common::ES_FROM_FIXTURE).
            std::env::set_var("TM_TEST_ES_FROM_FIXTURE", "1");
            std::env::set_var("TM_ES_URL", &url);
            std::env::set_var("TM_ES_BOOK_INDEX", "test_index");
            std::env::set_var("TM_ES_USER", "elastic");
            std::env::set_var("TM_ES_PASSWORD", "");
            std::env::remove_var("TM_ES_CA_CERT");
        }
        *CONTAINER.lock().unwrap() = Some(container);
        // SAFETY: callback은 panic을 FFI 경계 밖으로 내보내지 않는다.
        unsafe { atexit(remove_container_at_exit) };
        true
    });
    std::env::var("TM_ES_URL").expect("Elasticsearch fixture URL")
}
