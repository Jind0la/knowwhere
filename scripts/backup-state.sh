#!/usr/bin/env bash
# KnowWhere State Backup
# Sichert data/state.json nach data/backups/ mit Datumsstempel
# Rotation: behält die letzten 30 täglichen Backups
set -euo pipefail

DATA_DIR="${DATA_DIR:-$HOME/knowwhere/data}"
BACKUP_DIR="${BACKUP_DIR:-$DATA_DIR/backups}"
STATE_FILE="$DATA_DIR/state.json"

mkdir -p "$BACKUP_DIR"

if [ ! -f "$STATE_FILE" ]; then
    echo "ERROR: state.json not found at $STATE_FILE"
    exit 1
fi

TIMESTAMP=$(date +%Y-%m-%d)
BACKUP_FILE="$BACKUP_DIR/state_${TIMESTAMP}.json"

# Nur backupen wenn sich die Datei geändert hat (via md5)
if [ -f "$BACKUP_FILE" ]; then
    CURRENT_MD5=$(md5 -q "$STATE_FILE" 2>/dev/null || md5sum "$STATE_FILE" | cut -d' ' -f1)
    BACKUP_MD5=$(md5 -q "$BACKUP_FILE" 2>/dev/null || md5sum "$BACKUP_FILE" | cut -d' ' -f1)
    if [ "$CURRENT_MD5" = "$BACKUP_MD5" ]; then
        echo "state.json unchanged — skipping backup"
        exit 0
    fi
fi

cp "$STATE_FILE" "$BACKUP_FILE"
echo "backup: $BACKUP_FILE ($(wc -c < "$BACKUP_FILE" | tr -d ' ') bytes)"

# Rotation: nur die letzten 30 state_*-Backups behalten
ls -1t "$BACKUP_DIR"/state_????-??-??.json 2>/dev/null | tail -n +31 | xargs rm -f 2>/dev/null || true
