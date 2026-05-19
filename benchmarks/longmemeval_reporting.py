#!/usr/bin/env python3
"""
LongMemEval Reporting & Stratified Filter Module.

Extracted from longmemeval_eval.py to keep the main eval runner lean.
Contains:
  - Metrics engine (dcg, ndcg, recall, mrr)
  - Stratified case-list loading and validation
  - Aggregation, printing, and JSON saving of eval reports
"""

import json
import math
import os
import time
from collections import defaultdict
from pathlib import Path
from typing import Any, Dict, List, Optional, Set, Tuple

# ──────────────────────────────────────────────────────────────────────
# Constants
# ──────────────────────────────────────────────────────────────────────

QUESTION_TYPES = [
    "single-session-user",
    "single-session-assistant",
    "single-session-preference",
    "multi-session",
    "temporal-reasoning",
    "knowledge-update",
]

K_VALUES = [1, 3, 5, 10, 30, 50]

REPORT_VERSION = 2


# ──────────────────────────────────────────────────────────────────────
# Exceptions
# ──────────────────────────────────────────────────────────────────────

class StratifiedLoadError(ValueError):
    """Raised when a stratified file cannot be loaded or has invalid format."""


class StratifiedValidationError(ValueError):
    """Raised when stratified IDs don't match the dataset."""


# ──────────────────────────────────────────────────────────────────────
# Metrics Engine
# ──────────────────────────────────────────────────────────────────────

def dcg(relevances: list[float], k: int) -> float:
    """Discounted Cumulative Gain at k."""
    rels = relevances[:k]
    if not rels:
        return 0.0
    return rels[0] + sum(
        rels[i] / math.log2(i + 2) for i in range(1, len(rels))
    )


def ndcg_at_k(
    rankings: list[int], correct_ids: set[str], all_ids: list[str], k: int
) -> float:
    """Normalized DCG at k. Binary relevance: 1 if in correct_ids, 0 otherwise."""
    relevances = [1.0 if all_ids[i] in correct_ids else 0.0 for i in range(len(all_ids))]
    sorted_rels = sorted(relevances, reverse=True)
    ideal = dcg(sorted_rels, k)
    if ideal == 0:
        return 0.0
    actual = dcg([relevances[i] for i in rankings[:k]], k)
    return actual / ideal


def recall_any_at_k(
    rankings: list[int], correct_ids: set[str], all_ids: list[str], k: int
) -> float:
    """At least one correct doc in top-k."""
    recalled = {all_ids[i] for i in rankings[:k]}
    return 1.0 if recalled & correct_ids else 0.0


def recall_all_at_k(
    rankings: list[int], correct_ids: set[str], all_ids: list[str], k: int
) -> float:
    """All correct docs in top-k."""
    if not correct_ids:
        return 1.0
    recalled = {all_ids[i] for i in rankings[:k]}
    return 1.0 if correct_ids.issubset(recalled) else 0.0


def compute_metrics(
    rankings: list[int],
    all_ids: list[str],
    correct_ids: set[str],
    k_values: list[int],
) -> dict[int, dict[str, float]]:
    """Compute recall_any, recall_all, ndcg_any at each k."""
    results = {}
    for k in k_values:
        results[k] = {
            "recall_any": recall_any_at_k(rankings, correct_ids, all_ids, k),
            "recall_all": recall_all_at_k(rankings, correct_ids, all_ids, k),
            "ndcg_any": ndcg_at_k(rankings, correct_ids, all_ids, k),
        }
    return results


def mrr(rank: Optional[int]) -> float:
    """Mean Reciprocal Rank for a single rank (1-indexed)."""
    if rank is None:
        return 0.0
    return 1.0 / rank


# ──────────────────────────────────────────────────────────────────────
# Stratified Filter
# ──────────────────────────────────────────────────────────────────────

