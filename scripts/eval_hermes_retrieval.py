#!/usr/bin/env python3
"""Evaluate whether KnowWhere retrieval is useful for Hermes prefetch."""

from __future__ import annotations

import argparse
import json
import os
import time
import urllib.error
import urllib.request


GOLDEN_QUERIES = [
    {"intent": "open_recall", "query": "Was haben wir zuletzt an KnowWhere konkret geaendert?"},
    {"intent": "current_state", "query": "Ist KnowWhere gerade in Hermes aktiv?"},
    {"intent": "decision_why", "query": "Welche Entscheidungen wurden zur Retrieval-Scoring-Logik getroffen?"},
    {"intent": "procedure", "query": "Wie startet man KnowWhere fuer Hermes?"},
    {"intent": "open_recall", "query": "Welche Bugs sind aktuell offen in KnowWhere?"},
    {"intent": "current_state", "query": "Wie ist der aktuelle Stand der Hermes-Integration mit KnowWhere?"},
    {"intent": "open_recall", "query": "Welche bekannten Instabilitaeten gibt es bei Prefetch und Retrieval?"},
    {"intent": "decision_why", "query": "Was wurde heute zur Decision-Pipeline entschieden?"},
    {"intent": "open_recall", "query": "Welche Massnahmen wurden fuer bessere Retrieval-Qualitaet genannt?"},
    {"intent": "open_recall", "query": "Was sollten wir als Naechstes in KnowWhere umsetzen?"},
    {"intent": "historical", "query": "Warum wurde KnowWhere zeitweise aus Hermes entfernt?"},
    {"intent": "preference", "query": "Welche Praeferenzen gelten fuer die Arbeit an KnowWhere?"},
]

STALE_MARKERS = ("deaktiviert", "entfernt", "switch to", "null integration")
META_PREFIXES = ("<knowwhere_reflect>", "<knowwhere_memory>", "<memory-context>")


def env_api_key() -> str:
    if os.getenv("KNOWWHERE_API_KEY"):
        return os.environ["KNOWWHERE_API_KEY"]
    try:
        with open(".env", encoding="utf-8") as handle:
            for line in handle:
                if line.startswith("KNOWWHERE_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"').strip("'")
    except OSError:
        pass
    return "kw_testkey_12345"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="http://127.0.0.1:3737")
    parser.add_argument("--api-key", default=env_api_key())
    parser.add_argument("--top-k", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--reflect", action="store_true")
    parser.add_argument("--fixed-query-vector-dim", type=int, default=0)
    parser.add_argument("--fail-gates", action="store_true")
    parser.add_argument("--json", action="store_true")
    return parser.parse_args()


def is_meta(node: dict) -> bool:
    content = (node.get("content") or "").strip().lower()
    memory_type = (node.get("memory_type") or "").lower()
    return memory_type == "meta" or content.startswith(META_PREFIXES)


def has_provenance(node: dict) -> bool:
    metadata = node.get("metadata") or {}
    return any(
        key in metadata
        for key in (
            "session_id",
            "source_session_ids",
            "source_node_ids",
            "original_pointer",
            "imported_from",
            "observed_at",
        )
    )


def post_retrieve(args: argparse.Namespace, query: str, **extra: object) -> tuple[list, float]:
    payload = {"query_text": query, "top_k": args.top_k, **extra}
    if args.fixed_query_vector_dim > 0:
        payload["query_vector"] = [1.0] * args.fixed_query_vector_dim
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        f"{args.endpoint}/retrieve_fractal",
        data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {args.api_key}"},
        method="POST",
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=args.timeout) as response:
        return json.loads(response.read().decode("utf-8")), time.perf_counter() - started


def _session_id(node: dict) -> str:
    m = node.get("metadata") or {}
    sid = m.get("session_id")
    return sid if isinstance(sid, str) else ""


def _source_key(node: dict) -> str:
    return f"{node.get('memory_type') or ''}|{node.get('source') or ''}"


def _fractal_path_hit(node: dict) -> bool:
    m = node.get("metadata") or {}
    return any(
        k in m
        for k in (
            "derived_from",
            "source_node_ids",
            "source_session_ids",
            "source_turn_range",
        )
    )


def flags(nodes: list[dict]) -> dict:
    raw_top1_meta = bool(nodes and is_meta(nodes[0]))
    usable = [node for node in nodes if not is_meta(node)]
    top3 = usable[:3]
    text = " ".join((node.get("content") or "").lower() for node in top3)
    sessions = {_session_id(n) for n in top3 if _session_id(n)}
    sources = {_source_key(n) for n in top3}
    return {
        "raw_top1_is_meta": raw_top1_meta,
        "top1_is_meta": raw_top1_meta,
        "top3_non_meta": sum(not is_meta(node) for node in top3),
        "top3_decisions": sum((node.get("memory_type") or "").lower() == "decision" for node in top3),
        "top1_id": str(top3[0].get("id", "")) if top3 else "",
        "provenance_hits": sum(has_provenance(node) for node in top3),
        "stale_markers": sum(marker in text for marker in STALE_MARKERS),
        "parent_session_diversity": (len(sessions) / len(top3)) if top3 else 0.0,
        "source_diversity": (len(sources) / len(top3)) if top3 else 0.0,
        "fractal_path_hits": sum(_fractal_path_hit(node) for node in top3),
        "novelty_gain": novelty_gain(top3),
    }


