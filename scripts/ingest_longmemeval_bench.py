#!/usr/bin/env python3
"""
Strong temporal spread ingestion for benchmark.

Creates sessions 4-6 weeks apart with explicit timestamps.
"""

import json
import requests
from datetime import datetime, timedelta
from pathlib import Path

SERVER = "http://localhost:3738"
KEY = "kw_bench_key_12345"
HEADERS = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {KEY}"
}

DATA_FILE = Path("benchmarks/data/longmemeval_s_cleaned.json")
NUM_SESSIONS = 5
Q_PER_SESSION = 6


def load():
    with open(DATA_FILE) as f:
        return json.load(f)


def build_sessions(data):
    sessions = []
    base = datetime(2025, 4, 1)   # Start early enough
    
    relevant = [d for d in data if d.get("question_type") == "single-session-user"][:NUM_SESSIONS * Q_PER_SESSION]
    
    for i in range(NUM_SESSIONS):
        # 4-6 weeks between sessions
        session_start = base + timedelta(days=i * 35)
        sid = f"bench_sess_{i+1:02d}"
        
        sessions.append({
            "session_id": sid,
            "start_date": session_start,
            "items": relevant[i*Q_PER_SESSION : (i+1)*Q_PER_SESSION]
        })
    return sessions


def ingest(session):
    sid = session["session_id"]
    start = session["start_date"]
    success = 0
    failed = 0
    
    for idx, item in enumerate(session["items"]):
        ts = start + timedelta(days=idx * 2)   # spread within session
        
        content = f"Q: {item['question']}\nA: {item.get('answer', '')}"
        
        payload = {
            "content": content,
            "source": "benchmark",
            "source_id": f"{sid}_{idx}",
            "metadata": {
                "session_id": sid,
                "benchmark": True
            },
            "memory_type": "episodic",
            "pointer": "benchmark_longmemeval",
            "created_at": ts.isoformat() + "Z"
        }
        
        try:
            r = requests.post(f"{SERVER}/store_external", json=payload, headers=HEADERS, timeout=15)
            if r.status_code in (200, 201):
                status = "✓"
                success += 1
            else:
                status = f"✗{r.status_code}"
                failed += 1
                print(f"    ERROR: {r.text[:300]}")
        except Exception as e:
            status = "✗EXC"
            failed += 1
            print(f"    EXCEPTION: {e}")
        
        print(f"  {status} {sid} #{idx+1} ({ts.date()})")
    
    return success, failed

def main():
    print("Loading data...")
    data = load()
    
    print(f"Building {NUM_SESSIONS} sessions with 5-week gaps...")
    sessions = build_sessions(data)
    
    print("\nIngesting...\n")
    for s in sessions:
        print(f"Session {s['session_id']} starting {s['start_date'].date()}")
        ingest(s)
        print()
    
    print("Done.")


if __name__ == "__main__":
    main()