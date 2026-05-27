#!/usr/bin/env python

import os
import time
import urllib.request
import urllib.error
import json
import warnings
import logging
from pathlib import Path
import pytest

LOGGER = logging.getLogger(__name__)

# XMLParsedAsHTMLWarning 경고 전역 억제
warnings.filterwarnings("ignore", message=".*XML.*HTML.*")

# 기본 테스트 환경 변수 설정 (import 시점 SystemExit 방지)
# Book.path_prefix는 import 시점에 평가되므로 반드시 import 전에 설정해야 한다
PROJECT_ROOT = Path(__file__).parent.parent
os.environ["TM_BOOK_DIR"] = str(PROJECT_ROOT / "tests/books")
os.environ["TM_COMICS_DIR"] = str(PROJECT_ROOT / "tests/comics")
os.environ["TM_FRONTEND_URL"] = "http://localhost:3000"
os.environ["TM_JWT_SECRET"] = "test_jwt_secret_for_testing_minimum_32bytes"
os.environ["TM_ADMIN_EMAIL"] = "admin@test.com"
os.environ["TM_ALLOWED_EMAILS"] = "viewer@test.com"
os.environ.setdefault("TM_ES_COMICS_INDEX", "test_comics_index")

# Disable Ryuk to avoid docker.sock mount issues with Colima
os.environ["TESTCONTAINERS_RYUK_DISABLED"] = "true"

# env var 설정 후 import해야 Book.path_prefix가 올바른 경로로 초기화된다
from backend.book import Book  # noqa: E402
from backend.comics import Comics  # noqa: E402


@pytest.fixture(autouse=True)
def restore_model_path_prefixes():
    original_book_prefix = Book.path_prefix
    original_comics_prefix = Comics.path_prefix
    try:
        yield
    finally:
        Book.path_prefix = original_book_prefix
        Comics.path_prefix = original_comics_prefix


from testcontainers.core.container import DockerContainer  # noqa: E402
from testcontainers.mysql import MySqlContainer  # noqa: E402


# ========== MySQL 컨테이너 fixture ==========

MYSQL_USER = "testuser"
MYSQL_PASSWORD = "testpass"
MYSQL_DATABASE = "testdb"
MYSQL_ROOT_PASSWORD = "rootpass"


@pytest.fixture(scope="session")
def mysql_container():
    """Start MySQL container for testing."""
    print("\n>>> Starting MySQL container...")
    try:
        mysql = MySqlContainer(image="mysql:8.0", username=MYSQL_USER, password=MYSQL_PASSWORD, root_password=MYSQL_ROOT_PASSWORD, dbname=MYSQL_DATABASE)

        with mysql:
            host = mysql.get_container_host_ip()
            port = mysql.get_exposed_port(3306)
            print(f">>> MySQL container ready at {host}:{port}")

            os.environ["TM_MYSQL_HOST"] = host
            os.environ["TM_MYSQL_PORT"] = str(port)
            os.environ["TM_MYSQL_DATABASE"] = MYSQL_DATABASE
            os.environ["TM_MYSQL_USER"] = MYSQL_USER
            os.environ["TM_MYSQL_PASSWORD"] = MYSQL_PASSWORD

            yield mysql
        print(">>> MySQL container stopped")
    except Exception as e:
        pytest.skip(f"Docker/MySQL container unavailable: {e}")


# ========== ES 공통 설정 ==========
ES_INDEX_SETTINGS = {
    "index": {
        "similarity": {"default": {"type": "BM25"}},
        "number_of_shards": 1,
        "number_of_replicas": 0,  # Single-node용
    }
}

ES_INDEX_MAPPINGS = {
    "properties": {
        "category": {"type": "keyword", "fields": {"nori": {"type": "text", "analyzer": "nori"}}},
        "title": {"type": "text", "analyzer": "nori", "fields": {"keyword": {"type": "keyword"}}},
        "author": {"type": "text", "analyzer": "nori", "fields": {"keyword": {"type": "keyword"}}},
        "file_path": {"type": "keyword"},
        "file_type": {"type": "keyword"},
        "file_size": {"type": "unsigned_long"},
        "line_count": {"type": "unsigned_long"},
        "page_count": {"type": "unsigned_long"},
        "isbn": {"type": "keyword"},
        "summary": {"type": "text", "analyzer": "nori"},
        "updated_time": {"type": "date"},
    }
}


