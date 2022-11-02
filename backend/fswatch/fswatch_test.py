#!/usr/bin/env python

import os
import sys
from fswatch import Monitor
from datetime import datetime

monitor = Monitor()
monitor.set_recursive()
monitor.add_path(sys.argv[1])

OVERFLOW_MASK =     0b0010000000000000
LINK_MASK =         0b0001000000000000
ISSYMLINK_MASK =    0b0000100000000000
ISDIR_MASK =        0b0000010000000000
ISFILE_MASK =       0b0000001000000000
MOVEDTO_MASK =      0b0000000100000000
MOVEDFROM_MASK =    0b0000000010000000
ATTRMODIFIED_MASK = 0b0000000001000000
OWNMODIFIED_MASK =  0b0000000000100000
RENAMED_MASK =      0b0000000000010000
REMOVED_MASK =      0b0000000000001000
UPDATED_MASK =      0b0000000000000100
CREATED_MASK =      0b0000000000000010
PLATFORMSPEC_MASK = 0b0000000000000001

def get_flag_name(flag):
    result = []
    if bool(flag & OVERFLOW_MASK):
        result.append("overflow")
    if bool(flag & LINK_MASK):
        result.append("link")
    if bool(flag & ISSYMLINK_MASK):
        result.append("issymlink")
    if bool(flag & ISDIR_MASK):
        result.append("isdir")
    if bool(flag & ISFILE_MASK):
        result.append("isfile")
    if bool(flag & MOVEDTO_MASK):
        result.append("movedto")
    if bool(flag & MOVEDFROM_MASK):
        result.append("movedfrom")
    if bool(flag & ATTRMODIFIED_MASK):
        result.append("attrmodified")
    if bool(flag & OWNMODIFIED_MASK):
        result.append("ownmodified")
    if bool(flag & RENAMED_MASK):
        result.append("renamed")
    if bool(flag & REMOVED_MASK):
        result.append("removed")
    if bool(flag & UPDATED_MASK):
        result.append("updated")
    if bool(flag & CREATED_MASK):
        result.append("created")
    if bool(flag & PLATFORMSPEC_MASK):
        result.append("platformspec")
    else:
        result.append("noop")
    return result


def callback(path, evt_time, flags):
    print(path.decode(), evt_time)
    for flag in flags:
        flag_name = get_flag_name(flag)
        if bool(flag & RENAMED_MASK) or bool(flag & REMOVED_MASK) or bool(flag & UPDATED_MASK) or bool(flag & CREATED_MASK):
            print("********************************", end=" ")
        else:
            print("                                ", end=" ")
        print(flag_name)
        
monitor.set_callback(callback)
monitor.start()
