from pathlib import Path


def test_nginx_default_conf_includes_security_headers():
    conf_path = Path(__file__).resolve().parent.parent / "frontend" / "default.conf"
    text = conf_path.read_text(encoding="utf-8")

    assert "Content-Security-Policy" in text
    assert "script-src 'self' https://accounts.google.com" in text
    assert "object-src 'none'" in text
    assert "frame-ancestors 'self'" in text
    assert "X-Content-Type-Options" in text
    assert '"nosniff"' in text
    assert "Referrer-Policy" in text
    assert '"strict-origin-when-cross-origin"' in text
    assert "Permissions-Policy" in text
    assert '"camera=(), microphone=(), geolocation=()"' in text
    assert "X-Frame-Options" in text
    assert '"SAMEORIGIN"' in text