ES_BASE_IMAGE = "docker.elastic.co/elasticsearch/elasticsearch:8.15.0"
ES_NORI_IMAGE = "elasticsearch-nori:8.15.0"


def _ensure_nori_image() -> str:
    """Build ES image with nori plugin pre-installed if it doesn't exist."""
    import docker

    client = docker.from_env()
    try:
        client.images.get(ES_NORI_IMAGE)
        print(f">>> Using cached image: {ES_NORI_IMAGE}")
    except docker.errors.ImageNotFound:
        print(f">>> Building {ES_NORI_IMAGE} (one-time, installs nori plugin)...")
        import io

        dockerfile = f"FROM {ES_BASE_IMAGE}\nRUN bin/elasticsearch-plugin install analysis-nori --batch\n"
        client.images.build(fileobj=io.BytesIO(dockerfile.encode()), tag=ES_NORI_IMAGE, rm=True)
        print(f">>> Built {ES_NORI_IMAGE}")
    finally:
        client.close()
    return ES_NORI_IMAGE


class ElasticsearchContainer(DockerContainer):
    """Custom Elasticsearch container with nori plugin (auto-builds image if needed)."""

    def __init__(self, image: str = ES_NORI_IMAGE):
        _ensure_nori_image()
        super().__init__(image)
        self.with_exposed_ports(9200)
        self.with_env("discovery.type", "single-node")
        self.with_env("xpack.security.enabled", "false")
        self.with_env("ES_JAVA_OPTS", "-Xms512m -Xmx512m")

    def get_url(self) -> str:
        host = self.get_container_host_ip()
        port = self.get_exposed_port(9200)
        return f"http://{host}:{port}"

    def _wait_for_es_ready(self, timeout: int = 90) -> None:
        """Wait until ES is fully ready for requests."""
        start_time = time.time()

        # 1단계: 클러스터 health가 green/yellow가 될 때까지 대기
        while time.time() - start_time < timeout:
            try:
                url = f"{self.get_url()}/_cluster/health"
                req = urllib.request.Request(url)
                with urllib.request.urlopen(req, timeout=5) as response:
                    if response.status == 200:
                        data = json.loads(response.read().decode("utf-8"))
                        status = data.get("status")
                        if status in ("green", "yellow"):
                            print(f">>> Cluster health: {status}")
                            break
                        print(f">>> Cluster status: {status}, waiting...")
            except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, ConnectionRefusedError, OSError, json.JSONDecodeError) as e:
                print(f">>> Waiting for ES: {type(e).__name__}")
            time.sleep(1)
        else:
            raise TimeoutError(f"ES cluster not ready after {timeout}s")

        # 2단계: 실제 요청이 503 없이 가능해질 때까지 추가 대기
        for _ in range(20):
            try:
                url = f"{self.get_url()}/_cat/health"
                with urllib.request.urlopen(url, timeout=5) as response:
                    if response.status == 200:
                        print(">>> ES fully ready for requests")
                        return
            except urllib.error.HTTPError as e:
                if e.code == 503:
                    print(">>> ES still initializing (503)...")
                else:
                    print(f">>> ES check: HTTP {e.code}")
            except Exception:
                pass
            time.sleep(1)

    def start(self):
        super().start()
        self._wait_for_es_ready(timeout=90)
        return self


@pytest.fixture(scope="session")
def elasticsearch_container():
    """Start Elasticsearch container for testing."""
    print("\n>>> Starting Elasticsearch container (this may take a while for nori plugin installation)...")
    try:
        es = ElasticsearchContainer()
    except Exception as e:
        pytest.skip(f"Docker/ES container unavailable: {e}")

    try:
        with es:
            print(f">>> Elasticsearch container ready at {es.get_url()}")
            # Set environment variables for ESManager
            os.environ["TM_ES_URL"] = es.get_url()
            os.environ["TM_ES_BOOK_INDEX"] = "test_index"
            os.environ["TM_ES_USER"] = "elastic"
            os.environ["TM_ES_PASSWORD"] = ""
            # Set environment variables for backend/main.py
            os.environ["TM_FRONTEND_URL"] = "http://localhost:3000"
            os.environ["VITE_FACEBOOK_APP_ID"] = "test_app_id"
            os.environ["VITE_FACEBOOK_APP_SECRET"] = "test_app_secret"
            # JWT 인증 테스트용 환경변수
            os.environ["TM_JWT_SECRET"] = "test_jwt_secret_for_testing_minimum_32bytes"
            os.environ["TM_ADMIN_EMAIL"] = "admin@test.com"
            os.environ["TM_ALLOWED_EMAILS"] = "viewer@test.com"
            yield es
        print(">>> Elasticsearch container stopped")
    except Exception as e:
        pytest.skip(f"Failed to start ES container: {e}")


