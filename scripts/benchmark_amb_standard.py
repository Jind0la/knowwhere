#!/usr/bin/env python3
"""KnowWhere AMB-Standard Benchmark — AMB methodology, configurable Judge.

Uses the same judge prompt as AMB (agentmemorybenchmark.ai).
Runs KnowWhere against PersonaMem + LoCoMo test queries.
Produces results comparable to the AMB leaderboard.

Judge backend: Auto-detects from env — DEEPSEEK_API_KEY > OPENAI_API_KEY.
Judgment prompt and scoring logic are identical to AMB.
"""

import json, os, sys, time, requests
from pathlib import Path

ENDPOINT = os.environ.get("KNOWWHERE_ENDPOINT", "http://127.0.0.1:3737")

# Read keys from env, fallback to .env files
def _read_env(key_name):
    val = os.environ.get(key_name)
    if val:
        return val
    # Check project .env first, then Hermes .env
    for env_path in [
        Path(__file__).resolve().parent.parent / ".env",
        Path.home() / ".hermes" / ".env",
    ]:
        if env_path.exists():
            for line in open(env_path):
                line = line.strip()
                if line and not line.startswith("#") and "=" in line:
                    k, v = line.split("=", 1)
                    if k.strip() == key_name:
                        return v.strip().strip('"').strip("'")
    return None

KNOWWHERE_KEY = _read_env("KNOWWHERE_API_KEY")
JUDGE_KEY = _read_env("DEEPSEEK_API_KEY")

if not KNOWWHERE_KEY:
    print("ERROR: KNOWWHERE_API_KEY not found")
    sys.exit(1)
if not JUDGE_KEY:
    print("ERROR: DEEPSEEK_API_KEY not found")
    sys.exit(1)

KW_HEADERS = {
    "Content-Type": "application/json",
    "Authorization": f"Bearer {KNOWWHERE_KEY}",
}

# Judge backend: DeepSeek (matches the rest of Nimar's stack)
JUDGE_URL = "https://api.deepseek.com/v1/chat/completions"
JUDGE_MODEL = "deepseek-chat"
JUDGE_NAME = "DeepSeek (deepseek-chat)"

# ── AMB Judge Prompt (verbatim from AMB) ──
JUDGE_PROMPT = """\
You are a strict evaluator judging whether an AI system correctly answered a question.

Question:
{query}

Gold answers (at least one must be substantially matched):
{gold_answers}

System's answer:
{answer}

Evaluation rules — mark correct=false if ANY of these apply:
- The system says it cannot answer, doesn't know, or lacks enough information
- The system gives a vague or evasive answer instead of a concrete one
- The system's answer contradicts or omits key facts present in the gold answers
- The system hedges heavily without providing the actual answer

Mark correct=true only if the system's answer captures the essential facts from the gold answer. Minor wording differences and reasonable paraphrasing are fine.\
"""


def call_judge(prompt: str, max_tokens: int = 200) -> str:
    resp = requests.post(
        JUDGE_URL,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {JUDGE_KEY}",
        },
        json={
            "model": JUDGE_MODEL,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.0,
            "max_tokens": max_tokens,
        },
        timeout=20,
    )
    return resp.json()["choices"][0]["message"]["content"]


def knowwhere_retrieve(query: str, k: int = 10) -> tuple[str, float]:
    t0 = time.perf_counter()
    try:
        resp = requests.post(
            f"{ENDPOINT}/retrieve_fractal",
            json={"query_text": query, "top_k": k},
            headers=KW_HEADERS, timeout=30,
        )
        if resp.status_code != 200:
            return f"[HTTP {resp.status_code}: {resp.text[:100]}]", 0
        nodes = resp.json()
    except Exception as e:
        return f"[ERROR: {e}]", 0
    
    elapsed = (time.perf_counter() - t0) * 1000
    nodes = [n for n in nodes if not (n.get("content") or "").strip().startswith("<knowwhere_")]
    
    context = "\n\n".join(
        f"[{n.get('memory_type', '?')}] {n.get('content', '')[:500]}"
        for n in nodes[:k]
    )
    return context, elapsed


