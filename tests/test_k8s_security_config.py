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
