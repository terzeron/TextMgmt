#!/usr/bin/env python

import os
import logging.config
from pathlib import Path

from backend.book_manager import BookManager
from backend.comics import Comics
from backend.es_manager import ESManager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class ComicsManager(BookManager):
    item_class = Comics

    def __init__(self) -> None:
        if "TM_COMICS_DIR" not in os.environ:
            raise RuntimeError("The environment variable TM_COMICS_DIR is not set.")

        self.path_prefix = Path(os.environ["TM_COMICS_DIR"])
        LOGGER.debug(self.path_prefix)
        self.es_manager = ESManager(index_name=os.environ["TM_ES_COMICS_INDEX"])
        self.es_manager.create_index()
        self._backfill_created_time_if_enabled()
        self._mismatch_cache = None
        self._mismatch_cache_time: float = 0.0
