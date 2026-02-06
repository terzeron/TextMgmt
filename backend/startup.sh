#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# 가상환경 활성화
source "$PROJECT_ROOT/.venv/bin/activate"

# .env 파일 로드
if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    source "$PROJECT_ROOT/.env"
    set +a
fi

cd "$SCRIPT_DIR"
pwd

pidfile="uvicorn.pid"
if [ -f "$pidfile" ]; then
    echo "Killing old process..."
    cat "$pidfile"
    kill $(cat $pidfile)
    sleep 2
fi

rm -f nohup.out
echo "Starting service..."
nohup uvicorn main:app --workers=1 --reload --port=$TM_BACKEND_PORT &
echo "$!" > "$pidfile"
sleep 2
cat "$pidfile"
tail -f nohup.out
