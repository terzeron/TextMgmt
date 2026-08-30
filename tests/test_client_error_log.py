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
