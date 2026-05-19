# LongMemEval Evaluation Script

Cross-session evaluation for KnowWhere's memory system against the LongMemEval benchmark (ICLR 2025).

## What It Evaluates

**6 Question Types** with per-type breakdown:
- `single-session-user` — recall user-stated facts
- `single-session-assistant` — recall assistant-provided info
- `single-session-preference` — personalize from implicit preferences
- `multi-session` — synthesize across 2+ sessions
- `temporal-reasoning` — order events chronologically
- `knowledge-update` — prioritize newer over older info
- `abstention` — recognize unanswerable questions

**Two Metric Sets (old + new for comparison):**

| Metric Set | What |
|---|---|
| **Old** | top1, recall@5, recall@k, MRR (session-level) |
| **New** | recall_any@k, recall_all@k, ndcg_any@k at k=[1,3,5,10,30,50] (session + turn-level) |

## Quick Start

```bash
# Per-case mode (store → retrieve → score → cleanup per question)
python longmemeval_eval.py \
  --dataset benchmarks/data/longmemeval_s_cleaned.json \
  --mode percase \
  --api-key "$KNOWWHERE_API_KEY" \
  --max-cases 50

# Multi-session mode (index all sessions once, query all questions)
python longmemeval_eval.py \
  --dataset benchmarks/data/longmemeval_s_cleaned.json \
  --mode multi \
  --api-key "$KNOWWHERE_API_KEY" \
  --max-cases 50
```

## Evaluation Modes

### `percase` — Isolated (backward-compatible)
Each case is evaluated independently: store its sessions, retrieve, score, delete. 
Same semantics as the existing Rust `longmemeval_retrieval_eval` binary.

### `multi` — Genuine cross-session (recommended)
All sessions from all non-abstention cases are indexed ONCE into a shared haystack.
Then all questions are queried against the full corpus. This is how LongMemEval 
was designed — the retrieval system must find evidence among ALL sessions, not
just one case's sessions.

## Requirements

```
pip install aiohttp
```

KnowWhere API must be running at the configured base URL (default `http://127.0.0.1:3737`).

## Output

Human-readable terminal report with:
- Old metrics (top1, recall@5, recall@k, MRR)
- New metrics (session-level + turn-level recall/NDCG at 6 k-values)
- Per-type breakdown (each question type scored separately)
- Abstention accuracy
- Old-vs-new comparison table

Plus a JSON report file with full per-case details.

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `KNOWWHERE_API_KEY` | (required) | API key for KnowWhere |
| `KNOWWHERE_BASE_URL` | `http://127.0.0.1:3737` | KnowWhere API URL |
| `KNOWWHERE_TOP_K` | 20 | k for old-style recall@k |
| `KNOWWHERE_MAX_CASES` | 0 (all) | Limit number of cases |

## Dataset Format

Supports two formats:

**LongMemEval original** (with `has_answer` on turns):
```json
[{
  "question_id": "...",
  "question_type": "single-session-user",
  "question": "...",
  "answer": "...",
  "answer_session_ids": ["answer_abc123"],
  "haystack_session_ids": ["s0", "s1", "answer_abc123"],
  "haystack_dates": ["2023/05/20 ...", ...],
  "haystack_sessions": [
    [{"role": "user", "content": "...", "has_answer": false}, ...],
    ...
  ]
}]
```

**KnowWhere simplified** (session-level `answer_session_ids` only):
```json
[{
  "question_id": "...",
  "answer_session_ids": ["answer_abc123"],
  "haystack_session_ids": ["s0", "s1", "answer_abc123"],
  "haystack_sessions": [
    [{"role": "user", "content": "..."}, {"role": "assistant", "content": "..."}],
    ...
  ]
}]
```

The script auto-detects format. Turn-level metrics are only available with `has_answer` labels.

## Limitations

- **Turn-level granularity**: KnowWhere stores entire sessions as single nodes. Turn-level 
  evaluation maps session hits to all constituent turns, which is an approximation. 
  True turn-level evaluation requires storing individual turns (the paper's "round-level"
  granularity).
- **Embedding cost**: Each session requires an Ollama embedding call. 500 cases × 50 sessions 
  = 25,000 embeddings in multi-session mode. ~8-10 hours for a full run.
