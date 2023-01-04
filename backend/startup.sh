#!/bin/bash

pidfile="uvicorn.pid"
if [ -f "$pidfile" ]; then
    echo "Killing old process"
    kill -9 $(cat $pidfile)
fi

nohup uvicorn main:app --workers=2 &
echo $! > uvicorn.pid
