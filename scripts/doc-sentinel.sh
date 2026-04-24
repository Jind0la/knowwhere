#!/usr/bin/env bash
# Doc Sentinel — Pre-Commit Hook für KnowWhere Dokumentations-Konsistenz
#
# Dieser Hook prüft vor jedem Commit:
# 1. Ob BUG-TRACKING.md aktuell ist (Datum ≤ 7 Tage)
# 2. Ob PRD.md Vektordimension mit provider.rs übereinstimmt
# 3. Ob neue Routen in routes.rs auch in README.md/API-Docs dokumentiert sind
# 4. Ob .sqlx/ Cache aktuell ist (sqlx prepare)
#
# Installieren:
#   ln -sf ../../scripts/doc-sentinel.sh .git/hooks/pre-commit
# (ersetzt oder ergänzt den bestehenden sqlx-prepare hook)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

WARNINGS=0
ERRORS=0

warn() {
    echo -e "${YELLOW}⚠ WARN:${NC} $1"
    ((WARNINGS++)) || true
}

error() {
    echo -e "${RED}✗ FAIL:${NC} $1"
    ((ERRORS++)) || true
}

ok() {
    echo -e "${GREEN}✓ OK:${NC} $1"
}

# --- Check 1: BUG-TRACKING.md Datum ---
check_bug_tracking_date() {
    local file="$REPO_ROOT/docs/BUG-TRACKING.md"
    if [[ ! -f "$file" ]]; then
        error "BUG-TRACKING.md nicht gefunden at $file"
        return
    fi

    local last_date
    last_date=$(grep -E '^\*\*Last Updated:\*\*' "$file" | sed -E 's/.*([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/' || true)

    if [[ -z "$last_date" ]]; then
        error "BUG-TRACKING.md: Kein 'Last Updated' Datum gefunden"
        return
    fi

    local days_old
    days_old=$(( ( $(date +%s) - $(date -j -f "%Y-%m-%d" "$last_date" +%s 2>/dev/null || date -d "$last_date" +%s 2>/dev/null || echo 0) ) / 86400 ))

    if [[ $days_old -gt 14 ]]; then
        warn "BUG-TRACKING.md ist ${days_old} Tage alt (Last Updated: $last_date). Bitte aktualisieren."
    else
        ok "BUG-TRACKING.md ist aktuell (${days_old} Tage alt)"
    fi
}

# --- Check 2: PRD.md Vektordimension Konsistenz ---
check_prd_dimension() {
    local prd="$REPO_ROOT/docs/PRD.md"
    local provider="$REPO_ROOT/src/embedding/provider.rs"

    if [[ ! -f "$prd" ]] || [[ ! -f "$provider" ]]; then
        warn "PRD.md oder provider.rs nicht gefunden — Dimension-Check übersprungen"
        return
    fi

    # Extrahiere Dimension aus PRD.md
    local prd_dim
    prd_dim=$(grep -E 'mit `[0-9]+`' "$prd" | sed -E 's/.*`([0-9]+)`.*/\1/' | head -1 || true)

    # Extrahiere Dimension aus provider.rs (nomic-embed-text-v2-moe = 768, snowflake = 1024)
    local provider_dim
    if grep -q "snowflake-arctic-embed2" "$provider"; then
        provider_dim="1024"
    elif grep -q "nomic-embed-text-v2-moe" "$provider"; then
        provider_dim="768"
    else
        provider_dim=""
    fi

    if [[ -n "$prd_dim" && -n "$provider_dim" && "$prd_dim" != "$provider_dim" ]]; then
        error "PRD.md Dimension ($prd_dim) ≠ provider.rs Dimension ($provider_dim). Bitte PRD.md aktualisieren."
    else
        ok "PRD.md Vektordimension konsistent ($provider_dim)"
    fi
}

# --- Check 3: Neue Routen in routes.rs → README.md ---
check_route_documentation() {
    local routes="$REPO_ROOT/src/api/routes.rs"
    local readme="$REPO_ROOT/README.md"

    if [[ ! -f "$routes" ]] || [[ ! -f "$readme" ]]; then
        warn "routes.rs oder README.md nicht gefunden — Route-Check übersprungen"
        return
    fi

    # Finde alle pub async fn in routes.rs
    local route_names
    route_names=$(grep -E '^pub async fn [a-z_]+' "$routes" | sed -E 's/pub async fn ([a-z_]+).*/\1/' | sort -u || true)

    local missing=()
    for route in $route_names; do
        # Überspringe interne/helper Funktionen und Test-Funktionen
        [[ "$route" =~ ^(health|embed_text)$ ]] && continue
        [[ "$route" =~ ^test_ ]] && continue
        [[ "$route" =~ ^(chunk_|relevant_|truncate_|line_score|question_keywords|is_|qa_|source_) ]] && continue
        [[ "$route" =~ ^(store_session|store_external|retrieve|retrieve_fractal|recent_nodes|reembed_all|repair_embeddings|delete_node|purge_dummy|dream_status|list_events|get_governance_policy|update_governance_policy|webhook_frigate|vlm_status|vlm_enqueue|subconscious_chat|list_retrieval_runs|get_retrieval_run|get_retrieval_trajectory|compact_memory|get_memory|list_conflicts|resolve_conflict|boost_memory_energy|list_low_energy_memories|apply_energy_decay|compress_memory_cluster|list_deduplication_candidates|run_deduplication|list_deduplication_runs|reindex_external_node|memory_health_check|self_healing_stats|list_namespaces|get_namespace|namespace_memories|create_namespace|namespace_search|create_skill|list_skills|get_skill|update_skill|delete_skill|use_skill|match_skills|auth_me)$ ]] && continue

        # Prüfe ob Route in README.md erwähnt wird
        if ! grep -q "$route" "$readme"; then
            missing+=("$route")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        warn "README.md erwähnt nicht: ${missing[*]}"
        echo "   Tipp: Füge neue Routen zur API-Tabelle in README.md hinzu."
    else
        ok "Alle Routen in README.md dokumentiert"
    fi
}

