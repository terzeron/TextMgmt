#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

if [ -f "$PROJECT_ROOT/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$PROJECT_ROOT/.env"
    set +a
fi

cd "$PROJECT_ROOT"
port="${TM_BACKEND_PORT:-8020}"
pidfile="$SCRIPT_DIR/backend.pid"
logfile="$SCRIPT_DIR/backend.log"

cargo build --release --locked -p backend -p loader || exit $?
export TM_LOADER_BIN="${TM_LOADER_BIN:-$PROJECT_ROOT/target/release/loader}"

if [ -f "$pidfile" ]; then
    old_pid="$(cat "$pidfile")"
    if kill -0 "$old_pid" 2>/dev/null; then
        echo "Stopping backend process $old_pid"
        kill "$old_pid"
        for _ in {1..20}; do
            kill -0 "$old_pid" 2>/dev/null || break
            sleep 0.25
        done
    fi
    rm -f "$pidfile"
fi

echo "Starting Rust backend on port $port"
nohup "$PROJECT_ROOT/target/release/backend" serve --addr "0.0.0.0:$port" >>"$logfile" 2>&1 &
echo "$!" > "$pidfile"
sleep 2
cat "$pidfile"
tail -f "$logfile"
