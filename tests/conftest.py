#!/usr/bin/env python

import os
import time
import urllib.request
import urllib.error
import http.client
import pytest

# Disable Ryuk to avoid docker.sock mount issues with Colima
os.environ["TESTCONTAINERS_RYUK_DISABLED"] = "true"

from testcontainers.core.container import DockerContainer
from testcontainers.core.waiting_utils import wait_for_logs


class ElasticsearchContainer(DockerContainer):
    """Custom Elasticsearch container for ES 9.x support with nori plugin."""

    def __init__(self, image: str = "docker.elastic.co/elasticsearch/elasticsearch:9.1.0"):
        super().__init__(image)
        self.with_exposed_ports(9200)
        self.with_env("discovery.type", "single-node")
        self.with_env("xpack.security.enabled", "false")
        self.with_env("ES_JAVA_OPTS", "-Xms1g -Xmx1g")
        # Install nori plugin on startup
        self.with_command(
            "sh -c 'bin/elasticsearch-plugin install analysis-nori --batch && exec bin/elasticsearch'"
        )

    def get_url(self) -> str:
        host = self.get_container_host_ip()
        port = self.get_exposed_port(9200)
        return f"http://{host}:{port}"

    def _wait_for_http(self, timeout: int = 300) -> None:
        """Wait until ES responds to HTTP requests."""
        url = self.get_url()
        start_time = time.time()
        while time.time() - start_time < timeout:
            try:
                with urllib.request.urlopen(url, timeout=10) as response:
                    if response.status == 200:
                        return
            except (urllib.error.URLError, TimeoutError, ConnectionRefusedError,
                    http.client.RemoteDisconnected, OSError):
                pass
            time.sleep(3)
        raise TimeoutError(f"Elasticsearch not ready after {timeout} seconds")

    def start(self):
        super().start()
        # Wait for ES to be ready (longer timeout for plugin installation)
        wait_for_logs(self, "started", timeout=300)
        # Wait for HTTP endpoint to be ready
        self._wait_for_http(timeout=300)
        return self


@pytest.fixture(scope="session")
def elasticsearch_container():
    """Start Elasticsearch container for testing."""
    print("\n>>> Starting Elasticsearch container (this may take a while for nori plugin installation)...")
    es = ElasticsearchContainer()

    with es:
        print(f">>> Elasticsearch container ready at {es.get_url()}")
        # Set environment variables for ESManager
        os.environ["TM_ES_URL"] = es.get_url()
        os.environ["TM_ES_INDEX"] = "test_index"
        os.environ["TM_ES_USER"] = "elastic"
        os.environ["TM_ES_PASSWORD"] = ""
        yield es
    print(">>> Elasticsearch container stopped")
