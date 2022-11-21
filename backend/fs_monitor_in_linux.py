#!/usr/bin/env python

import logging.config
from pathlib import Path
from threading import Thread
import inotify.adapters
from text_manager import TextManager

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
# ignore debug logs from inotify.adapters
logging.getLogger("inotify.adapters").setLevel(logging.WARNING)

def inotify_worker(text_manager: TextManager):
    LOGGER.debug(f"# inotify_worker(path={text_manager.path_prefix})")
    inotify_client = inotify.adapters.InotifyTree(str(text_manager.path_prefix))
    for event in inotify_client.event_gen(yield_nones=False):
        _, type_names, path, filename = event
        for type_name in type_names:
            if type_name in ("IN_CREATE", "IN_DELETE", "IN_MOVED_FROM", "IN_MOVED_TO", "IN_CLOSE_WRITE", "IN_MODIFY", "IN_DELETE_SELF", "IN_MOVE_SELF"):
                LOGGER.debug(f"PATH=[{path}] FILENAME=[{filename}] EVENT_TYPE={type_name}")
                text_manager.reset_cache()

def start(text_manager: TextManager):
    Thread(target=inotify_worker, args=(text_manager, ), daemon=True).start()
