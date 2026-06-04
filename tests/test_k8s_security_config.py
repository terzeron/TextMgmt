from pathlib import Path


def test_httproute_routes_wake_only_from_frontend_host():
    route_path = Path(__file__).resolve().parent.parent / "k8s" / "httproute.yml"
    text = route_path.read_text(encoding="utf-8")

    frontend_section, backend_section, _ = text.split("\n---\n", 2)

    assert 'hostnames:\n    - "tm.terzeron.com"' in frontend_section
    assert "type: Exact" in frontend_section
    assert "value: /wake" in frontend_section
    assert "- name: tm-backend" in frontend_section
    assert "port: 8020" in frontend_section

    assert 'hostnames:\n    - "api-tm.terzeron.com"' in backend_section
    assert "value: /wake" not in backend_section


def test_es_appuser_uses_least_privilege_role_not_superuser():
    """C1 회귀 방지: appuser는 superuser가 아니라 범위 제한된 tm_role을 사용해야 한다."""
    es_path = Path(__file__).resolve().parent.parent / "k8s" / "es.sh"
    # 주석(설명에 superuser 등 단어 포함 가능)은 제외하고 실제 실행되는 명령만 검사한다
    code = "\n".join(line for line in es_path.read_text(encoding="utf-8").splitlines() if not line.lstrip().startswith("#"))

    # appuser에 superuser 롤을 부여하지 않는다
    assert '[\\"superuser\\"]' not in code, "appuser에 superuser 권한이 다시 들어갔습니다 (최소권한 위반)"

    # appuser는 tm_role 만 사용한다
    assert "_security/user/appuser" in code
    assert '[\\"tm_role\\"]' in code, "appuser가 tm_role을 사용하도록 설정돼 있어야 한다"

    # tm_role 은 manage/read/write 만 부여하고, 인덱스에 all 권한을 주지 않는다
    assert "_security/role/tm_role" in code
    assert '\\"manage\\",\\"read\\",\\"write\\"' in code
    assert '\\"all\\"' not in code, "tm_role에 과도한 all 권한이 부여되어 있습니다"

    # 앱이 사용하지 않는 클러스터 권한은 부여하지 않는다
    assert "manage_index_templates" not in code


def test_es_http_tls_enabled():
    """C4 회귀 방지: ES HTTP TLS(자체서명)가 비활성화돼 있으면 안 된다."""
    values_path = Path(__file__).resolve().parent.parent / "k8s" / "es-values.yml"
    text = values_path.read_text(encoding="utf-8")

    # selfSignedCertificate.disabled: true 로 TLS 를 끄지 않는다
    assert "disabled: true" not in text, "ES HTTP TLS 가 비활성화되어 있습니다 (평문 전송)"


def test_backend_connects_to_es_over_https():
    """C4 회귀 방지: backend 의 TM_ES_URL 은 https 여야 하고 평문 http 로 ES 에 붙지 않는다."""
    dep_path = Path(__file__).resolve().parent.parent / "k8s" / "tm-deployment.yml"
    text = dep_path.read_text(encoding="utf-8")

    assert "https://elasticsearch-es-http:9200" in text
    assert "http://elasticsearch-es-http:9200" not in text, "ES 에 평문 http 로 연결하고 있습니다"


def test_backend_security_context_pins_numeric_uid():
    """배포 회귀 방지: runAsNonRoot 사용 시 비숫자 이미지 USER(terzeron) 검증을 위해
    runAsUser(숫자 UID)를 명시해야 한다. (누락 시 CreateContainerConfigError)"""
    dep_path = Path(__file__).resolve().parent.parent / "k8s" / "tm-deployment.yml"
    backend_section = dep_path.read_text(encoding="utf-8").split("\n---\n")[0]

    assert "runAsNonRoot: true" in backend_section
    assert "runAsUser: 1000" in backend_section, "runAsNonRoot 와 함께 runAsUser(숫자 UID)가 필요합니다"
