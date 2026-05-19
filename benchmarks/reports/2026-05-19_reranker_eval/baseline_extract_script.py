#!/usr/bin/env python3
"""
Extract per-query and aggregate NDCG@5, NDCG@10, Recall@5, MRR
from the reranker evaluation run (t_a21da4b4).

Concrete plan:
  1. Load test_queries.json (the 25-case dataset used in the eval).
  2. Run dense_proxy_baseline (deterministic, no server needed).
  3. If the correct ONNX MiniLM model is found, rerank with it.
     Otherwise, fall back to the dense_proxy results and note the gap.
  4. For each query, extract the four target metrics.
  5. Print per-query table + aggregate summary.
  6. Save structured JSON to workspace.
"""

import json
import sys
import os
import time
from pathlib import Path
from typing import List, Dict, Any

import numpy as np

# ── Path setup: pull in the eval framework from t_8b5a6703 ──
EVAL_FRAMEWORK = Path(
    "/Users/nimarfranklinmac/.hermes/kanban/workspaces/t_8b5a6703"
)
sys.path.insert(0, str(EVAL_FRAMEWORK))

from rerank_eval.dataset import load_dataset, dataset_stats
from rerank_eval.evaluation import evaluate_single_case, K_VALUES
from rerank_eval.baseline import dense_proxy_baseline


def run_dense_proxy(cases):
    """Phase 1: deterministic dense_proxy baseline."""
    results = []
    for case in cases:
        hit_ids = dense_proxy_baseline(case)
        result = evaluate_single_case(hit_ids, case, K_VALUES)
        results.append(result)
    return results


def attempt_onnx_rerank(cases, baseline_results, top_k=20):
    """Phase 2: cross-encoder reranking. Returns None if ONNX unavailable."""
    try:
        import onnxruntime as ort
        from tokenizers import Tokenizer  # noqa: F811
    except ImportError:
        print("onnxruntime or tokenizers not installed — skipping ONNX rerank")
        return None

    model_path = "/Users/nimarfranklinmac/.cache/knowwhere/reranker/model.onnx"
    tok_path = "/Users/nimarfranklinmac/.cache/knowwhere/reranker/tokenizer.json"

    if not os.path.exists(model_path) or not os.path.exists(tok_path):
        print(f"ONNX model or tokenizer not found, skipping")
        return None

    print(f"Loading ONNX model: {model_path} ({os.path.getsize(model_path)/1e6:.0f} MB)")
    session = ort.InferenceSession(
        model_path,
        providers=["CPUExecutionProvider"],
        sess_options=ort.SessionOptions(),
    )
    tokenizer = Tokenizer.from_file(tok_path)
    print(f"  Inputs: {[i.name for i in session.get_inputs()]}")
    print(f"  Outputs: {[o.name for o in session.get_outputs()]}")

    # Helper: sigmoid
    def sigmoid(x):
        return 1.0 / (1.0 + np.exp(-x))

    reranker_results = []
    reranker_timings = []

    for idx, case in enumerate(cases):
        baseline_ids = dense_proxy_baseline(case)

        # Build candidate texts from top-k
        candidate_texts = []
        candidate_ids = []
        for sid in baseline_ids[:top_k]:
            if sid in case.haystack_session_ids:
                sid_idx = case.haystack_session_ids.index(sid)
                candidate_texts.append(case.session_text(sid_idx))
                candidate_ids.append(sid)

        if not candidate_ids:
            # Edge case: no candidates
            result = evaluate_single_case(baseline_ids, case, K_VALUES)
            result["reranker_timing_ms"] = 0.0
            reranker_results.append(result)
            continue

        # Batch inference
        t0 = time.monotonic()
        batch_size = 32
        scores = []
        for b in range(0, len(candidate_texts), batch_size):
            batch = candidate_texts[b:b + batch_size]
            encodings = [tokenizer.encode(f"{case.question} [SEP] {doc}") for doc in batch]
            max_len = min(max(len(e.ids) for e in encodings), 512)
            bs = len(batch)

            input_ids = np.zeros((bs, max_len), dtype=np.int64)
            attention_mask = np.zeros((bs, max_len), dtype=np.int64)
            token_type_ids = np.zeros((bs, max_len), dtype=np.int64)

            for bi, enc in enumerate(encodings):
                ids = enc.ids[:max_len]
                mask = enc.attention_mask[:max_len]
                types = enc.type_ids[:max_len] if enc.type_ids else [0] * len(ids)
                input_ids[bi, :len(ids)] = ids
                attention_mask[bi, :len(mask)] = mask
                token_type_ids[bi, :len(types)] = types

            inputs = {
                "input_ids": input_ids,
                "attention_mask": attention_mask,
            }
            # Only add token_type_ids if the model expects it
            input_names = [i.name for i in session.get_inputs()]
            if "token_type_ids" in input_names:
                inputs["token_type_ids"] = token_type_ids
            outputs = session.run(None, inputs)
            logits = outputs[0]
            batch_scores = sigmoid(logits.flatten() if logits.ndim > 1 else logits)
            scores.extend(batch_scores.tolist())

        elapsed_ms = (time.monotonic() - t0) * 1000
        reranker_timings.append(elapsed_ms)

        # Sort by cross-encoder score
        scored = list(zip(candidate_ids, scores))
        scored.sort(key=lambda x: x[1], reverse=True)
        reranked_ids = [sid for sid, _ in scored]

        # Append remaining baseline IDs
        for sid in baseline_ids[top_k:]:
            if sid not in reranked_ids:
                reranked_ids.append(sid)

        result = evaluate_single_case(reranked_ids, case, K_VALUES)
        result["reranker_timing_ms"] = elapsed_ms
        reranker_results.append(result)

        if (idx + 1) % 10 == 0 or idx == len(cases) - 1:
            avg_t = np.mean(reranker_timings) if reranker_timings else 0
            print(f"  [{idx+1}/{len(cases)}] reranked (avg {avg_t:.0f}ms/case)", flush=True)

    avg_ms = float(np.mean(reranker_timings)) if reranker_timings else 0
    print(f"  Done. Mean: {avg_ms:.1f} ms/case")
    return reranker_results


