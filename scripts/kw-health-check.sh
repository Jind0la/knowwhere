#!/usr/bin/env bash
# =============================================================================
# KnowWhere Infrastructure Health Check & Maintenance Script
# =============================================================================
# Checks: KnowWhere server, Ollama, PostgreSQL, binary freshness
# Usage:  ./kw-health-check.sh [quick|full|start|stop|status]
# =============================================================================
set -euo pipefail

# ── Config ───────────────────────────────────────────────────────────────────
# Detect actual user home (ops profile $HOME is not the real home)
if [[ -d "/Users/${USER}" ]]; then
    REAL_HOME="/Users/${USER}"
else
    REAL_HOME="$HOME"
fi
KW_REPO="${KNOWWHERE_REPO:-$REAL_HOME/knowwhere}"
KW_HOST="${KNOWWHERE_HOST:-localhost}"
KW_PORT="${KNOWWHERE_PORT:-3737}"
KW_API_KEY="${KNOWWHERE_API_KEY:-kw_testkey_12345}"
OLLAMA_HOST="${OLLAMA_HOST:-localhost}"
OLLAMA_PORT="${OLLAMA_PORT:-11434}"
PG_HOST="${PG_HOST:-localhost}"
PG_PORT="${PG_PORT:-5432}"
PG_DB="${PG_DB:-knowwhere_dev}"
PG_USER="${PG_USER:-$USER}"
EMBEDDING_PROVIDER="${KNOWWHERE_EMBEDDING_PROVIDER:-ollama}"
OLLAMA_MODEL="${OLLAMA_MODEL:-nomic-embed-text:latest}"
OLLAMA_VLM_MODEL="${OLLAMA_VLM_MODEL:-llama3.2}"
REQUIRED_OLLAMA_MODELS=("nomic-embed-text:latest" "llama3.2")

# ── Colors ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'
BOLD='\033[1m';   NC='\033[0m'
PASS="${GREEN}✓${NC}"; FAIL="${RED}✗${NC}"; WARN="${YELLOW}⚠${NC}"; INFO="${BLUE}ℹ${NC}"

# ── Helpers ──────────────────────────────────────────────────────────────────
section()  { echo -e "\n${BOLD}═══ $1 ═══${NC}"; }
ok()       { echo -e "  ${PASS} $1"; }
fail()     { echo -e "  ${FAIL} $1"; }
warn()     { echo -e "  ${WARN} $1"; }
info()     { echo -e "  ${INFO} $1"; }
stat_ok()  { echo -e "  ${PASS} ${GREEN}$1${NC}"; }

