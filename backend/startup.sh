#!/bin/bash

cd "$(dirname "$0")/"
pwd

pidfile="uvicorn.pid"
if [ -f "$pidfile" ]; then
    echo "Killing old process..."
    kill $(cat $pidfile)
    sleep 3
fi

rm -f nohup.out
echo "Starting service..."
nohup uvicorn backend.main:app --workers=4 &
echo "$!" > "$pidfile"
sleep 1
tail -f nohup.out
