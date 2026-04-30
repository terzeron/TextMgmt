import sys
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


class DummyESManager:
    def __init__(self, update_result=True, delete_result=True):
        self.update_result = update_result
        self.delete_result = delete_result
        self.updated = []
        self.deleted = []

    def update(self, **kwargs):
        self.updated.append(kwargs)
        return self.update_result

    def search_by_id(self, doc_id):
        return {"book_id": doc_id, "title": "book"}

    def delete(self, doc_id):
        self.deleted.append(doc_id)
        return self.delete_result


def test_main_success(monkeypatch, capsys):
    import utils.update as update_mod

    es = DummyESManager(update_result=True, delete_result=True)
    monkeypatch.setattr(update_mod, "ESManager", lambda: es)

    assert update_mod.main() == 0
    assert es.updated[0]["doc_id"] == 3384
    assert es.deleted == [3384]
    assert "book_id" in capsys.readouterr().out


def test_main_fails_when_update_fails(monkeypatch):
    import utils.update as update_mod

    es = DummyESManager(update_result=False, delete_result=True)
    monkeypatch.setattr(update_mod, "ESManager", lambda: es)

    assert update_mod.main() == -1
    assert es.deleted == []


def test_main_fails_when_delete_fails(monkeypatch):
    import utils.update as update_mod

    es = DummyESManager(update_result=True, delete_result=False)
    monkeypatch.setattr(update_mod, "ESManager", lambda: es)

    assert update_mod.main() == -1
    assert es.deleted == [3384]
