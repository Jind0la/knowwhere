#!/usr/bin/env python3
"""
Reranker-Augmented Evaluation for KnowWhere.

Runs cross-encoder reranking on top of baseline rankings using the
exported ONNX model. Compares reranker-augmented metrics against
pure bi-encoder (dense_proxy) baselines.

Uses the same test_queries.json and evaluation framework as prior baselines.
"""

import json
import sys
import os
import time
from pathlib import Path
from typing import List, Dict, Any, Optional, Tuple

import numpy as np

# ── Path setup ──
EVAL_DIR = Path(os.environ.get(
    "RERANK_EVAL_DIR",
    "/Users/nimarfranklinmac/.hermes/kanban/workspaces/t_8b5a6703/rerank_eval"
))
sys.path.insert(0, str(EVAL_DIR.parent))
sys.path.insert(0, str(EVAL_DIR))

from rerank_eval.dataset import load_dataset, dataset_stats
from rerank_eval.evaluation import evaluate_single_case, K_VALUES
from rerank_eval.harness import aggregate_results, format_summary
from rerank_eval.baseline import dense_proxy_baseline

# ── ONNX Reranker ──
try:
    import onnxruntime as ort
except ImportError:
    print("ERROR: onnxruntime not installed. Run: pip3 install onnxruntime")
    sys.exit(1)

try:
    from tokenizers import Tokenizer
except ImportError:
    print("ERROR: tokenizers not installed. Run: pip3 install tokenizers")
    sys.exit(1)


def sigmoid(x: np.ndarray) -> np.ndarray:
    return 1.0 / (1.0 + np.exp(-x))


def load_reranker(
    model_path: str,
    tokenizer_path: str,
    max_length: int = 512,
) -> Tuple[ort.InferenceSession, Tokenizer, List[str]]:
    """Load ONNX model and tokenizer. Returns (session, tokenizer, input_names)."""
    print(f"Loading ONNX model: {model_path}")
    t0 = time.monotonic()
    session = ort.InferenceSession(
        model_path,
        providers=["CPUExecutionProvider"],
        sess_options=ort.SessionOptions(),
    )
    tokenizer = Tokenizer.from_file(tokenizer_path)
    # Disable auto-padding — we handle it ourselves
    tokenizer.no_padding()
    input_names = [i.name for i in session.get_inputs()]
    print(f"  Loaded in {time.monotonic() - t0:.1f}s")
    print(f"  Inputs: {input_names}")
    print(f"  Outputs: {[o.name for o in session.get_outputs()]}")
    return session, tokenizer, input_names


def rerank_candidates(
    session: ort.InferenceSession,
    tokenizer: Tokenizer,
    query: str,
    documents: List[str],
    batch_size: int = 32,
    max_length: int = 512,
    input_names: List[str] = None,
) -> List[float]:
    """Score (query, doc) pairs with cross-encoder and return relevance scores."""
    scores = []
    has_token_type_ids = input_names and "token_type_ids" in input_names
    for i in range(0, len(documents), batch_size):
        batch = documents[i : i + batch_size]
        # Tokenize: query [SEP] document (MiniLM/GTE format)
        encodings = [
            tokenizer.encode(f"{query} [SEP] {doc}") for doc in batch
        ]

        # Pad to max length in batch
        max_len = min(max(len(e.ids) for e in encodings), max_length)
        bs = len(batch)

        input_ids = np.zeros((bs, max_len), dtype=np.int64)
        attention_mask = np.zeros((bs, max_len), dtype=np.int64)

        for b, enc in enumerate(encodings):
            ids = enc.ids[:max_len]
            mask = enc.attention_mask[:max_len]
            input_ids[b, :len(ids)] = ids
            attention_mask[b, :len(mask)] = mask

        # Build inputs based on what the model expects
        ort_inputs = {
            "input_ids": input_ids,
            "attention_mask": attention_mask,
        }
        if has_token_type_ids:
            token_type_ids = np.zeros((bs, max_len), dtype=np.int64)
            for b, enc in enumerate(encodings):
                types = (enc.type_ids[:max_len] if enc.type_ids else [0] * len(enc.ids))
                token_type_ids[b, :len(types)] = types
            ort_inputs["token_type_ids"] = token_type_ids

        outputs = session.run(None, ort_inputs)
        logits = outputs[0]  # shape: (bs, 1) or (bs,)

        batch_scores = sigmoid(logits.flatten() if logits.ndim > 1 else logits)
        scores.extend(batch_scores.tolist())

    return scores


