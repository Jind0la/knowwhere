#!/usr/bin/env python3
"""Import KnowWhere in-memory nodes (state.json) into PostgreSQL."""
import json
import sys
from datetime import datetime, timezone

STATE_FILE = "/Users/nimarfranklinmac/knowwhere/native_data/state.json"
BATCH_SIZE = 100

print("-- KnowWhere Memory Import — generated", datetime.now(timezone.utc).isoformat())
print("BEGIN;")
print()

with open(STATE_FILE) as f:
    data = json.load(f)

nodes = data.get("nodes", {})
print(f"-- Total nodes to import: {len(nodes)}", file=sys.stderr)

# Sort: raw first (no FK deps), then overview/summary (may have parent_tier_id)
ordered = sorted(nodes.items(), key=lambda x: {
    'raw': 0, 'summary': 1, 'overview': 2
}.get(x[1].get('context_tier', 'raw'), 99))

inserted = 0
for node_id, node in ordered:
    # Map fields
    memory_type = node.get('memory_type', 'episodic')
    source = node.get('source', 'conversation')
    content = (node.get('content') or '').replace("'", "''")  # SQL escape
    vector = node.get('vector', [])
    metadata = json.dumps(node.get('metadata', {}))
    context_tier = node.get('context_tier', 'raw')
    parent_tier_id = node.get('parent_tier_id') or 'NULL'
    if parent_tier_id != 'NULL':
        parent_tier_id = f"'{parent_tier_id}'"
    confidence = node.get('confidence', 0.8)
    sensitivity = node.get('sensitivity', 'normal')
    importance = node.get('importance', 5)
    status = node.get('status', 'active')
    conflict_state = node.get('conflict_state', 'none')
    superseded_by = node.get('superseded_by') or 'NULL'
    if superseded_by != 'NULL':
        superseded_by = f"'{superseded_by}'"
    access_count = node.get('access_count', 0)
    last_accessed = node.get('last_accessed')
    created_at = node.get('created_at')
    summary_content = (node.get('summary_content') or '').replace("'", "''")
    overview_content = (node.get('overview_content') or '').replace("'", "''")
    original_pointer = (node.get('original_pointer') or '').replace("'", "''")

    if not last_accessed:
        last_accessed = 'NOW()'
    else:
        last_accessed = f"'{last_accessed}'"
    if not created_at:
        created_at = 'NOW()'
    else:
        created_at = f"'{created_at}'"

    # Format vector as pgvector literal
    vec_str = '[' + ','.join(str(v) for v in vector) + ']'

    sql = f"""INSERT INTO memories (
    id, memory_type, source, content, embedding, metadata,
    context_tier, parent_tier_id, confidence, sensitivity,
    importance, status, conflict_state, superseded_by,
    access_count, last_accessed, created_at,
    summary_content, overview_content, original_pointer
) VALUES (
    '{node_id}', '{memory_type}', '{source}', '{content}',
    '{vec_str}'::vector, '{metadata}'::jsonb,
    '{context_tier}', {parent_tier_id}, {confidence}, '{sensitivity}',
    {importance}, '{status}', '{conflict_state}', {superseded_by},
    {access_count}, {last_accessed}, {created_at},
    '{summary_content}', '{overview_content}', '{original_pointer}'
)
ON CONFLICT (id) DO NOTHING;"""

    print(sql)
    inserted += 1

    if inserted % BATCH_SIZE == 0:
        print(f"-- {inserted}/{len(nodes)}", file=sys.stderr)

print()
print("COMMIT;")
print(f"-- Done: {inserted} nodes", file=sys.stderr)