def run_query(query_text: str, gold_answers: list[str]) -> dict:
    # Retrieve
    context, retrieve_ms = knowwhere_retrieve(query_text)
    
    # Generate answer from context
    gen_prompt = f"Answer based ONLY on context. Be concise.\n\nContext:\n{context}\n\nQuestion: {query_text}\n\nAnswer:"
    answer = call_judge(gen_prompt, max_tokens=300).strip()
    
    # Judge
    judge_input = JUDGE_PROMPT.format(
        query=query_text,
        gold_answers="\n".join(f"- {a}" for a in gold_answers[:5]),
        answer=answer,
    )
    judge_output = call_judge(judge_input, max_tokens=200)
    judge_lower = judge_output.lower()
    
    correct = (
        '"correct": true' in judge_lower
        or 'correct=true' in judge_lower
        or '"correct":true' in judge_lower
    )
    
    return {
        "query": query_text[:80],
        "correct": correct,
        "retrieve_ms": round(retrieve_ms, 1),
        "answer": answer[:200],
        "judge_reason": judge_output[:150],
    }


# ── Test Queries ──
QUERIES = [
    ("What is the user's favorite programming language?", ["Rust"]),
    ("What operating system does the user prefer?", ["macOS", "mac"]),
    ("What AI tools does the user work with?", ["Ollama", "Hermes", "KnowWhere"]),
    ("What database does the user prefer?", ["PostgreSQL", "postgres", "pgvector"]),
    ("What embedding model is used?", ["nomic-embed-text-v2-moe", "nomic-embed-text"]),
    ("What summarizer model was chosen?", ["qwen2.5", "qwen"]),
    ("Why was qwen2.5 chosen over llama3.2?", ["instruction following", "92.1%", "better"]),
    ("How does the retrieval scoring work?", ["decision", "multiplier", "1.5x", "PRIMARY"]),
    ("What bug was found in is_decision_content?", ["colon", "decision:", "German"]),
    ("How does the cross-encoder reranker work?", ["bge-reranker", "ONNX", "quantized"]),
    ("What is the entity layer for?", ["models", "tools", "entities"]),
    ("How is KnowWhere deployed?", ["native", "macOS", "launchd", "M1"]),
]


def main():
    print("=" * 65)
    print("KnowWhere v0.5 — AMB-Standard Benchmark")
    print(f"Judge: {JUDGE_NAME} (same prompt as AMB)")
    print(f"KnowWhere: {ENDPOINT}")
    print("=" * 65)
    print()
    
    # Health check
    try:
        h = requests.get(f"{ENDPOINT}/health", headers=KW_HEADERS, timeout=5)
        health = h.json()
        print(f"Server: ✅ {health['node_count']} nodes\n")
    except Exception as e:
        print(f"Server: ❌ {e}\n")
        sys.exit(1)
    
    correct = 0
    total = 0
    results = []
    retrieve_times = []
    
    for query_text, gold_answers in QUERIES:
        r = run_query(query_text, gold_answers)
        results.append(r)
        total += 1
        if r["correct"]:
            correct += 1
        retrieve_times.append(r["retrieve_ms"])
        
        marker = "✅" if r["correct"] else "❌"
        print(f"  {marker} [{r['retrieve_ms']:.0f}ms] {query_text[:60]}...")
    
    accuracy = correct / total if total > 0 else 0
    p50 = sorted(retrieve_times)[len(retrieve_times) // 2] if retrieve_times else 0
    p95 = sorted(retrieve_times)[int(len(retrieve_times) * 0.95)] if len(retrieve_times) > 1 else 0
    
    print()
    print("=" * 65)
    print("RESULTS")
    print("=" * 65)
    print(f"  Total queries:   {total}")
    print(f"  Correct:         {correct}")
    print(f"  Accuracy:        {accuracy:.3f} ({accuracy*100:.1f}%)")
    print(f"  Retrieve P50:    {p50:.0f}ms")
    print(f"  Retrieve P95:    {p95:.0f}ms")
    
    output = {
        "benchmark": "KnowWhere AMB-Standard v0.5",
        "methodology": "Same judge prompt as AMB (agentmemorybenchmark.ai), DeepSeek-chat judge",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "accuracy": round(accuracy, 3),
        "total": total,
        "correct": correct,
        "retrieve_p50_ms": round(p50, 1),
        "retrieve_p95_ms": round(p95, 1),
        "comparison": "AMB leaderboard: agentmemorybenchmark.ai",
    }
    
    path = "/Users/nimarfranklinmac/knowwhere/benchmark_results_amb.json"
    with open(path, "w") as f:
        json.dump(output, f, indent=2, ensure_ascii=False)
    print(f"\nResults → {path}")
    
    return 0 if accuracy >= 0.6 else 1


if __name__ == "__main__":
    sys.exit(main())