# ── 1. KnowWhere Server Health ───────────────────────────────────────────────
check_kw_server() {
    section "KnowWhere Server (${KW_HOST}:${KW_PORT})"

    # Basic HTTP reachability
    local resp
    resp=$(curl -s -o /dev/null -w "%{http_code}" "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null || echo "000")

    if [[ "$resp" == "200" ]]; then
        local health_json
        health_json=$(curl -s "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null)
        local node_count
        node_count=$(echo "$health_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('node_count','?'))" 2>/dev/null || echo "?")
        stat_ok "HTTP 200 — ${node_count} nodes loaded"
    elif [[ "$resp" == "000" ]]; then
        fail "Server is DOWN (connection refused)"
        return 1
    else
        fail "Server returned HTTP ${resp}"
        return 1
    fi

    # API auth check
    local auth_resp
    auth_resp=$(curl -s -o /dev/null -w "%{http_code}" \
        -H "Authorization: Bearer ${KW_API_KEY}" \
        "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null || echo "000")
    if [[ "$auth_resp" == "200" ]]; then
        ok "API key accepted"
    else
        fail "API key rejected (HTTP ${auth_resp})"
        return 1
    fi

    # Store + retrieve smoke test (POST /store_turn → GET /health node count increases)
    local before_count
    before_count=$(curl -s "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('node_count',-1))" 2>/dev/null || echo "-1")

    local store_resp
    store_resp=$(curl -s -w "\n%{http_code}" \
        -X POST "http://${KW_HOST}:${KW_PORT}/store_session" \
        -H "Authorization: Bearer ${KW_API_KEY}" \
        -H "Content-Type: application/json" \
        -d "{\"content\":\"health-check smoke test $(date +%s)\",\"source_type\":\"human\"}" 2>/dev/null)
    local store_code
    store_code=$(echo "$store_resp" | tail -1)

    if [[ "$store_code" == "200" || "$store_code" == "201" ]]; then
        # Verify node count increased
        local after_count
        after_count=$(curl -s "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('node_count',-1))" 2>/dev/null || echo "-1")
        if [[ "$after_count" -gt "$before_count" ]]; then
            ok "Smoke test: stored + verified (${before_count}→${after_count} nodes)"
        else
            ok "Smoke test stored (HTTP ${store_code})"
        fi
    else
        warn "Smoke test store failed (HTTP ${store_code}) — server may be read-only or missing endpoint"
        info "Response: $(echo "$store_resp" | head -3)"
    fi

    return 0
}

# ── 2. Ollama Health ─────────────────────────────────────────────────────────
check_ollama() {
    section "Ollama (${OLLAMA_HOST}:${OLLAMA_PORT})"

    local resp
    resp=$(curl -s -o /dev/null -w "%{http_code}" "http://${OLLAMA_HOST}:${OLLAMA_PORT}/api/tags" 2>/dev/null || echo "000")

    if [[ "$resp" == "200" ]]; then
        local tags_json
        tags_json=$(curl -s "http://${OLLAMA_HOST}:${OLLAMA_PORT}/api/tags" 2>/dev/null)
        local model_count
        model_count=$(echo "$tags_json" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('models',[])))" 2>/dev/null || echo "?")
        stat_ok "HTTP 200 — ${model_count} models"

        # Check required models
        local models_list
        models_list=$(echo "$tags_json" | python3 -c "
import sys, json
for m in json.load(sys.stdin).get('models',[]):
    print(m['name'])
" 2>/dev/null)

        for required in "${REQUIRED_OLLAMA_MODELS[@]}"; do
            if echo "$models_list" | grep -qF "$required"; then
                ok "Model present: ${required}"
            else
                fail "Model MISSING: ${required} — run: ollama pull ${required}"
            fi
        done
    elif [[ "$resp" == "000" ]]; then
        fail "Ollama is DOWN"
        return 1
    else
        fail "Ollama returned HTTP ${resp}"
        return 1
    fi
}

# ── 3. PostgreSQL Health ─────────────────────────────────────────────────────
check_postgres() {
    section "PostgreSQL (${PG_HOST}:${PG_PORT}/${PG_DB})"

    # Check port reachability
    if python3 -c "
import socket
s = socket.socket()
s.settimeout(2)
try:
    s.connect(('${PG_HOST}', ${PG_PORT}))
    s.close()
    print('OPEN')
except:
    print('CLOSED')
" 2>/dev/null | grep -q "OPEN"; then
        ok "Port ${PG_PORT} reachable"

        # Try connection with psql if available
        local psql_cmd=""
        if command -v psql &>/dev/null; then
            psql_cmd="psql"
        elif [[ -f "/opt/homebrew/opt/libpq/bin/psql" ]]; then
            psql_cmd="/opt/homebrew/opt/libpq/bin/psql"
        elif [[ -f "/usr/local/opt/libpq/bin/psql" ]]; then
            psql_cmd="/usr/local/opt/libpq/bin/psql"
        fi

        if [[ -n "$psql_cmd" ]]; then
            if PGPASSWORD="" "$psql_cmd" -h "${PG_HOST}" -p "${PG_PORT}" -U "${PG_USER}" -d "${PG_DB}" -c "SELECT 1;" &>/dev/null; then
                stat_ok "psql connection OK — db=${PG_DB} (${psql_cmd})"
            else
                warn "psql connection failed (db may not exist or auth issue)"
                info "Expected: postgresql://${PG_USER}@${PG_HOST}:${PG_PORT}/${PG_DB}"
                info "Create DB: createdb ${PG_DB}"
            fi
        else
            info "psql not in PATH — skipping direct connection test"
        fi
    else
        fail "Port ${PG_PORT} is CLOSED"
        return 1
    fi
}

# ── 4. Binary Freshness ──────────────────────────────────────────────────────
check_binary() {
    section "KnowWhere Binary"

    local binary="${KW_REPO}/target/release/knowwhere-server"
    if [[ ! -f "$binary" ]]; then
        fail "Binary not found: ${binary}"
        info "Build: cd ${KW_REPO} && SQLX_OFFLINE=true cargo build --release --features postgres-storage,summarizer,reranker"
        return 1
    fi

    local bin_time
    bin_time=$(stat -f "%Sm" -t "%Y-%m-%d %H:%M" "$binary" 2>/dev/null || stat -c "%y" "$binary" 2>/dev/null | cut -d. -f1)
    local bin_age_sec
    bin_age_sec=$(($(date +%s) - $(stat -f "%m" "$binary" 2>/dev/null || stat -c "%Y" "$binary" 2>/dev/null)))
    local bin_age_hours=$((bin_age_sec / 3600))
    local bin_mb
    bin_mb=$(du -m "$binary" 2>/dev/null | cut -f1)

    stat_ok "Binary exists (${bin_mb}MB, built ${bin_time})"

    if [[ $bin_age_hours -gt 168 ]]; then  # > 1 week
        warn "Binary is ${bin_age_hours}h old — consider rebuilding"
    fi

    # Check binary type (native vs docker)
    local bin_type
    bin_type=$(file "$binary" 2>/dev/null)
    if echo "$bin_type" | grep -q "Mach-O.*arm64"; then
        ok "Native macOS arm64 binary"
    elif echo "$bin_type" | grep -q "Mach-O.*x86_64"; then
        warn "macOS x86_64 binary (Rosetta — slow)"
    elif echo "$bin_type" | grep -q "ELF"; then
        warn "Linux ELF binary — won't run on macOS"
    else
        warn "Unknown binary type: ${bin_type}"
    fi

    # Check source freshness vs binary
    local latest_src
    latest_src=$(find "${KW_REPO}/src" -name "*.rs" -newer "$binary" 2>/dev/null | wc -l | tr -d ' ')
    if [[ "$latest_src" -gt 0 ]]; then
        warn "${latest_src} source files newer than binary — rebuild recommended"
    fi
}

# ── 5. Full Status ───────────────────────────────────────────────────────────
status_all() {
    echo -e "${BOLD}${BLUE}╔══════════════════════════════════════════════════════╗${NC}"
    echo -e "${BOLD}${BLUE}║     KnowWhere Infrastructure Health Report           ║${NC}"
    echo -e "${BOLD}${BLUE}╚══════════════════════════════════════════════════════╝${NC}"
    echo -e "Time: $(date '+%Y-%m-%d %H:%M:%S %Z')"
    echo -e "Host: $(hostname)"

    local failures=0
    check_kw_server    || ((failures++)) || true
    check_ollama       || ((failures++)) || true
    check_postgres     || ((failures++)) || true
    echo ""
    check_binary       || ((failures++)) || true

    echo ""
    echo -e "${BOLD}─── Summary ───${NC}"
    if [[ $failures -eq 0 ]]; then
        echo -e "  ${PASS} All checks passed — KnowWhere is healthy"
    else
        echo -e "  ${FAIL} ${failures} check(s) failed"
    fi

    # Running processes summary
    echo ""
    echo -e "${BOLD}─── Processes ───${NC}"
    local kw_pid
    kw_pid=$(pgrep -f "knowwhere-server" 2>/dev/null | head -1 || echo "none")
    local ollama_pid
    ollama_pid=$(pgrep -f "Ollama" 2>/dev/null | head -1 || echo "none")
    echo "  knowwhere-server PID: ${kw_pid}"
    echo "  Ollama PID:           ${ollama_pid}"

    return $failures
}

# ── 6. Start Server ──────────────────────────────────────────────────────────
start_server() {
    section "Starting KnowWhere Server"

    # Check if already running
    if curl -s -o /dev/null "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null; then
        warn "Server already running on port ${KW_PORT}"
        return 0
    fi

    # Check prerequisites
    if ! curl -s -o /dev/null "http://${OLLAMA_HOST}:${OLLAMA_PORT}/api/tags" 2>/dev/null; then
        fail "Ollama is not running — start it first"
        return 1
    fi

    local binary="${KW_REPO}/target/release/knowwhere-server"
    if [[ ! -f "$binary" ]]; then
        fail "Binary not found — build first"
        return 1
    fi

    info "Starting: ${binary}"
    cd "${KW_REPO}"

    KNOWWHERE_EMBEDDING_PROVIDER="${EMBEDDING_PROVIDER}" \
    OLLAMA_URL="http://${OLLAMA_HOST}:${OLLAMA_PORT}" \
    OLLAMA_MODEL="${OLLAMA_MODEL}" \
    OLLAMA_VLM_MODEL="${OLLAMA_VLM_MODEL}" \
    KNOWWHERE_API_KEY="${KW_API_KEY}" \
    DATABASE_URL="postgresql://${PG_USER}@${PG_HOST}:${PG_PORT}/${PG_DB}" \
    RUST_LOG=info \
    nohup "${binary}" > /tmp/knowwhere-server.log 2>&1 &

    local pid=$!
    echo "  PID: ${pid}"

    # Wait for health check
    info "Waiting for server to be ready..."
    for i in $(seq 1 60); do
        if curl -s -o /dev/null "http://${KW_HOST}:${KW_PORT}/health" 2>/dev/null; then
            ok "Server ready (took ${i}s)"
            return 0
        fi
        sleep 1
    done

    fail "Server did not become ready within 60s"
    echo "Last log lines:"
    tail -20 /tmp/knowwhere-server.log
    return 1
}

# ── 7. Stop Server ───────────────────────────────────────────────────────────
stop_server() {
    section "Stopping KnowWhere Server"

    local pid
    pid=$(pgrep -f "knowwhere-server" 2>/dev/null | head -1 || echo "")

    if [[ -z "$pid" ]]; then
        warn "No knowwhere-server process found"
        return 0
    fi

    info "Sending SIGTERM to PID ${pid}"
    kill "$pid" 2>/dev/null || true

    # Wait for graceful shutdown
    for i in $(seq 1 10); do
        if ! kill -0 "$pid" 2>/dev/null; then
            ok "Server stopped (PID ${pid})"
            return 0
        fi
        sleep 1
    done

    warn "Graceful shutdown timeout — sending SIGKILL"
    kill -9 "$pid" 2>/dev/null || true
    ok "Server force-stopped"
}

# ── 8. Rebuild Binary ────────────────────────────────────────────────────────
rebuild_binary() {
    section "Rebuilding KnowWhere Binary"

    cd "${KW_REPO}"

    # Check source changes
    local changed
    changed=$(find src -name "*.rs" -newer target/release/knowwhere-server 2>/dev/null | wc -l | tr -d ' ')
    if [[ "$changed" -eq 0 ]] && [[ "${FORCE_REBUILD:-0}" != "1" ]]; then
        info "No source changes detected — skipping rebuild (use FORCE_REBUILD=1 to override)"
        return 0
    fi

    info "Building with features: postgres-storage,summarizer,reranker"
    info "Source files changed: ${changed}"

    SQLX_OFFLINE=true cargo build --release --features "postgres-storage,summarizer,reranker" -j 2

    if [[ -f target/release/knowwhere-server ]]; then
        ok "Build successful"
        local size_mb
        size_mb=$(du -m target/release/knowwhere-server | cut -f1)
        stat_ok "Binary: ${size_mb}MB"
    else
        fail "Build failed"
        return 1
    fi
}

# ── Main Dispatch ────────────────────────────────────────────────────────────
case "${1:-status}" in
    quick)
        # Quick check: just health endpoints
        check_kw_server
        check_ollama
        ;;
    full)
        status_all
        ;;
    status)
        status_all
        ;;
    start)
        start_server
        ;;
    stop)
        stop_server
        ;;
    restart)
        stop_server
        sleep 2
        start_server
        ;;
    rebuild)
        rebuild_binary
        ;;
    *)
        echo "Usage: $0 {status|quick|full|start|stop|restart|rebuild}"
        echo ""
        echo "Commands:"
        echo "  status   Full health report (default)"
        echo "  quick    Fast health check (server + Ollama only)"
        echo "  full     Full health report (same as status)"
        echo "  start    Start KnowWhere server"
        echo "  stop     Stop KnowWhere server"
        echo "  restart  Restart KnowWhere server"
        echo "  rebuild  Rebuild binary if sources changed"
        echo ""
        echo "Environment variables:"
        echo "  KNOWWHERE_REPO        Repo path (default: ~/knowwhere)"
        echo "  KNOWWHERE_HOST        Server host (default: localhost)"
        echo "  KNOWWHERE_PORT        Server port (default: 3737)"
        echo "  KNOWWHERE_API_KEY     API key (default: kw_testkey_12345)"
        echo "  OLLAMA_HOST           Ollama host (default: localhost)"
        echo "  OLLAMA_PORT           Ollama port (default: 11434)"
        echo "  PG_HOST/PORT/DB/USER  PostgreSQL config"
        echo "  FORCE_REBUILD=1       Force binary rebuild"
        exit 1
        ;;
esac
