#!/bin/bash

cd "$(dirname "$0")"
pwd

pidfile="uvicorn.pid"
if [ -f "$pidfile" ]; then
    echo "Killing old process..."
    kill $(cat $pidfile)
    sleep 2
fi

rm -f nohup.out
echo "Starting service..."
nohup uvicorn main:app --workers=1 &
echo "$!" > "$pidfile"
sleep 2
tail -f nohup.out