def load_stratified_filter(path: str) -> tuple[set[str], dict[str, list[str]]]:
    """Load a stratified case list from a JSON file.

    Supports two formats:

    1. Type-keyed strata (dict of type → list of question_ids):
       {
         "single-session-user": ["qid1", "qid2"],
         "multi-session": ["qid3"]
       }

    2. Flat list of question_ids:
       ["qid1", "qid2", "qid3"]

    Returns (all_ids_set, strata_mapping).
    strata_mapping is empty for flat-list format.

    Raises StratifiedLoadError on missing file or unrecognised format.
    """
    filepath = Path(path)
    if not filepath.exists():
        raise StratifiedLoadError(f"Stratified file not found: {path}")

    with open(path) as f:
        data = json.load(f)

    if isinstance(data, list):
        ids = set(data)
        return ids, {}

    if isinstance(data, dict):
        all_ids = set()
        strata = {}
        for qtype, ids in data.items():
            strata[qtype] = list(ids)
            all_ids.update(ids)
        return all_ids, strata

    raise StratifiedLoadError(
        f"Unrecognized stratified file format in {path}. "
        f"Expected a JSON object (type→[qids]) or a JSON array of qids."
    )


def validate_stratified_ids(
    allowed_ids: set[str],
    cases: list,  # list[EvalCase] — avoid circular import
) -> Tuple[set[str], set[str]]:
    """Validate that stratified IDs exist in the dataset.

    Returns (matched_ids, missing_ids) where:
      - matched_ids: IDs present in both stratified list and cases
      - missing_ids: IDs in stratified list but not in any case

    Raises StratifiedValidationError if NO stratified IDs match any case.
    """
    case_ids = {c.question_id for c in cases}
    matched = allowed_ids & case_ids
    missing = allowed_ids - case_ids
    if not matched:
        raise StratifiedValidationError(
            f"None of the {len(allowed_ids)} stratified question IDs "
            f"were found in the dataset ({len(cases)} cases). "
            f"First few expected: {list(allowed_ids)[:5]}"
        )
    return matched, missing


def apply_stratified_filter(
    cases: list,  # list[EvalCase] — avoid circular import
    allowed_ids: set[str],
    strata: dict[str, list[str]],
) -> tuple[list, dict[str, list[str]]]:
    """Filter cases to only those in the stratified list, preserving strata info.

    Returns (filtered_cases, applied_strata) where applied_strata maps
    question_type_norm → list of question_ids that were actually found.
    """
    allowed = set(allowed_ids)
    filtered = [c for c in cases if c.question_id in allowed]

    applied_strata = {}
    if strata:
        for qtype, ids in strata.items():
            found = [qid for qid in ids if qid in allowed]
            if found:
                applied_strata[qtype] = found
    else:
        for c in filtered:
            qt = c.question_type_norm
            if qt not in applied_strata:
                applied_strata[qt] = []
            applied_strata[qt].append(c.question_id)

    return filtered, applied_strata


# ──────────────────────────────────────────────────────────────────────
# Aggregation
# ──────────────────────────────────────────────────────────────────────

def _compute_per_type_metrics(
    non_abs_results: list[dict],
    k_values: list[int],
    top_k: int,
) -> dict:
    """Compute per-type breakdown with full new metrics at all k-values."""
    per_type = {}
    for qtype in QUESTION_TYPES:
        type_cases = [
            r for r in non_abs_results if r.get("question_type_norm") == qtype
        ]
        tn_type = max(len(type_cases), 1)
        if not type_cases:
            continue

        per_type[qtype] = {
            "count": len(type_cases),
            "old": {
                "top1": sum(r["session"]["top1"] for r in type_cases) / tn_type,
                "recall_at_5": sum(r["session"]["recall_at_5"] for r in type_cases) / tn_type,
                f"recall_at_{top_k}": sum(
                    1.0 if r["session"]["rank"] is not None and r["session"]["rank"] <= top_k
                    else 0.0
                    for r in type_cases
                ) / tn_type,
                "mrr": sum(r["session"]["mrr"] for r in type_cases) / tn_type,
            },
            "new_session": {},
            "new_turn": {},
        }

        # Full new session metrics per-k
        for k in k_values:
            per_type[qtype]["new_session"][k] = {
                "recall_any": sum(
                    r["session"]["new"].get(k, {}).get("recall_any", 0.0)
                    for r in type_cases
                ) / tn_type,
                "recall_all": sum(
                    r["session"]["new"].get(k, {}).get("recall_all", 0.0)
                    for r in type_cases
                ) / tn_type,
                "ndcg_any": sum(
                    r["session"]["new"].get(k, {}).get("ndcg_any", 0.0)
                    for r in type_cases
                ) / tn_type,
            }

        # Full new turn metrics per-k (if available)
        type_turn_cases = [r for r in type_cases if r.get("turn")]
        if type_turn_cases:
            ttn = max(len(type_turn_cases), 1)
            for k in k_values:
                per_type[qtype]["new_turn"][k] = {
                    "recall_any": sum(
                        r["turn"].get(k, {}).get("recall_any", 0.0)
                        for r in type_turn_cases
                    ) / ttn,
                    "recall_all": sum(
                        r["turn"].get(k, {}).get("recall_all", 0.0)
                        for r in type_turn_cases
                    ) / ttn,
                    "ndcg_any": sum(
                        r["turn"].get(k, {}).get("ndcg_any", 0.0)
                        for r in type_turn_cases
                    ) / ttn,
                }

    return per_type


