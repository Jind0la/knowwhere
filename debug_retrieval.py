#!/usr/bin/env python3
"""Debug: check why LongMemEval returns 0.0"""
import subprocess, json, urllib.request

result = subprocess.run(['ps', 'eww', '-p', '1376'], capture_output=True, text=True)
key = None
for part in result.stdout.split():
    if part.startswith('KNOWWHERE_API_KEY='):
        key = part.split('=', 1)[1]
        break

if not key:
    print("KEY NOT FOUND")
    exit(1)

print(f"Using key: {key[:15]}...{key[-5:]}")

headers = {
    'Authorization': f'Bearer {key}',
    'Content-Type': 'application/json'
}

# Query with the first case question
payload = json.dumps({
    'query_text': 'What degree did I graduate with?',
    'top_k': 80,
    'max_depth': 3,
    'governance_enabled': True,
    'retrieval_profile': 'full-fidelity',
    'include_debug': False
}).encode()

req = urllib.request.Request('http://127.0.0.1:3738/retrieve_fractal', data=payload, headers=headers)
resp = urllib.request.urlopen(req, timeout=30)
hits = json.loads(resp.read())

# Count session_ids
all_sids = set()
benchmark_types = set()
for h in hits:
    meta = h.get('metadata', {})
    sid = meta.get('session_id', '')
    if sid:
        all_sids.add(sid)
    benchmark_types.add(meta.get('benchmark', 'NONE'))

print(f"\nTotal hits: {len(hits)}")
print(f"Unique session_ids: {len(all_sids)}")
print(f"Benchmark types: {benchmark_types}")

# Check for answer sessions
answer_hits = [h for h in hits if 'answer' in h.get('metadata', {}).get('session_id', '').lower()]
print(f"Hits with 'answer' in session_id: {len(answer_hits)}")
for h in answer_hits[:5]:
    print(f"  {h['metadata']['session_id']}: {h.get('content','')[:100]}")

# Load first case
with open('benchmarks/data/longmemeval_s_cleaned.json') as f:
    data = json.load(f)

case = data[0]
ans_sids = set(case['answer_session_ids'])
print(f"\nExpected answer sessions for case 0: {ans_sids}")
print(f"Found in top-80: {ans_sids & all_sids}")

# Check if any hits match answer_sids
for h in hits:
    if h.get('metadata', {}).get('session_id', '') in ans_sids:
        print(f"\nFOUND ANSWER HIT! sid={h['metadata']['session_id']}")
        print(f"  content[:200]: {h.get('content','')[:200]}")

# Also check: what about the old bench data (benchmark: True)?
bench_true = [h for h in hits if h.get('metadata', {}).get('benchmark') == True]
print(f"\nOld bench data (benchmark=True): {len(bench_true)} hits")
for h in bench_true[:3]:
    print(f"  session_id={h['metadata'].get('session_id')} content[:100]={h.get('content','')[:100]}")

# Check: do we have the answer_280352e9 session at all in the DB?
# Use a very specific query
print("\n\n--- Direct search for answer_280352e9 ---")
payload2 = json.dumps({
    'query_text': 'answer_280352e9',
    'top_k': 100,
    'max_depth': 3,
    'governance_enabled': True,
    'retrieval_profile': 'full-fidelity',
    'include_debug': False
}).encode()
req2 = urllib.request.Request('http://127.0.0.1:3738/retrieve_fractal', data=payload2, headers=headers)
resp2 = urllib.request.urlopen(req2, timeout=30)
hits2 = json.loads(resp2.read())
for h in hits2:
    sid = h.get('metadata', {}).get('session_id', '')
    if 'answer_280352e9' in sid:
        print(f"FOUND! sid={sid}, content[:200]={h.get('content','')[:200]}")
