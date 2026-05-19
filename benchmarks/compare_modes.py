#!/usr/bin/env python3
"""Compare percase vs multi mode LongMemEval evaluation results.

Produces side-by-side tables showing:
- Old metrics (top1, recall@5, recall@k, MRR)
- New session-level metrics (recall_any/recall_all/ndcg_any @ k=[1,3,5,10,30,50])
- New turn-level metrics (if available)
- Per-type breakdown comparison
- Delta (multi − percase) for all metrics
"""

import json
import sys
from pathlib import Path


def load_report(path: str) -> dict:
    with open(path) as f:
        return json.load(f)


def fmt_pct(val: float) -> str:
    return f"{val:.4f}"


def fmt_delta(val: float) -> str:
    sign = "+" if val >= 0 else ""
    return f"{sign}{val:.4f}"


def compare_reports(percase_path: str, multi_path: str) -> str:
    percase = load_report(percase_path)
    multi = load_report(multi_path)

    pc_summary = percase["summary"]
    ms_summary = multi["summary"]

    pc_old = pc_summary["old"]
    ms_old = ms_summary["old"]
    pc_new_sess = pc_summary["new_session"]
    ms_new_sess = ms_summary["new_session"]
    pc_new_turn = pc_summary.get("new_turn", {})
    ms_new_turn = ms_summary.get("new_turn", {})
    pc_pt = pc_summary.get("per_type", {})
    ms_pt = ms_summary.get("per_type", {})

    lines = []
    sep = "=" * 90

    lines.append(sep)
    lines.append("  LongMemEval: Percase vs Multi-Mode Comparison")
    lines.append(sep)

    # ── Dataset Info ──
    lines.append(f"\n  Dataset: {pc_summary['old']['total_cases']} cases "
                 f"({pc_summary['old']['evaluated_cases']} eval)")
    lines.append(f"  Percase mode: {percase['config']['mode']}")
    lines.append(f"  Multi mode:   {multi['config']['mode']}")
    lines.append(f"  k-values:     {percase['config']['k_values']}")

    # ── Old Metrics Comparison ──
    lines.append(f"\n  {'─' * 88}")
    lines.append("  OLD METRICS (session-level)")
    lines.append(f"  {'─' * 88}")
    lines.append(f"  {'Metric':<20} {'Percase':>12} {'Multi':>12} {'Delta':>12}")
    lines.append(f"  {'-' * 56}")

    old_metrics = [
        ("top1", "top1"),
        ("recall@5", "recall_at_5"),
        (f"recall@{pc_old['top_k']}", f"recall_at_{pc_old['top_k']}"),
        ("MRR", "mrr"),
    ]
    for label, key in old_metrics:
        pc_val = pc_old[key]
        ms_val = ms_old[key]
        delta = ms_val - pc_val
        lines.append(f"  {label:<20} {fmt_pct(pc_val):>12} "
                     f"{fmt_pct(ms_val):>12} {fmt_delta(delta):>12}")

    # ── New Session Metrics ──
    lines.append(f"\n  {'─' * 88}")
    lines.append("  NEW SESSION-LEVEL METRICS (recall_any / ndcg_any)")
    lines.append(f"  {'─' * 88}")

    for k in [1, 3, 5, 10, 30, 50]:
        pc_r = pc_new_sess[str(k)]
        ms_r = ms_new_sess[str(k)]
        lines.append(f"\n  @k={k}:")
        lines.append(f"  {'Metric':<20} {'Percase':>12} {'Multi':>12} {'Delta':>12}")
        lines.append(f"  {'-' * 56}")
        for metric in ["recall_any", "recall_all", "ndcg_any"]:
            pc_val = pc_r.get(metric, 0)
            ms_val = ms_r.get(metric, 0)
            delta = ms_val - pc_val
            lines.append(f"  {metric:<20} {fmt_pct(pc_val):>12} "
                         f"{fmt_pct(ms_val):>12} {fmt_delta(delta):>12}")

    # ── New Turn Metrics ──
    if pc_new_turn or ms_new_turn:
        lines.append(f"\n  {'─' * 88}")
        lines.append("  NEW TURN-LEVEL METRICS (recall_any / ndcg_any)")
        lines.append(f"  {'─' * 88}")

        for k in [1, 3, 5, 10, 30, 50]:
            k_str = str(k)
            if k_str not in pc_new_turn and k_str not in ms_new_turn:
                continue
            pc_r = pc_new_turn.get(k_str, {})
            ms_r = ms_new_turn.get(k_str, {})
            lines.append(f"\n  @k={k}:")
            lines.append(f"  {'Metric':<20} {'Percase':>12} {'Multi':>12} {'Delta':>12}")
            lines.append(f"  {'-' * 56}")
            for metric in ["recall_any", "recall_all", "ndcg_any"]:
                pc_val = pc_r.get(metric, 0)
                ms_val = ms_r.get(metric, 0)
                delta = ms_val - pc_val
                lines.append(f"  {metric:<20} {fmt_pct(pc_val):>12} "
                             f"{fmt_pct(ms_val):>12} {fmt_delta(delta):>12}")

    # ── Per-Type Breakdown ──
    lines.append(f"\n  {'─' * 88}")
    lines.append("  PER-TYPE BREAKDOWN (old recall@5 / new session recall@5 / NDCG@5)")
    lines.append(f"  {'─' * 88}")
    lines.append(f"  {'Type':<28} {'Count':>5}  {'PC top1':>8}  {'PC r@5':>8}  "
                 f"{'MS r@5':>8}  {'MS NDCG@5':>10}")
    lines.append(f"  {'-' * 78}")

    all_types = sorted(set(list(pc_pt.keys()) + list(ms_pt.keys())))
    for qtype in all_types:
        pc_t = pc_pt.get(qtype, {})
        ms_t = ms_pt.get(qtype, {})
        count = pc_t.get("count", ms_t.get("count", 0))
        if count == 0:
            continue
        pc_top1 = pc_t.get("old", {}).get("top1", 0)
        pc_r5 = pc_t.get("old", {}).get("recall_at_5", 0)
        ms_r5 = ms_t.get("new_session_k5", {}).get("recall_any", 0)
        ms_ndcg5 = ms_t.get("new_session_k5", {}).get("ndcg_any", 0)
        lines.append(f"  {qtype:<28} {count:>5}  {fmt_pct(pc_top1):>8}  "
                     f"{fmt_pct(pc_r5):>8}  {fmt_pct(ms_r5):>8}  {fmt_pct(ms_ndcg5):>10}")

    # ── Abstention ──
    pc_abs = pc_summary.get("abstention", {})
    ms_abs = ms_summary.get("abstention", {})
    lines.append(f"\n  {'─' * 88}")
    lines.append("  ABSTENTION")
    lines.append(f"  {'─' * 88}")
    lines.append(f"  Percase: {pc_abs.get('count', 0)} total, "
                 f"{pc_abs.get('correctly_abstained', 0)} correct "
                 f"({pc_abs.get('accuracy', 0):.2%})")
    lines.append(f"  Multi:   {ms_abs.get('count', 0)} total, "
                 f"{ms_abs.get('correctly_abstained', 0)} correct "
                 f"({ms_abs.get('accuracy', 0):.2%})")

    # ── Key Takeaways ──
    lines.append(f"\n  {'─' * 88}")
    lines.append("  KEY TAKEAWAYS")
    lines.append(f"  {'─' * 88}")

    # Find the biggest deltas
    pc_top1 = pc_old["top1"]
    ms_top1 = ms_old["top1"]
    pc_r5 = pc_old["recall_at_5"]
    ms_r5 = ms_old["recall_at_5"]

    lines.append(f"  top1 delta:       {fmt_delta(ms_top1 - pc_top1)} "
                 f"({fmt_pct(pc_top1)} → {fmt_pct(ms_top1)})")
    lines.append(f"  recall@5 delta:   {fmt_delta(ms_r5 - pc_r5)} "
                 f"({fmt_pct(pc_r5)} → {fmt_pct(ms_r5)})")

    # NDCG@5 delta
    pc_ndcg5 = pc_new_sess.get("5", {}).get("ndcg_any", 0)
    ms_ndcg5 = ms_new_sess.get("5", {}).get("ndcg_any", 0)
    lines.append(f"  NDCG@5 delta:     {fmt_delta(ms_ndcg5 - pc_ndcg5)} "
                 f"({fmt_pct(pc_ndcg5)} → {fmt_pct(ms_ndcg5)})")

    lines.append("")
    lines.append(sep)

    return "\n".join(lines)


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <percase_report.json> <multi_report.json>")
        sys.exit(1)

    report = compare_reports(sys.argv[1], sys.argv[2])
    print(report)

    # Save to file
    out_path = Path(sys.argv[2]).parent / "longmemeval_comparison_report.txt"
    with open(out_path, "w") as f:
        f.write(report)
    print(f"\nSaved to: {out_path}")


if __name__ == "__main__":
    main()
