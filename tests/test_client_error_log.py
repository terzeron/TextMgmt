import pytest
from fastapi.testclient import TestClient
from backend.main import app
from backend.auth import create_jwt_token, ACCESS_COOKIE_NAME


@pytest.fixture
def client():
    return TestClient(app)


def test_log_client_error_anonymous(client, caplog):
    """비로그인(익명) 사용자의 에러 로그 전송 성공 및 로그 확인"""
    payload = {
        "error_type": "REACT_RENDER_ERROR",
        "message": "Cannot read properties of null (reading 'map')",
        "stack": "TypeError: Cannot read properties of null\n    at BookView.jsx:42:15",
        "component_stack": "\n    at BookView\n    at App",
        "url": "http://localhost:3000/book-view/1",
        "user_agent": "Mozilla/5.0 TestBrowser",
        "timestamp": "2026-08-30T11:30:00Z",
    }
    with caplog.at_level("ERROR"):
        response = client.post("/logs/client-error", json=payload)
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}
    assert "[CLIENT_ERROR] type=REACT_RENDER_ERROR, user=anonymous(anonymous)" in caplog.text
    assert "Cannot read properties of null" in caplog.text
    assert "Component Stack:" in caplog.text
    assert "Stack Trace:" in caplog.text


def test_log_client_error_authenticated(client, caplog):
    """로그인된 사용자의 에러 로그 전송 시 사용자 이메일 및 역할 로깅 확인"""
    token = create_jwt_token("viewer@test.com", "viewer")
    payload = {
        "error_type": "WINDOW_ERROR",
        "message": "Uncaught SyntaxError: Unexpected token",
        "url": "http://localhost:3000/viewer/epub/123",
    }
    with caplog.at_level("ERROR"):
        response = client.post(
            "/logs/client-error",
            json=payload,
            cookies={ACCESS_COOKIE_NAME: token},
        )
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}
    assert "[CLIENT_ERROR] type=WINDOW_ERROR, user=viewer@test.com(viewer)" in caplog.text


def test_log_client_error_invalid_error_type(client):
    """허용되지 않은 error_type 전송 시 422 검증 오류"""
    payload = {
        "error_type": "INVALID_ERROR_TYPE",
        "message": "Some error",
        "url": "http://localhost:3000/",
    }
    response = client.post("/logs/client-error", json=payload)
    assert response.status_code == 422


def test_log_client_error_missing_required_fields(client):
    """필수 필드(message, url 등) 누락 시 422 검증 오류"""
    payload = {
        "error_type": "UNHANDLED_PROMISE",
        # message and url missing
    }
    response = client.post("/logs/client-error", json=payload)
    assert response.status_code == 422


# --- 무인증 경로 입력 상한과 로그 위조 방어 (CWE-117, CWE-770) ---


def test_log_client_error_truncates_oversized_fields(client, caplog):
    """상한을 넘는 필드는 거부하지 않고 잘라서 기록한다."""
    from backend.main import CLIENT_ERROR_FIELD_LIMITS

    payload = {
        "error_type": "CUSTOM_ERROR",
        "message": "A" * 10000,
        "stack": "B" * 50000,
        "component_stack": "C" * 50000,
        "url": "http://localhost/" + "D" * 10000,
        "user_agent": "E" * 5000,
        "timestamp": "F" * 5000,
    }
    with caplog.at_level("ERROR"):
        response = client.post("/logs/client-error", json=payload)

    assert response.status_code == 200
    assert "A" * CLIENT_ERROR_FIELD_LIMITS["message"] in caplog.text
    assert "A" * (CLIENT_ERROR_FIELD_LIMITS["message"] + 1) not in caplog.text
    assert "B" * CLIENT_ERROR_FIELD_LIMITS["stack"] in caplog.text
    assert "B" * (CLIENT_ERROR_FIELD_LIMITS["stack"] + 1) not in caplog.text


def test_log_client_error_strips_newlines_from_single_line_fields(client, caplog):
    """message 의 개행으로 가짜 로그 줄을 심을 수 없어야 한다."""
    payload = {
        "error_type": "CUSTOM_ERROR",
        "message": "real error\n[CLIENT_ERROR] type=FAKE, user=admin@evil.com(admin)",
        "url": "http://localhost/x",
    }
    with caplog.at_level("ERROR"):
        response = client.post("/logs/client-error", json=payload)

    assert response.status_code == 200
    record = next(r for r in caplog.records if "[CLIENT_ERROR] type=" in r.getMessage())
    logged = record.getMessage()
    # 개행이 공백이 되어 한 줄로 남는다. 새 줄이 생기지 않으므로 로그를 읽을 때
    # 위조된 항목이 별도 기록으로 보이지 않는다.
    assert "\n" not in logged
    assert "real error [CLIENT_ERROR] type=FAKE" in logged


def test_log_client_error_keeps_newlines_in_stack(client, caplog):
    """stack 은 여러 줄이 의미를 가지므로 개행을 보존한다."""
    payload = {
        "error_type": "CUSTOM_ERROR",
        "message": "boom",
        "stack": "TypeError: boom\n    at A.jsx:1:1\n    at B.jsx:2:2",
        "url": "http://localhost/x",
    }
    with caplog.at_level("ERROR"):
        response = client.post("/logs/client-error", json=payload)

    assert response.status_code == 200
    assert "at A.jsx:1:1\n    at B.jsx:2:2" in caplog.text


def test_log_client_error_accepts_realistic_payload_unchanged(client, caplog):
    """정상 클라이언트가 보내는 크기는 잘리지 않아야 한다(프론트는 이미 자른다)."""
    message = "Cannot read properties of null (reading 'map')"
    url = "https://tm.terzeron.com/book-edit/204252164?category=" + "%EC%97%AD" * 40
    payload = {
        "error_type": "REACT_RENDER_ERROR",
        "message": message,
        "stack": "TypeError\n" + "    at Component.jsx:10:5\n" * 100,
        "url": url,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36",
    }
    with caplog.at_level("ERROR"):
        response = client.post("/logs/client-error", json=payload)

    assert response.status_code == 200
    assert message in caplog.text
    assert url in caplog.text  # 422 자 URL 이 그대로 남는다
