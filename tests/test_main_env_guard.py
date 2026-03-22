import importlib
import os
import sys

import pytest


def _reload_without_env(module_name, env_key):
    prev = os.environ.pop(env_key, None)
    try:
        if module_name in sys.modules:
            del sys.modules[module_name]
        with pytest.raises(SystemExit):
            importlib.import_module(module_name)
    finally:
        if prev is not None:
            os.environ[env_key] = prev


def test_main_requires_frontend_url():
    _reload_without_env("backend.main", "TM_FRONTEND_URL")