@pytest.fixture(scope="session")
def es_client(elasticsearch_container):
    """Session-scoped Elasticsearch client (공유됨, 컨테이너 재시작 없음)."""
    from elasticsearch import Elasticsearch

    try:
        client = Elasticsearch(hosts=[os.environ["TM_ES_URL"]], basic_auth=(os.environ.get("TM_ES_USER", ""), os.environ.get("TM_ES_PASSWORD", "")), request_timeout=30, retry_on_timeout=True, verify_certs=False, max_retries=3)
    except Exception as e:
        pytest.skip(f"ES client creation failed: {e}")

    # 클러스터가 완전히 준비될 때까지 대기 (컨테이너에서 이미 확인됨, 짧게)
    ready = False
    for attempt in range(10):
        try:
            health = client.cluster.health(wait_for_status="yellow", timeout="3s")
            if health["status"] in ("green", "yellow"):
                LOGGER.info("ES client ready, cluster health: %s", health["status"])
                ready = True
                break
        except Exception as e:
            LOGGER.debug("Waiting for cluster: %s (attempt %d)", e, attempt)
        time.sleep(1)

    if not ready:
        pytest.skip("ES cluster not ready after retries")

    yield client


@pytest.fixture(scope="module")
def es_index(es_client):
    """Module-scoped ES index (모듈마다 인덱스 재생성)."""
    from elasticsearch import BadRequestError

    index_name = os.environ["TM_ES_BOOK_INDEX"]

    # 기존 인덱스 삭제
    try:
        if es_client.indices.exists(index=index_name):
            es_client.indices.delete(index=index_name)
            LOGGER.info("Deleted existing index: %s", index_name)
    except Exception as e:
        LOGGER.warning("Error deleting index: %s", e)

    # nori 플러그인 확인
    try:
        plugins = es_client.cat.plugins(format="json")
        nori_installed = any(p.get("component") == "analysis-nori" for p in plugins)
        if not nori_installed:
            pytest.skip("analysis-nori plugin not installed in ES container")
    except Exception as e:
        LOGGER.warning("Failed to check plugins: %s", e)

    # 인덱스 생성
    try:
        es_client.indices.create(index=index_name, settings=ES_INDEX_SETTINGS, mappings=ES_INDEX_MAPPINGS)
        LOGGER.info("Created index: %s", index_name)
    except BadRequestError as e:
        if "resource_already_exists_exception" not in str(e):
            raise
        LOGGER.info("Index already exists: %s", index_name)

    # 인덱스 준비 대기 — shard 할당 실패 시 빠르게 skip
    for attempt in range(3):
        try:
            es_client.cluster.health(index=index_name, wait_for_status="yellow", timeout="10s")
            break
        except Exception as e:
            if attempt == 2:
                pytest.skip(f"ES index shard allocation failed: {e}")
            LOGGER.warning("Waiting for index ready (attempt %d): %s", attempt + 1, e)
            time.sleep(2)

    yield index_name

    # 테스트 후 인덱스 삭제
    try:
        if es_client.indices.exists(index=index_name):
            es_client.indices.delete(index=index_name)
            LOGGER.info("Cleaned up index: %s", index_name)
    except Exception as e:
        LOGGER.warning("Error cleaning up index: %s", e)


@pytest.fixture(scope="session")
def admin_auth_cookies():
    """테스트용 admin JWT 쿠키."""
    from backend.auth import create_jwt_token

    token = create_jwt_token(email="admin@test.com", role="admin", name="Test Admin")
    return {"tm_access_token": token}


@pytest.fixture(scope="function")
def es_clean_data(es_client, es_index):
    """Function-scoped fixture: 테스트 전/후 데이터만 삭제 (인덱스 유지)."""
    # 테스트 전 데이터 삭제
    try:
        es_client.delete_by_query(index=es_index, body={"query": {"match_all": {}}}, refresh=True)
    except Exception:
        pass  # 인덱스가 비어있으면 무시

    yield es_index

    # 테스트 후 데이터 삭제
    try:
        es_client.delete_by_query(index=es_index, body={"query": {"match_all": {}}}, refresh=True)
    except Exception:
        pass
