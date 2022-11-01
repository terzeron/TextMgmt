#!/usr/bin/env python

import os
from fswatch import Monitor

monitor = Monitor()
monitor.add_path(os.environ["TM_WORK_DIR"])

def callback(path, evn_time, flags, flags_num, event_num):
    print(path.decode())

monitor.set_callback(callback)
monitor.start()