def extract_metrics(result: dict) -> dict:
    """Pull NDCG@5, NDCG@10, Recall@5, MRR from a single-case result."""
    session = result.get("session", {})
    new_metrics = session.get("new", {})

    m5 = new_metrics.get(5, {})
    m10 = new_metrics.get(10, {})

    return {
        "question_id": result.get("question_id", "?"),
        "question_type": result.get("question_type_norm", result.get("question_type", "?")),
        "ndcg_at_5": m5.get("ndcg_any", 0.0),
        "ndcg_at_10": m10.get("ndcg_any", 0.0),
        "recall_at_5": m5.get("recall_any", 0.0),
        "mrr": session.get("mrr", 0.0),
    }


def compute_aggregate(per_query: List[dict]) -> dict:
    """Mean of per-query metrics across all non-abstention cases."""
    n = max(len(per_query), 1)
    return {
        "ndcg_at_5": sum(q["ndcg_at_5"] for q in per_query) / n,
        "ndcg_at_10": sum(q["ndcg_at_10"] for q in per_query) / n,
        "recall_at_5": sum(q["recall_at_5"] for q in per_query) / n,
        "mrr": sum(q["mrr"] for q in per_query) / n,
        "num_queries": len(per_query),
    }


def format_table(per_query: List[dict], aggregate: dict, mode: str) -> str:
    """ASCII table with per-query rows + aggregate footer."""
    header = f"{'#':>3}  {'Query ID':<24} {'Type':<24} {'NDCG@5':>8} {'NDCG@10':>8} {'Recall@5':>9} {'MRR':>8}"
    sep = "-" * len(header)
    lines = [f"\n{mode} — Per-Query Metrics", sep, header, sep]

    for i, q in enumerate(per_query, 1):
        lines.append(
            f"{i:>3}  {q['question_id']:<24} {q['question_type']:<24} "
            f"{q['ndcg_at_5']:>8.4f} {q['ndcg_at_10']:>8.4f} "
            f"{q['recall_at_5']:>9.4f} {q['mrr']:>8.4f}"
        )

    lines.append(sep)
    lines.append(
        f"{'AGG':>3}  {'':24} {'':24} "
        f"{aggregate['ndcg_at_5']:>8.4f} {aggregate['ndcg_at_10']:>8.4f} "
        f"{aggregate['recall_at_5']:>9.4f} {aggregate['mrr']:>8.4f}"
    )
    lines.append(sep)
    return "\n".join(lines)


