#!/usr/bin/env python

import os
import sys
import platform
import pymysql
import logging.config
from threading import Thread
from pathlib import Path
from debounce import debounce

logging.config.fileConfig("logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
# ignore debug logs from inotify.adapters
logging.getLogger("inotify.adapters").setLevel(logging.WARNING)

if platform.system() == "Linux":
    import inotify.adapters


@debounce(60)
def trigger_update(self):
    LOGGER.debug("# trigger_update()")
    host = os.environ["TM_DB_HOST"] if "TM_DB_HOST" in os.environ else ""
    port = os.environ["TM_DB_PORT"] if "TM_DB_PORT" in os.environ else ""
    db = os.environ["TM_DB_NAME"] if "TM_DB_NAME" in os.environ else ""
    user = os.environ["TM_DB_USER"] if "TM_DB_USER" in os.environ else ""
    passwd = os.environ["TM_DB_PASSWD"] if "TM_DB_PASSWD" in os.environ else ""
    conn = pymysql.connect(host=host, user=user, password=passwd, database=db, port=int(port), charset="utf8", cursorclass=pymysql.cursors.DictCursor)
    conn.cursor().execute("UPDATE fs_modification SET last_modified_time = NOW()")
    conn.commit()
    LOGGER.debug("updated last modified time of file system")
    conn.cursor().close()
    conn.close()


def inotify_worker(path_prefix: Path):
    LOGGER.debug(f"# inotify_worker(path={path_prefix})")
    inotify_client = inotify.adapters.InotifyTree(str(path_prefix))
    for event in inotify_client.event_gen(yield_nones=False):
        _, type_names, path, filename = event
        for type_name in type_names:
            if type_name in ("IN_CREATE", "IN_DELETE", "IN_MOVED_FROM", "IN_MOVED_TO", "IN_CLOSE_WRITE", "IN_MODIFY", "IN_DELETE_SELF", "IN_MOVE_SELF"):
                LOGGER.debug(f"PATH=[{path}] FILENAME=[{filename}] EVENT_TYPE={type_name}")
                trigger_update()


def main() -> int:
    if platform.system() == "Linux":
        while True:
            t = Thread(target=inotify_worker, args=(os.environ["TM_WORK_DIR"],), daemon=True)
            t.start()
            t.join()
    else:
        LOGGER.error("can't monitor filesystem changes on this platform")
    return 0


if __name__ == "__main__":
    sys.exit(main())