def _compute_overall_score(per_type: dict) -> float:
    """Compute overall score as mean recall@5 across types with data."""
    type_recalls = []
    for qtype in QUESTION_TYPES:
        pt = per_type.get(qtype)
        if pt and pt["count"] > 0:
            type_recalls.append(pt["old"]["recall_at_5"])
    return sum(type_recalls) / len(type_recalls) if type_recalls else 0.0


def aggregate_results(
    case_results: list[dict],
    k_values: list[int],
    top_k: int,
) -> dict:
    """Aggregate per-case results into summary statistics."""
    non_abs = [r for r in case_results if not r["is_abstention"]]
    n = max(len(non_abs), 1)

    # Old metrics aggregation
    old = {
        "total_cases": len(case_results),
        "evaluated_cases": len(non_abs),
        "top1": sum(r["session"]["top1"] for r in non_abs) / n,
        "recall_at_5": sum(r["session"]["recall_at_5"] for r in non_abs) / n,
        f"recall_at_{top_k}": sum(
            1.0 if r["session"]["rank"] is not None and r["session"]["rank"] <= top_k
            else 0.0
            for r in non_abs
        ) / n,
        "mrr": sum(r["session"]["mrr"] for r in non_abs) / n,
        "top_k": top_k,
    }

    # New metrics aggregation (session-level)
    new_session = {}
    for k in k_values:
        vals = {}
        for metric in ["recall_any", "recall_all", "ndcg_any"]:
            vals[metric] = (
                sum(r["session"]["new"].get(k, {}).get(metric, 0.0) for r in non_abs)
                / n
            )
        new_session[k] = vals

    # New metrics aggregation (turn-level, if available)
    new_turn = {}
    turn_cases = [r for r in non_abs if r.get("turn")]
    tn = max(len(turn_cases), 1)
    if turn_cases:
        for k in k_values:
            vals = {}
            for metric in ["recall_any", "recall_all", "ndcg_any"]:
                vals[metric] = (
                    sum(r["turn"].get(k, {}).get(metric, 0.0) for r in turn_cases)
                    / tn
                )
            new_turn[k] = vals

    # Per-type breakdown (delegated to helper)
    per_type = _compute_per_type_metrics(non_abs, k_values, top_k)

    # Abstention metrics
    abs_cases = [r for r in case_results if r["is_abstention"]]
    abstention = {
        "count": len(abs_cases),
        "correctly_abstained": sum(
            1 for r in abs_cases if r["session"]["rank"] is None
        ),
    }
    if abstention["count"] > 0:
        abstention["accuracy"] = (
            abstention["correctly_abstained"] / abstention["count"]
        )

    overall_score = _compute_overall_score(per_type)

    return {
        "overall_score": overall_score,
        "old": old,
        "new_session": new_session,
        "new_turn": new_turn,
        "per_type": per_type,
        "abstention": abstention,
    }


