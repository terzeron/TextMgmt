#!/usr/bin/env python

import logging.config
import os
import shutil
import time
from pathlib import Path

import pytest
from elasticsearch import Elasticsearch

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY = "_epub"


@pytest.fixture(scope="module")
def backend_test_setup(elasticsearch_container):
    """Create BookManager and TestClient with test data loaded using testcontainers."""
    from elasticsearch import BadRequestError
    from fastapi.testclient import TestClient
    from backend.main import app
    from backend.book_manager import BookManager
    from utils.loader import Loader

    # Create BookManager (it will use env vars set by elasticsearch_container fixture)
    bm = BookManager()

    # Override ES client with longer timeout for testcontainers
    bm.es_manager.es = Elasticsearch(
        hosts=[os.environ["TM_ES_URL"]],
        basic_auth=(os.environ.get("TM_ES_USER", ""), os.environ.get("TM_ES_PASSWORD", "")),
        request_timeout=120,
        retry_on_timeout=True,
        verify_certs=False,
        max_retries=5
    )

    # Wait for cluster to be ready
    for _ in range(60):
        try:
            health = bm.es_manager.es.cluster.health(wait_for_status="yellow", timeout="5s")
            LOGGER.info("Cluster health: %s", health["status"])
            break
        except Exception as e:
            LOGGER.warning("Waiting for cluster: %s", e)
            time.sleep(1)

    # Delete index if exists, then create fresh
    try:
        if bm.es_manager.do_exist_index():
            bm.es_manager.delete_index()
    except Exception as e:
        LOGGER.warning("Error deleting index: %s", e)

    # Create index with single-node compatible settings (no replicas)
    try:
        settings = {
            "index": {
                "similarity": {"default": {"type": "BM25"}},
                "number_of_shards": 1,
                "number_of_replicas": 0,
            }
        }
        mappings = {
            "properties": {
                "category": {"type": "keyword"},
                "title": {"type": "text", "analyzer": "nori", "fields": {"keyword": {"type": "keyword"}}},
                "author": {"type": "text", "analyzer": "nori", "fields": {"keyword": {"type": "keyword"}}},
                "file_path": {"type": "keyword"},
                "file_type": {"type": "keyword"},
                "file_size": {"type": "unsigned_long"},
                "summary": {"type": "text", "analyzer": "nori"},
                "updated_time": {"type": "date"},
            }
        }
        bm.es_manager.es.indices.create(index=bm.es_manager.index_name, settings=settings, mappings=mappings)
        LOGGER.info("Index created: %s", bm.es_manager.index_name)
    except BadRequestError as e:
        if "resource_already_exists_exception" not in str(e):
            raise
        LOGGER.info("Index already exists")

    # Wait for index to be ready
    bm.es_manager.es.cluster.health(index=bm.es_manager.index_name, wait_for_status="yellow", timeout="30s")

    # Load test data from actual files if available
    epub_path = bm.path_prefix / CATEGORY
    if epub_path.exists():
        data = Loader.read_files(epub_path, num_files=100)
        if data:
            bm.es_manager.insert(data, num_docs=100)
            LOGGER.info("Inserted %d epub documents", len(data))

    # Refresh and wait for data to be searchable
    try:
        bm.es_manager.es.indices.refresh(index=bm.es_manager.index_name)
    except Exception as e:
        LOGGER.warning("Failed to refresh index: %s", e)

    # Verify documents are searchable
    for attempt in range(30):
        try:
            count = bm.es_manager.es.count(index=bm.es_manager.index_name)["count"]
            if count > 0:
                LOGGER.info("Documents ready: %d", count)
                break
        except Exception as e:
            LOGGER.warning("Count failed (attempt %d): %s", attempt, e)
        time.sleep(0.2)

    client = TestClient(app)

    yield {"bm": bm, "client": client}

    try:
        bm.es_manager.delete_index()
    except Exception:
        pass


@pytest.fixture
def test_book(backend_test_setup):
    """Create a temporary test book for each test."""
    import asyncio
    from backend.book import Book
    from utils.loader import Loader

    bm = backend_test_setup["bm"]
    client = backend_test_setup["client"]

    epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
    if not epub_files:
        pytest.skip("No epub files available for testing")

    epub_file_path = epub_files[0]
    temp_file_path = Book.path_prefix / epub_file_path.parent.name / ("temp_" + epub_file_path.name)
    shutil.copy(epub_file_path, temp_file_path)
    data = Loader.read_file(temp_file_path)

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    try:
        book_id, error = loop.run_until_complete(bm.add_book(data))
        assert book_id and not error

        book, error = loop.run_until_complete(bm.get_book(book_id))
        assert book and not error

        yield {"book": book, "bm": bm, "client": client}

        # Cleanup: delete the test book
        try:
            client.delete(f"/books/{book.book_id}")
        except Exception:
            pass
    finally:
        loop.close()


class TestBackend:

    @pytest.mark.asyncio
    async def test_update_book(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        doc = {
            "book_id": book.book_id,
            "category": book.category,
            "title": "renamed_" + book.title,
            "author": "anonymous_" + book.author,
            "file_path": book.category + "/renamed_" + book.title + "." + book.file_type,
            "file_type": book.file_type,
            "file_size": 100,
            "summary": "summary1",
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        response = client.put(f"/books/{book.book_id}", json=doc)
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}

    @pytest.mark.asyncio
    async def test_delete_book(self, backend_test_setup):
        from backend.book import Book
        from utils.loader import Loader

        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
        if not epub_files:
            pytest.skip("No epub files available for testing")

        epub_file_path = epub_files[0]
        temp_file_path = Book.path_prefix / epub_file_path.parent.name / ("to_be_deleted_" + epub_file_path.name)
        shutil.copy(epub_file_path, temp_file_path)
        data = Loader.read_file(temp_file_path)

        book_id, error = await bm.add_book(data)
        assert book_id and not error

        book, error = await bm.get_book(book_id)
        assert book and not error

        response = client.delete(f"/books/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}

    @pytest.mark.asyncio
    async def test_get_file_content(self, test_book):
        from backend.book_manager import BookManager

        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/download/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.content
        assert len(response.content) > 1024
        media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
        assert response.headers["Content-Type"].split(";")[0] == media_type

    @pytest.mark.asyncio
    async def test_get_book(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/books/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {
            "status": "success",
            "result": book.dict()
        }

    @pytest.mark.asyncio
    async def test_get_books_in_category(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/categories/{book.category}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert book.book_id in [b["book_id"] for b in books]

    @pytest.mark.asyncio
    async def test_get_categories(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get("/categories")
        assert response
        assert response.status_code == 200
        assert response.json()["status"] == "success"
        assert book.category in response.json()["result"]

    @pytest.mark.asyncio
    async def test_get_similar_book_list(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/similar/{book.book_id}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0

    @pytest.mark.asyncio
    async def test_search_by_keyword(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        keyword = book.title
        response = client.get(f"/search/{keyword}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert book.book_id in [b["book_id"] for b in books]


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