def main():
    dataset_path = os.path.join(
        EVAL_FRAMEWORK, "fixtures", "test_queries.json"
    )
    print(f"Loading dataset: {dataset_path}")
    cases = load_dataset(dataset_path)
    stats = dataset_stats(cases)
    print(f"  {stats['total']} cases ({stats['non_abstention']} eval, {stats['abstention']} abstention)")

    # ── Phase 1: Dense proxy baseline ──
    print("\n── Phase 1: Dense Proxy Baseline ──")
    baseline_results = run_dense_proxy(cases)

    # Exclude abstention cases
    non_abs = [r for r in baseline_results if not r.get("is_abstention", False)]

    baseline_per_query = [extract_metrics(r) for r in non_abs]
    baseline_agg = compute_aggregate(baseline_per_query)

    print(format_table(baseline_per_query, baseline_agg, "Dense Proxy Baseline"))

    # ── Phase 2: ONNX Reranker (if available) ──
    print("\n── Phase 2: ONNX Reranker ──")
    reranker_results = attempt_onnx_rerank(cases, baseline_results)

    if reranker_results:
        non_abs_rr = [r for r in reranker_results if not r.get("is_abstention", False)]
        reranker_per_query = [extract_metrics(r) for r in non_abs_rr]
        reranker_agg = compute_aggregate(reranker_per_query)
        print(format_table(reranker_per_query, reranker_agg, "Reranker-Augmented (ONNX)"))
    else:
        reranker_per_query = None
        reranker_agg = None
        print("  Skipped — ONNX model not available or dependencies missing")

    # ── Save structured output ──
    output = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "task": "t_9885be77",
        "source_task": "t_a21da4b4",
        "dataset_stats": stats,
        "baseline": {
            "per_query": baseline_per_query,
            "aggregate": baseline_agg,
        },
    }
    if reranker_per_query:
        output["reranker"] = {
            "per_query": reranker_per_query,
            "aggregate": reranker_agg,
        }

    out_path = "per_query_metrics.json"
    with open(out_path, "w") as f:
        json.dump(output, f, indent=2)
    print(f"\n✓ Saved to {out_path}")

    # ── Comparison against stored report ──
    report_path = str(
        Path(EVAL_FRAMEWORK).parent
        / "t_a21da4b4"
        / "reranker_eval_report_20260519_072705.json"
    )
    if os.path.exists(report_path):
        with open(report_path) as f:
            stored = json.load(f)

        print("\n── Cross-check against stored aggregate report ──")
        stored_ndcg5 = stored.get("deltas", {}).get("ndcg_at_5", {})
        stored_ndcg10 = stored.get("deltas", {}).get("ndcg_at_10", {})
        stored_recall5 = stored.get("deltas", {}).get("recall_at_5", {})
        stored_mrr = stored.get("deltas", {}).get("mrr", {})

        print(f"  Stored baseline NDCG@5:  {stored_ndcg5.get('baseline', '?'):.4f}")
        print(f"  Our     baseline NDCG@5:  {baseline_agg['ndcg_at_5']:.4f}")
        print(f"  Stored baseline NDCG@10: {stored_ndcg10.get('baseline', '?'):.4f}")
        print(f"  Our     baseline NDCG@10: {baseline_agg['ndcg_at_10']:.4f}")
        print(f"  Stored baseline MRR:     {stored_mrr.get('baseline', '?'):.4f}")
        print(f"  Our     baseline MRR:    {baseline_agg['mrr']:.4f}")

        if reranker_agg:
            print(f"  Stored reranker NDCG@5:  {stored_ndcg5.get('reranker', '?'):.4f}")
            print(f"  Our     reranker NDCG@5:  {reranker_agg['ndcg_at_5']:.4f}")
            print(f"  Stored reranker NDCG@10: {stored_ndcg10.get('reranker', '?'):.4f}")
            print(f"  Our     reranker NDCG@10: {reranker_agg['ndcg_at_10']:.4f}")
            print(f"  Stored reranker MRR:     {stored_mrr.get('reranker', '?'):.4f}")
            print(f"  Our     reranker MRR:    {reranker_agg['mrr']:.4f}")

            # Check if our reranker matches the stored report
            match_ndcg5 = abs(reranker_agg['ndcg_at_5'] - stored_ndcg5.get('reranker', 0)) < 0.001
            match_ndcg10 = abs(reranker_agg['ndcg_at_10'] - stored_ndcg10.get('reranker', 0)) < 0.001
            match_mrr = abs(reranker_agg['mrr'] - stored_mrr.get('reranker', 0)) < 0.001
            if match_ndcg5 and match_ndcg10 and match_mrr:
                print("\n  ✓ Reranker results MATCH stored report — ONNX model is consistent")
            else:
                print("\n  ⚠ Reranker results DIFFER from stored report — different ONNX model?")

    return 0


if __name__ == "__main__":
    sys.exit(main())