def run_reranker_evaluation(
    dataset_path: str,
    model_path: str,
    tokenizer_path: str,
    output_path: str,
    top_k: int = 20,
) -> Dict[str, Any]:
    """Full reranker-augmented evaluation pipeline."""
    # Load dataset
    print(f"\nLoading dataset: {dataset_path}")
    cases = load_dataset(dataset_path)
    stats = dataset_stats(cases)
    print(f"  {stats['total']} cases ({stats['non_abstention']} eval, {stats['abstention']} abstention)")
    print(f"  Types: {stats['by_type']}")

    # Load reranker
    session, tokenizer, input_names = load_reranker(model_path, tokenizer_path)

    # Determine effective max_length from model inputs
    # gte-modernbert defaults to 8192, others to 512
    effective_max_length = 8192  # gte-modernbert supports 8K

    # ── Phase 1: Dense proxy baseline (bi-encoder proxy) ──
    print("\n── Phase 1: Dense Proxy Baseline (Bi-Encoder) ──")
    dense_proxy_results = []
    for idx, case in enumerate(cases):
        hit_ids = dense_proxy_baseline(case)
        result = evaluate_single_case(hit_ids, case, K_VALUES)
        dense_proxy_results.append(result)
        if (idx + 1) % 5 == 0:
            print(f"  [{idx + 1}/{len(cases)}] baseline scored", flush=True)

    dense_summary = aggregate_results(dense_proxy_results, K_VALUES, top_k)

    # ── Phase 2: Reranker-augmented evaluation ──
    print("\n── Phase 2: Reranker-Augmented (Cross-Encoder) ──")
    print(f"  Reranking top-{top_k} candidates from dense_proxy for each query...")

    reranker_results = []
    reranker_timing_ms = []

    for idx, case in enumerate(cases):
        try:
            # Get baseline ranking
            baseline_ids = dense_proxy_baseline(case)

            # Get document texts for top-k candidates
            candidate_texts = []
            candidate_ids = []
            for sid in baseline_ids[:top_k]:
                sid_idx = case.haystack_session_ids.index(sid) if sid in case.haystack_session_ids else -1
                if sid_idx >= 0:
                    candidate_texts.append(case.session_text(sid_idx))
                    candidate_ids.append(sid)

            # Rerank
            t0 = time.monotonic()
            cross_scores = rerank_candidates(
                session, tokenizer, case.question, candidate_texts,
                input_names=input_names, max_length=effective_max_length
            )
            elapsed_ms = (time.monotonic() - t0) * 1000
            reranker_timing_ms.append(elapsed_ms)

            # Sort by cross-encoder score (descending)
            scored = list(zip(candidate_ids, cross_scores))
            scored.sort(key=lambda x: x[1], reverse=True)
            reranked_ids = [sid for sid, _ in scored]

            # Append remaining baseline IDs that weren't reranked
            for sid in baseline_ids[top_k:]:
                if sid not in reranked_ids:
                    reranked_ids.append(sid)

            # Evaluate
            result = evaluate_single_case(reranked_ids, case, K_VALUES)
            result["reranker_timing_ms"] = elapsed_ms
            reranker_results.append(result)

        except Exception as e:
            print(f"  FAIL case={case.question_id}: {e}", flush=True)
            # Fallback to baseline
            result = evaluate_single_case(baseline_ids, case, K_VALUES)
            result["reranker_timing_ms"] = 0.0
            result["error"] = str(e)
            reranker_results.append(result)

        if (idx + 1) % 5 == 0 or idx == len(cases) - 1:
            avg_timing = np.mean(reranker_timing_ms) if reranker_timing_ms else 0
            print(
                f"  [{idx + 1}/{len(cases)}] reranked "
                f"(avg {avg_timing:.0f}ms/case)",
                flush=True,
            )

    reranker_summary = aggregate_results(reranker_results, K_VALUES, top_k)

    # ── Phase 3: Compute deltas ──
    print("\n── Phase 3: Reranker vs Baseline Delta ──")
    deltas = {}

    # Old metrics
    old_metric_map = [
        ("top1", "Top-1 Rate"),
        ("recall_at_5", "Recall@5"),
        ("mrr", "MRR"),
    ]
    for key, label in old_metric_map:
        baseline_val = dense_summary["old"].get(key, 0.0)
        reranker_val = reranker_summary["old"].get(key, 0.0)
        deltas[key] = {
            "label": label,
            "baseline": baseline_val,
            "reranker": reranker_val,
            "delta": reranker_val - baseline_val,
        }
        delta_str = "+" if reranker_val >= baseline_val else ""
        print(f"  {label:12s}: {baseline_val:.4f} → {reranker_val:.4f} "
              f"({delta_str}{reranker_val - baseline_val:+.4f})")

    # NDCG@k metrics from new_session
    for k in [1, 3, 5, 10]:
        key = f"ndcg_at_{k}"
        baseline_val = dense_summary["new_session"].get(k, {}).get("ndcg_any", 0.0)
        reranker_val = reranker_summary["new_session"].get(k, {}).get("ndcg_any", 0.0)
        deltas[key] = {
            "label": f"NDCG@{k}",
            "baseline": baseline_val,
            "reranker": reranker_val,
            "delta": reranker_val - baseline_val,
        }
        delta_str = "+" if reranker_val >= baseline_val else ""
        print(f"  NDCG@{k:1d}       : {baseline_val:.4f} → {reranker_val:.4f} "
              f"({delta_str}{reranker_val - baseline_val:+.4f})")

    # ── Assemble report ──
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "task": "t_2c5c6d41",
        "reranker_model": "Alibaba-NLP/gte-reranker-modernbert-base (ONNX)",
        "reranker_format": "gte-modernbert",
        "dataset_stats": stats,
        "k_values": K_VALUES,
        "baselines": {
            "dense_proxy": dense_summary["old"],
        },
        "reranker": {
            "old": reranker_summary["old"],
            "new_session": reranker_summary["new_session"],
            "new_turn": reranker_summary["new_turn"],
            "per_type": reranker_summary["per_type"],
            "timing": {
                "mean_ms_per_case": float(np.mean(reranker_timing_ms)) if reranker_timing_ms else 0,
                "median_ms_per_case": float(np.median(reranker_timing_ms)) if reranker_timing_ms else 0,
                "total_cases": len(reranker_timing_ms),
            },
        },
        "deltas": deltas,
        "acceptance_criteria_ndcg_delta_015": {
            "threshold": 0.15,
            "baseline_ndcg_at_5": dense_summary["new_session"].get(5, {}).get("ndcg_any", 0),
            "reranker_ndcg_at_5": reranker_summary["new_session"].get(5, {}).get("ndcg_any", 0),
            "pass": (reranker_summary["new_session"].get(5, {}).get("ndcg_any", 0)
                     - dense_summary["new_session"].get(5, {}).get("ndcg_any", 0)) >= 0.15,
        },
    }

    # Save
    with open(output_path, "w") as f:
        json.dump(report, f, indent=2)
    print(f"\n✓ Report saved: {output_path}")

    # Print formatted summary
    print(format_summary(reranker_summary, stats, "reranker-augmented (gte-reranker-modernbert-base cross-encoder)"))
    baseline_ndcg5 = dense_summary["new_session"].get(5, {}).get("ndcg_any", 0.0)
    reranker_ndcg5 = reranker_summary["new_session"].get(5, {}).get("ndcg_any", 0.0)
    print(f"\n  Baseline NDCG@5: {baseline_ndcg5:.4f}")
    print(f"  Reranker NDCG@5: {reranker_ndcg5:.4f}")
    print(f"  Delta: {reranker_ndcg5 - baseline_ndcg5:+.4f}")

    return report