# ──────────────────────────────────────────────────────────────────────
# Reporting (Human-Readable + JSON)
# ──────────────────────────────────────────────────────────────────────

def print_report(summary: dict, stats: dict, mode: str) -> None:
    """Print human-readable evaluation report."""
    print("\n" + "=" * 70)
    print("  LongMemEval Evaluation Report")
    print("=" * 70)

    print(f"\n  Dataset: {stats['total']} cases ({stats['non_abstention']} eval, {stats['abstention']} abstention)")
    print(f"  Mode: {mode}")
    if stats.get("has_turn_labels"):
        print(f"  Turn labels: YES ({stats['has_turn_labels']} cases with has_answer)")
    else:
        print(f"  Turn labels: NO (session-level only)")

    # ── Old Metrics ──
    old = summary["old"]
    print("\n  ── OLD METRICS (session-level, for comparison) ──")
    print(f"  top1:          {old['top1']:.4f}")
    print(f"  recall@5:      {old['recall_at_5']:.4f}")
    rk_key = f"recall_at_{old['top_k']}"
    print(f"  recall@{old['top_k']}:     {old[rk_key]:.4f}")
    print(f"  MRR:           {old['mrr']:.4f}")

    # ── New Session Metrics ──
    new_sess = summary["new_session"]
    print("\n  ── NEW METRICS (session-level recall + NDCG) ──")
    header = f"  {'k':>3}  {'recall_any':>12}  {'recall_all':>12}  {'ndcg_any':>10}"
    print(header)
    print("  " + "-" * len(header))
    for k in K_VALUES:
        m = new_sess.get(k, {})
        print(
            f"  {k:>3}  {m.get('recall_any', 0):>12.4f}  "
            f"{m.get('recall_all', 0):>12.4f}  {m.get('ndcg_any', 0):>10.4f}"
        )

    # ── New Turn Metrics ──
    new_turn = summary.get("new_turn", {})
    if new_turn:
        print("\n  ── NEW METRICS (turn-level recall + NDCG) ──")
        header = f"  {'k':>3}  {'recall_any':>12}  {'recall_all':>12}  {'ndcg_any':>10}"
        print(header)
        print("  " + "-" * len(header))
        for k in K_VALUES:
            m = new_turn.get(k, {})
            print(
                f"  {k:>3}  {m.get('recall_any', 0):>12.4f}  "
                f"{m.get('recall_all', 0):>12.4f}  {m.get('ndcg_any', 0):>10.4f}"
            )

    # ── Per-Type Breakdown ──
    per_type = summary["per_type"]
    if per_type:
        print("\n  ── PER-TYPE BREAKDOWN ──")
        print(f"  {'Type':<28} {'Count':>5}  {'top1':>8}  {'recall@5':>10}  {'recall@k':>10}  {'MRR':>8}  {'new_recall@5':>14}  {'new_ndcg@5':>12}")
        print("  " + "-" * 120)
        for qtype in QUESTION_TYPES:
            pt = per_type.get(qtype)
            if pt and pt.get("count", 0) > 0:
                old_t = pt["old"]
                ns = pt.get("new_session", {}).get(5, {})
                rk_val = old_t.get(f"recall_at_{old['top_k']}", 0)
                print(
                    f"  {qtype:<28} {pt['count']:>5}  "
                    f"{old_t['top1']:>8.4f}  {old_t['recall_at_5']:>10.4f}  "
                    f"{rk_val:>10.4f}  "
                    f"{old_t['mrr']:>8.4f}  "
                    f"{ns.get('recall_any', 0):>14.4f}  "
                    f"{ns.get('ndcg_any', 0):>12.4f}"
                )

        # Show full per-type new metrics at all k-values
        print("\n  ── PER-TYPE NEW METRICS (all k-values) ──")
        for qtype in QUESTION_TYPES:
            pt = per_type.get(qtype)
            if pt and pt.get("count", 0) > 0:
                ns_full = pt.get("new_session", {})
                if ns_full:
                    print(f"\n  {qtype} ({pt['count']} cases):")
                    header = f"    {'k':>3}  {'recall_any':>12}  {'recall_all':>12}  {'ndcg_any':>10}"
                    print(header)
                    print("    " + "-" * len(header))
                    for k in K_VALUES:
                        m = ns_full.get(k, {})
                        print(
                            f"    {k:>3}  {m.get('recall_any', 0):>12.4f}  "
                            f"{m.get('recall_all', 0):>12.4f}  {m.get('ndcg_any', 0):>10.4f}"
                        )

    # ── Abstention ──
    abst = summary.get("abstention", {})
    if abst.get("count", 0) > 0:
        print(f"\n  ── ABSTENTION ──")
        print(f"  Total: {abst['count']}")
        print(f"  Correctly abstained: {abst.get('correctly_abstained', 0)}")
        if "accuracy" in abst:
            print(f"  Accuracy: {abst['accuracy']:.4f}")

    # ── Comparison Summary ──
    print("\n  ── COMPARISON: Old vs New ──")
    old_recall_at_5 = old["recall_at_5"]
    new_recall_at_5 = new_sess.get(5, {}).get("recall_any", 0)
    new_ndcg_at_5 = new_sess.get(5, {}).get("ndcg_any", 0)
    overall_score = summary.get("overall_score", 0.0)
    print(f"  Overall score (mean recall@5): {overall_score:.4f}")
    print(f"  Old recall@5:        {old_recall_at_5:.4f}")
    print(f"  New session recall@5: {new_recall_at_5:.4f}")
    print(f"  New session NDCG@5:   {new_ndcg_at_5:.4f}")
    print(f"  Δ (recall):           {new_recall_at_5 - old_recall_at_5:+.4f}")
    print(f"  Old MRR:              {old['mrr']:.4f}")
    print("=" * 70)


