#!/usr/bin/env python

import threading
import random
from datetime import datetime
import time
from debounce import debounce

last_modified_time = datetime.utcnow()
do_trigger_caching = False

@debounce(600)
def modify():
    global last_modified_time, do_trigger_caching
    do_trigger_caching = True
    last_modified_time = datetime.utcnow()
    print("*", last_modified_time.strftime("%M:%S"))


def emitter():
    print("# emitter()")
    while True:
        time.sleep(random.randint(1, 1800))
        for i in range(random.randint(1, 10)):
            dt = datetime.utcnow()
            print("!", dt.strftime("%M:%S"))
            modify()
            time.sleep(random.randint(1, 3))


def check():
    print("# check()")
    global last_modified_time, do_trigger_caching
    if do_trigger_caching:
        print("                         v", last_modified_time.strftime("%M:%S"))
        do_trigger_caching = False


def checker():
    print("# checker()")
    while True:
        time.sleep(600-10)
        check()


thr1 = threading.Thread(target=emitter)
thr1.start()
thr2 = threading.Thread(target=checker)
thr2.start()

while True:
    time.sleep(5)
