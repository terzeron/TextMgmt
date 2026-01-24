#!/usr/bin/env python

import os
import time
import urllib.request
import urllib.error
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

    def _wait_for_ready(self) -> None:
        """Wait until ES cluster is ready (yellow or green status)."""
        url = f"{self.get_url()}/_cluster/health?wait_for_status=yellow&timeout=120s"
        while True:
            try:
                with urllib.request.urlopen(url, timeout=130) as response:
                    if response.status == 200:
                        return
            except (urllib.error.URLError, TimeoutError, ConnectionRefusedError, OSError):
                pass
            time.sleep(1)

    def start(self):
        super().start()
        # Wait for "started" log which indicates ES is ready
        wait_for_logs(self, "started")
        # Wait for cluster to be ready (yellow status)
        self._wait_for_ready()
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