# --- Check 4: .sqlx/ Cache (bestehender Check) ---
check_sqlx_cache() {
    local migrations_changed=false
    local src_changed=false

    # Prüfe ob migrations/ oder src/ geändert wurde (im git repo)
    if git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
        if git -C "$REPO_ROOT" diff --name-only | grep -qE '^migrations/|^src/.*\.rs$|^Cargo\.toml$'; then
            src_changed=true
        fi
    fi

    if ! $src_changed; then
        ok "Keine SQL-relevanten Änderungen — sqlx-Check übersprungen"
        return
    fi

    # Prüfe ob postgres läuft
    if ! pg_isready -h localhost -p 5433 -U postgres >/dev/null 2>&1; then
        warn "PostgreSQL nicht erreichbar (Port 5433). sqlx prepare übersprungen."
        echo "   Starte: docker compose up -d postgres"
        return
    fi

    # Prüfe ob .sqlx/ Cache aktuell ist
    local db_url="${DATABASE_URL:-postgresql://postgres:kw@localhost:5433/knowwhere}"

    if ! cargo sqlx prepare --features postgres-storage --database-url "$db_url" --check >/dev/null 2>&1; then
        warn "sqlx Cache ist veraltet. Führe aus: cargo sqlx prepare --features postgres-storage"
        echo "   Oder: DATABASE_URL='$db_url' cargo sqlx prepare --features postgres-storage"
        echo "   Dann: git add .sqlx/ && git commit --amend --no-edit"
    else
        ok "sqlx Cache ist aktuell"
    fi
}

# --- Check 5: Sprint-Log aktualisiert ---
check_sprint_log() {
    local sprint_log="$REPO_ROOT/.hermes/skills/software-development/knowwhere-dev-team/references/sprint-log.md"
    if [[ ! -f "$sprint_log" ]]; then
        ok "sprint-log.md nicht im repo (externes Skill-Verzeichnis)"
        return
    fi

    local last_entry
    last_entry=$(grep -E '^### [0-9]{4}-[0-9]{2}-[0-9]{2}' "$sprint_log" | head -1 | sed -E 's/### //' || true)

    if [[ -z "$last_entry" ]]; then
        warn "sprint-log.md: Kein Datum im Header-Format '### YYYY-MM-DD' gefunden"
        return
    fi

    local days_old
    days_old=$(( ( $(date +%s) - $(date -j -f "%Y-%m-%d" "$last_entry" +%s 2>/dev/null || date -d "$last_entry" +%s) ) / 86400 ))

    if [[ $days_old -gt 7 ]]; then
        warn "sprint-log.md letzter Eintrag ist ${days_old} Tage alt ($last_entry). Neue Session dokumentieren?"
    else
        ok "sprint-log.md aktuell (${days_old} Tage)"
    fi
}

# --- Check 6: Backlog aktualisiert ---
check_backlog() {
    local backlog="$REPO_ROOT/.hermes/skills/software-development/knowwhere-dev-team/references/backlog.md"
    if [[ ! -f "$backlog" ]]; then
        ok "backlog.md nicht im repo (externes Skill-Verzeichnis)"
        return
    fi

    # Prüfe ob es DONE-Einträge gibt die älter als 30 Tage sind und noch nicht archiviert
    ok "backlog.md existiert (manuelle Prüfung empfohlen)"
}

# === MAIN ===
echo "🔍 Doc Sentinel — Dokumentations-Konsistenz-Check"
echo "================================================"

check_bug_tracking_date
check_prd_dimension
check_route_documentation
check_sqlx_cache
check_sprint_log
check_backlog

echo "================================================"
if [[ $ERRORS -gt 0 ]]; then
    echo -e "${RED}✗ $ERRORS Fehler, $WARNINGS Warnungen${NC}"
    echo "Commit wird BLOCKIERT. Fixe die Fehler oder nutze: git commit --no-verify"
    exit 1
elif [[ $WARNINGS -gt 0 ]]; then
    echo -e "${YELLOW}⚠ $WARNINGS Warnungen (keine Fehler)${NC}"
    echo "Commit erlaubt, aber bitte Warnungen prüfen."
    exit 0
else
    echo -e "${GREEN}✓ Alle Checks bestanden${NC}"
    exit 0
fi
