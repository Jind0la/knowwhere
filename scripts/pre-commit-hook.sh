#!/bin/sh
# Pre-commit hook for KnowWhere
# Ensures sqlx offline query cache is up-to-date before committing

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "🔍 Checking sqlx offline query cache..."

# Check if there are any sqlx-related changes
if git diff --cached --name-only | grep -qE "(migrations/|src/.*\.rs|Cargo\.toml)"; then
    echo "📦 SQL-related changes detected, running cargo sqlx prepare..."
    
    # Check if DATABASE_URL is set
    if [ -z "$DATABASE_URL" ]; then
        echo "${YELLOW}⚠️  DATABASE_URL not set, trying local postgres...${NC}"
        export DATABASE_URL="postgresql://postgres:kw@localhost:5433/knowwhere"
    fi
    
    # Check if postgres is running
    if ! pg_isready -h localhost -p 5433 -U postgres > /dev/null 2>&1; then
        echo "${RED}❌ PostgreSQL not running on port 5433${NC}"
        echo "   Start it with: docker start knowwhere-kw-postgres-1"
        echo "   Or skip this check with: git commit --no-verify"
        exit 1
    fi
    
    # Run cargo sqlx prepare
    if cargo sqlx prepare -- --features postgres-storage > /dev/null 2>&1; then
        echo "${GREEN}✅ sqlx query cache updated${NC}"
        
        # Check if .sqlx/ files changed
        if git diff --name-only | grep -q "\.sqlx/"; then
            echo "📁 Adding updated .sqlx/ files to commit..."
            git add .sqlx/
            echo "${GREEN}✅ .sqlx/ files staged${NC}"
        fi
    else
        echo "${RED}❌ cargo sqlx prepare failed${NC}"
        echo "   Fix the errors above, then commit again."
        echo "   Or skip this check with: git commit --no-verify"
        exit 1
    fi
else
    echo "${GREEN}✅ No SQL-related changes, skipping sqlx check${NC}"
fi

# Check if .sqlx/ is in .gitignore (it shouldn't be)
if [ -f ".gitignore" ] && grep -q "^\.sqlx/" .gitignore; then
    echo "${RED}❌ .sqlx/ is in .gitignore — remove it!${NC}"
    echo "   .sqlx/ must be tracked for offline builds."
    exit 1
fi

echo "${GREEN}✅ Pre-commit checks passed${NC}"

# --- Doc Sentinel: Dokumentations-Konsistenz ---
echo ""
echo "🔍 Running Doc Sentinel..."
if [ -f "scripts/doc-sentinel.sh" ]; then
    bash scripts/doc-sentinel.sh || exit $?
else
    echo "${YELLOW}⚠️  doc-sentinel.sh not found, skipping doc checks${NC}"
fi

exit 0