def novelty_gain(nodes: list[dict]) -> float:
    if not nodes:
        return 0.0
    seen = set()
    gained = 0
    for node in nodes:
        bits = (
            _session_id(node),
            _source_key(node),
            str((node.get("metadata") or {}).get("source_node_ids", [])),
            (node.get("memory_type") or "").lower(),
        )
        if bits not in seen:
            gained += 1
            seen.add(bits)
    return gained / len(nodes)


def evaluate_query(args: argparse.Namespace, item: dict) -> dict:
    query = item["query"]
    intent = item["intent"]
    try:
        nodes, latency = post_retrieve(args, query, reflect=args.reflect, query_intent=intent)
        decision_nodes, _ = post_retrieve(args, query, memory_type_filter="decision", query_intent="decision_why")
        decision_pure = all((n.get("memory_type") or "").lower() == "decision" for n in decision_nodes)
        return {
            "query": query,
            "intent": intent,
            "ok": True,
            "latency": latency,
            "flags": flags(nodes),
            "decision_pure": decision_pure,
        }
    except (OSError, urllib.error.HTTPError, TimeoutError) as exc:
        return {"query": query, "intent": intent, "ok": False, "error": str(exc)}


def unique_top1_rate(items: list[dict]) -> float:
    top_ids = [item["flags"]["top1_id"] for item in items if item["flags"]["top1_id"]]
    if not top_ids:
        return 0.0
    return round(len(set(top_ids)) / len(top_ids), 3)


def mean_flag(ok: list[dict], key: str) -> float:
    if not ok:
        return 0.0
    return round(sum(item["flags"][key] for item in ok) / len(ok), 3)


def summarize(results: list[dict]) -> dict:
    ok = [item for item in results if item.get("ok")]
    latencies = sorted(item["latency"] for item in ok)
    return {
        "total": len(results),
        "successful": len(ok),
        "failed": len(results) - len(ok),
        "top1_non_meta_rate": rate(ok, lambda item: not item["flags"]["top1_is_meta"]),
        "raw_meta_top1_rate": rate(ok, lambda item: item["flags"]["raw_top1_is_meta"]),
        "top3_actionable_rate": rate(ok, lambda item: item["flags"]["top3_non_meta"] > 0),
        "decision_filter_purity": rate(ok, lambda item: item["decision_pure"]),
        "provenance_coverage": provenance_coverage(ok),
        "repeated_top1_rate": repeated_top1_rate(ok),
        "unique_top1_rate": unique_top1_rate(ok),
        "mean_source_diversity": mean_flag(ok, "source_diversity"),
        "mean_session_diversity": mean_flag(ok, "parent_session_diversity"),
        "fractal_path_coverage": fractal_path_coverage(ok),
        "mean_novelty_gain": mean_flag(ok, "novelty_gain"),
        "stale_conflict_rate": rate(ok, lambda item: item["flags"]["stale_markers"] == 0),
        "latency_p50": percentile(latencies, 0.50),
        "latency_p95": percentile(latencies, 0.95),
    }


def rate(items: list[dict], predicate) -> float:
    if not items:
        return 0.0
    return round(sum(1 for item in items if predicate(item)) / len(items), 3)


def provenance_coverage(items: list[dict]) -> float:
    denominator = sum(item["flags"]["top3_non_meta"] for item in items)
    if denominator == 0:
        return 0.0
    numerator = sum(item["flags"]["provenance_hits"] for item in items)
    return round(numerator / denominator, 3)


def fractal_path_coverage(items: list[dict]) -> float:
    denominator = sum(item["flags"]["top3_non_meta"] for item in items)
    if denominator == 0:
        return 0.0
    numerator = sum(item["flags"]["fractal_path_hits"] for item in items)
    return round(numerator / denominator, 3)


def repeated_top1_rate(items: list[dict]) -> float:
    top_ids = [item["flags"]["top1_id"] for item in items if item["flags"]["top1_id"]]
    if not top_ids:
        return 0.0
    most_common = max(top_ids.count(value) for value in set(top_ids))
    return round(most_common / len(top_ids), 3)


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        return 0.0
    index = min(len(values) - 1, round((len(values) - 1) * quantile))
    return round(values[index], 3)


def main() -> int:
    args = parse_args()
    results = [evaluate_query(args, item) for item in GOLDEN_QUERIES]
    report = {"summary": summarize(results), "results": results}
    print(json.dumps(report, ensure_ascii=False, indent=2) if args.json else json.dumps(report["summary"], indent=2))
    if args.fail_gates and not gates_pass(report["summary"]):
        return 2
    return 0 if report["summary"]["failed"] == 0 else 1


def gates_pass(summary: dict) -> bool:
    return (
        summary["failed"] == 0
        and summary["top1_non_meta_rate"] >= 0.7
        and summary["decision_filter_purity"] == 1.0
        and summary["provenance_coverage"] >= 0.8
        and summary["repeated_top1_rate"] <= 0.5
        and summary["mean_source_diversity"] >= 0.35
        and summary["mean_novelty_gain"] >= 0.5
        and summary["stale_conflict_rate"] >= 0.9
    )


if __name__ == "__main__":
    raise SystemExit(main())
