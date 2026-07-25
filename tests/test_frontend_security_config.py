from pathlib import Path


def test_nginx_default_conf_includes_security_headers():
    frontend_dir = Path(__file__).resolve().parent.parent / "frontend"
    sec_conf = frontend_dir / "security-headers.conf"
    text = sec_conf.read_text(encoding="utf-8") if sec_conf.exists() else (frontend_dir / "default.conf").read_text(encoding="utf-8")

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