def save_report(
    summary: dict,
    stats: dict,
    mode: str,
    output_path: str,
    strata: Optional[dict] = None,
    legacy: bool = False,
) -> None:
    """Save full report as structured JSON.

    Args:
        summary: Aggregated results from aggregate_results().
        stats: Dataset statistics from dataset_stats().
        mode: Evaluation mode string.
        output_path: Where to write the JSON file.
        strata: Optional strata mapping for stratified runs.
        legacy: If True, emit v1-format report (old metrics only, no new_session/new_turn).

    Report format (v2, default):
        {
          "version": 2,
          "config": { "mode": "...", "k_values": [...] },
          "stats": { ... },
          "overall": {
            "score": 0.85,
            "old": { ... },
            "new_session": { ... },
            "new_turn": { ... },
            "abstention": { ... }
          },
          "per_type": {
            "single-session-user": {
              "count": N,
              "old": { ... },
              "new_session": { 1: {...}, 3: {...}, ... },
              "new_turn": { 1: {...}, 3: {...}, ... }
            },
            ...
          }
        }
    """
    report: dict[str, Any] = {
        "version": REPORT_VERSION if not legacy else 1,
        "config": {
            "mode": mode,
            "k_values": K_VALUES,
        },
        "stats": stats,
    }

    overall: dict[str, Any] = {
        "score": summary.get("overall_score", 0.0),
    }

    if legacy:
        overall["description"] = "Mean recall@5 across question types"
        overall["legacy"] = True
    else:
        overall["description"] = "Mean recall@5 across question types"

    overall["old"] = summary["old"]

    if not legacy:
        overall["new_session"] = summary["new_session"]
        overall["new_turn"] = summary.get("new_turn", {})
        overall["abstention"] = summary.get("abstention", {})

    report["overall"] = overall

    if legacy:
        # Strip new_session/new_turn from per_type — keep only old metrics
        legacy_per_type = {}
        for qtype, pt in summary.get("per_type", {}).items():
            legacy_per_type[qtype] = {
                "count": pt["count"],
                "old": pt["old"],
            }
        report["per_type"] = legacy_per_type
    else:
        report["per_type"] = summary.get("per_type", {})

    if strata:
        report["strata"] = strata

    os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(report, f, indent=2, default=str)
    print(f"\n  Report saved to: {output_path}")
