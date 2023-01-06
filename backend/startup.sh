#!/bin/bash

pidfile="uvicorn.pid"
if [ -f "$pidfile" ]; then
    echo "Killing old process..."
    kill $(cat $pidfile)
    sleep 3
fi

echo "Starting service..."
daemon --pidfile=$(pwd)/uvicorn.pid -- uvicorn main:app --workers=2
echo $! > uvicorn.pid
sleep 2