if __name__ == "__main__":
    import argparse

    p = argparse.ArgumentParser(description="Reranker-Augmented Evaluation")
    p.add_argument(
        "--dataset",
        default="/Users/nimarfranklinmac/.hermes/kanban/workspaces/t_8b5a6703/fixtures/test_queries.json",
        help="Path to test_queries.json",
    )
    p.add_argument(
        "--model",
        default=os.path.expanduser("~/.cache/knowwhere/reranker/model.onnx"),
        help="Path to ONNX model",
    )
    p.add_argument(
        "--tokenizer",
        default=os.path.expanduser("~/.cache/knowwhere/reranker/tokenizer.json"),
        help="Path to tokenizer.json",
    )
    p.add_argument(
        "--output",
        default="",
        help="Output path for report JSON",
    )
    p.add_argument(
        "--top-k",
        type=int,
        default=20,
        help="Number of candidates to rerank",
    )

    args = p.parse_args()

    output = args.output
    if not output:
        ts = time.strftime("%Y%m%d_%H%M%S")
        output = f"reranker_eval_report_{ts}.json"

    report = run_reranker_evaluation(
        args.dataset,
        args.model,
        args.tokenizer,
        output,
        args.top_k,
    )

    # Print key delta
    deltas = report.get("deltas", {})
    ndcg5 = deltas.get("ndcg_at_5", {})
    print(f"\n✓ Reranker NDCG@5: {ndcg5.get('reranker', 0):.4f} "
          f"(baseline: {ndcg5.get('baseline', 0):.4f}, "
          f"delta: {ndcg5.get('delta', 0):+.4f})")
