#!/usr/bin/env python3
"""
LongMemEval Evaluation Script — Updated for cross-session metrics.

Supports:
  - All 6 question types (single-session-user, single-session-assistant,
    single-session-preference, multi-session, temporal-reasoning,
    knowledge-update) plus abstention
  - Genuine multi-session data layout (index all, query all — or per-case)
  - Cross-session metrics: turn-level + session-level recall/NDCG
  - Per-type breakdown with full new metrics at all k-values
  - Stratified evaluation: restrict to pre-selected case lists by type
  - Structured JSON report with overall score + per-type breakdown
  - Old vs new metric comparison output

Usage:
    python longmemeval_eval.py --dataset <path> [--mode multi|percase] [--base-url URL] [--api-key KEY]
    python longmemeval_eval.py --dataset <path> --stratified <path> [--mode multi|percase]

Environment:
    KNOWWHERE_BASE_URL   — default http://127.0.0.1:3737
    KNOWWHERE_API_KEY    — required
    KNOWWHERE_TOP_K      — default 20 (for old recall@k), new metrics use k=[1,3,5,10,30,50]
    KNOWWHERE_MAX_CASES  — limit number of cases (default: all)
"""

import argparse
import asyncio
import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Optional

# Ensure benchmarks/ is on the path so we can import longmemeval_reporting
_here = os.path.dirname(os.path.abspath(__file__))
if _here not in sys.path:
    sys.path.insert(0, _here)

import aiohttp

from longmemeval_reporting import (
    K_VALUES,
    QUESTION_TYPES,
    StratifiedLoadError,
    StratifiedValidationError,
    aggregate_results,
    apply_stratified_filter,
    compute_metrics,
    load_stratified_filter,
    print_report,
    save_report,
    validate_stratified_ids,
)


# ──────────────────────────────────────────────────────────────────────
# Data Model
# ──────────────────────────────────────────────────────────────────────


class EvalCase:
    """Parsed evaluation case from dataset."""

    def __init__(self, raw: dict):
        self.question_id: str = raw["question_id"]
        self.question: str = raw["question"]
        self.question_type: str = raw.get("question_type", "")
        self.question_date: str = raw.get("question_date", "")
        self.answer: str = raw.get("answer", "")
        self.answer_session_ids: list[str] = [
            str(x) for x in raw.get("answer_session_ids", [])
        ]
        self.haystack_session_ids: list[str] = [
            str(x) for x in raw.get("haystack_session_ids", [])
        ]
        self.haystack_dates: list[str] = [
            str(x) for x in raw.get("haystack_dates", [])
        ]
        self.haystack_sessions: list[list[dict]] = raw.get("haystack_sessions", [])

        # Detect format
        self._has_answer_labels = self._detect_has_answer()

    def _detect_has_answer(self) -> bool:
        for session in self.haystack_sessions:
            for turn in session:
                if turn.get("role") == "user" and "has_answer" in turn:
                    return True
        return False

    @property
    def is_abstention(self) -> bool:
        return self.question_id.endswith("_abs") or not self.answer_session_ids

    @property
    def question_type_norm(self) -> str:
        """Normalized question type for grouping."""
        t = self.question_type or ""
        mapping = {
            "single-session-user": "single-session-user",
            "single-session-assistant": "single-session-assistant",
            "implicit_preference_v2": "single-session-preference",
            "single-session-preference": "single-session-preference",
            "two_hop": "multi-session",
            "multi_session_synthesis": "multi-session",
            "multi-session": "multi-session",
            "temp_reasoning_explicit": "temporal-reasoning",
            "temp_reasoning_implicit": "temporal-reasoning",
            "temporal-reasoning": "temporal-reasoning",
            "knowledge_update": "knowledge-update",
            "knowledge-update": "knowledge-update",
        }
        return mapping.get(t, t)

    def session_text(self, idx: int) -> str:
        """Render session at index as text."""
        session = self.haystack_sessions[idx] if idx < len(self.haystack_sessions) else []
        lines = []
        for turn in session:
            role = turn.get("role", "unknown")
            content = turn.get("content", "")
            lines.append(f"{role}: {content}")
        return "\n".join(lines) if lines else str(session)

    def turn_texts_and_labels(self) -> list[tuple[str, str, bool, str]]:
        """
        Extract all turns with their labels.
        Returns list of (text, session_id, has_answer, turn_id).
        turn_id format: session_id_turnIndex (1-indexed)
        """
        turns = []
        for sidx, (sid, session) in enumerate(
            zip(self.haystack_session_ids, self.haystack_sessions)
        ):
            for tidx, turn in enumerate(session):
                if turn.get("role") == "user":
                    text = turn.get("content", "")
                    if self._has_answer_labels:
                        has_answer = turn.get("has_answer", False)
                    else:
                        has_answer = sid in set(self.answer_session_ids)
                    turn_id = f"{sid}_{tidx + 1}"
                    turns.append((text, sid, has_answer, turn_id))
        return turns

    @property
    def has_turn_labels(self) -> bool:
        return self._has_answer_labels

    def evidence_turn_ids(self) -> set[str]:
        """Return set of turn IDs that are evidence."""
        turns = self.turn_texts_and_labels()
        return {tid for _, _, has_ans, tid in turns if has_ans}


# ──────────────────────────────────────────────────────────────────────
# Dataset Loading
# ──────────────────────────────────────────────────────────────────────


def load_dataset(path: str, max_cases: int = 0) -> list[EvalCase]:
    """Load LongMemEval dataset. Supports .json (array) and .jsonl."""
    filepath = Path(path)
    if not filepath.exists():
        print(f"ERROR: dataset not found: {path}")
        sys.exit(1)

    raw_cases = []
    if filepath.suffix == ".jsonl":
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    raw_cases.append(json.loads(line))
    else:
        with open(path) as f:
            data = json.load(f)
        if isinstance(data, list):
            raw_cases = data
        else:
            raw_cases = [data]

    cases = [EvalCase(c) for c in raw_cases]
    if max_cases > 0 and max_cases < len(cases):
        cases = cases[:max_cases]
    return cases


def dataset_stats(cases: list[EvalCase]) -> dict:
    """Return dataset statistics."""
    type_counts = defaultdict(int)
    abstention_count = 0
    turn_label_count = 0
    for c in cases:
        if c.is_abstention:
            abstention_count += 1
        else:
            type_counts[c.question_type_norm] += 1
        if c.has_turn_labels:
            turn_label_count += 1
    return {
        "total": len(cases),
        "non_abstention": len(cases) - abstention_count,
        "abstention": abstention_count,
        "by_type": dict(type_counts),
        "has_turn_labels": turn_label_count,
    }


# ──────────────────────────────────────────────────────────────────────
# KnowWhere API Client
# ──────────────────────────────────────────────────────────────────────


class KnowWhereClient:
    """Async HTTP client for KnowWhere API."""

    def __init__(self, base_url: str, api_key: str):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key

    def _headers(self) -> dict:
        return {
            "Authorization": f"Bearer {self.api_key}",
            "Content-Type": "application/json",
        }

    async def store_session_batch(
        self, session: aiohttp.ClientSession, run_id: str, case: EvalCase
    ) -> list[str]:
        """Store all haystack sessions for a case via batch endpoint. Returns stored UUIDs."""
        sessions_payload = []
        for idx, sid in enumerate(case.haystack_session_ids):
            session_date = (
                case.haystack_dates[idx]
                if idx < len(case.haystack_dates)
                else ""
            )
            content = case.session_text(idx)
            sessions_payload.append(
                {
                    "content": content,
                    "metadata": {
                        "benchmark": "longmemeval_eval",
                        "run_id": run_id,
                        "question_id": case.question_id,
                        "question_type": case.question_type,
                        "question_date": case.question_date,
                        "session_id": sid,
                        "benchmark_session_date": session_date,
                        "source_timestamp": session_date,
                    },
                    "memory_type": "episodic",
                    "source": "conversation",
                }
            )

        url = f"{self.base_url}/store_session_batch"
        payload = {"sessions": sessions_payload}
        async with session.post(url, headers=self._headers(), json=payload) as resp:
            if resp.status not in (200, 201):
                body = await resp.text()
                raise RuntimeError(
                    f"store_session_batch failed {resp.status}: {body[:300]}"
                )
            data = await resp.json()
            results = data.get("results", [])
            ids = []
            for entry in results:
                primary = entry.get("id")
                if primary:
                    ids.append(primary)
                for cid in entry.get("chunk_ids", []):
                    if cid != primary:
                        ids.append(cid)
            return ids

    async def store_session(
        self, session: aiohttp.ClientSession, run_id: str, case: EvalCase
    ) -> list[str]:
        """Store sessions one-by-one (fallback)."""
        ids = []
        for idx, sid in enumerate(case.haystack_session_ids):
            session_date = (
                case.haystack_dates[idx]
                if idx < len(case.haystack_dates)
                else ""
            )
            content = case.session_text(idx)
            url = f"{self.base_url}/store_session"
            payload = {
                "content": content,
                "metadata": {
                    "benchmark": "longmemeval_eval",
                    "run_id": run_id,
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "question_date": case.question_date,
                    "session_id": sid,
                    "benchmark_session_date": session_date,
                    "source_timestamp": session_date,
                },
                "memory_type": "episodic",
                "source": "conversation",
            }
            for attempt in range(1, 7):
                async with session.post(
                    url, headers=self._headers(), json=payload
                ) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        node_id = data.get("id")
                        if node_id:
                            ids.append(node_id)
                        break
                    if resp.status >= 500 and attempt < 6:
                        await asyncio.sleep(0.35 * attempt)
                        continue
                    body = await resp.text()
                    raise RuntimeError(
                        f"store_session failed {resp.status}: {body[:200]}"
                    )
        return ids

    async def retrieve(
        self,
        session: aiohttp.ClientSession,
        query: str,
        top_k: int = 80,
        max_depth: int = 3,
    ) -> list[dict]:
        """Retrieve from KnowWhere. Returns list of hit dicts with metadata."""
        url = f"{self.base_url}/retrieve_fractal"
        payload = {
            "query_text": query,
            "top_k": top_k,
            "max_depth": max_depth,
            "governance_enabled": True,
            "retrieval_profile": "full-fidelity",
            "include_debug": False,
        }
        async with session.post(url, headers=self._headers(), json=payload) as resp:
            if resp.status != 200:
                body = await resp.text()
                raise RuntimeError(f"retrieve failed {resp.status}: {body[:300]}")
            return await resp.json()

    async def batch_delete(
        self, session: aiohttp.ClientSession, ids: list[str]
    ) -> None:
        """Batch delete nodes."""
        if not ids:
            return
        url = f"{self.base_url}/nodes/batch_delete"
        async with session.post(
            url, headers=self._headers(), json={"ids": ids}
        ) as resp:
            if resp.status not in (200, 204):
                body = await resp.text()
                print(f"  WARN: batch_delete {resp.status}: {body[:200]}", flush=True)

    async def delete_node(
        self, session: aiohttp.ClientSession, node_id: str
    ) -> None:
        """Delete a single node."""
        url = f"{self.base_url}/nodes/{node_id}"
        async with session.delete(url, headers=self._headers()) as resp:
            if resp.status not in (200, 204):
                pass  # best-effort cleanup


# ──────────────────────────────────────────────────────────────────────
# Hit Extraction
# ──────────────────────────────────────────────────────────────────────


def extract_session_ids(hits: list[dict]) -> list[str]:
    """Extract and deduplicate session IDs from retrieval hits, preserving rank order."""
    seen = set()
    result = []
    for hit in hits:
        sid = hit.get("metadata", {}).get("session_id", "")
        if sid and sid not in seen:
            seen.add(sid)
            result.append(sid)
    return result


def extract_turn_ids(hits: list[dict]) -> list[str]:
    """Extract turn-precise IDs. Returns deduplicated IDs.

    With session-level storage, we estimate turn-hits by assuming
    a hit on a session covers all its turns.
    """
    seen = set()
    result = []
    for hit in hits:
        sid = hit.get("metadata", {}).get("session_id", "")
        if sid and sid not in seen:
            seen.add(sid)
            result.append(sid)
    return result


# ──────────────────────────────────────────────────────────────────────
# Evaluation Engine
# ──────────────────────────────────────────────────────────────────────


def evaluate_session_level(
    hit_session_ids: list[str],
    case: EvalCase,
    k_values: list[int],
) -> dict:
    """Compute session-level metrics (old + new)."""
    answer_sids = set(case.answer_session_ids)
    rankings = list(range(len(hit_session_ids)))

    new_metrics = compute_metrics(rankings, hit_session_ids, answer_sids, k_values)

    rank = None
    for i, sid in enumerate(hit_session_ids):
        if sid in answer_sids:
            rank = i + 1
            break

    return {
        "rank": rank,
        "top1": 1.0 if rank == 1 else 0.0,
        "recall_at_5": 1.0 if rank is not None and rank <= 5 else 0.0,
        "mrr": 1.0 / rank if rank else 0.0,
        "new": new_metrics,
    }


def evaluate_turn_level(
    hit_session_ids: list[str],
    case: EvalCase,
    k_values: list[int],
) -> Optional[dict]:
    """Compute turn-level metrics. Requires has_answer labels on turns."""
    if not case.has_turn_labels:
        return None

    turns = case.turn_texts_and_labels()
    evidence_tids = case.evidence_turn_ids()

    all_turn_ids = []
    for sid in hit_session_ids:
        for tidx in range(100):
            tid = f"{sid}_{tidx + 1}"
            exists = any(t[3] == tid for t in turns)
            if exists:
                all_turn_ids.append(tid)
            elif tidx > 20:
                break

    rankings = list(range(len(all_turn_ids)))
    return compute_metrics(rankings, all_turn_ids, evidence_tids, k_values)


# ──────────────────────────────────────────────────────────────────────
# Main Evaluation Loops
# ──────────────────────────────────────────────────────────────────────


async def run_percase_eval(
    client: KnowWhereClient,
    cases: list[EvalCase],
    top_k: int,
) -> list[dict]:
    """Per-case evaluation: store → retrieve → score → cleanup."""
    results = []
    total = len(cases)

    async with aiohttp.ClientSession() as http:
        for idx, case in enumerate(cases):
            run_id = f"lme-eval-{idx}-{case.question_id}"
            try:
                stored_ids = await client.store_session_batch(http, run_id, case)
                fetch_k = max(top_k * 4, 80)
                hits = await client.retrieve(http, case.question, fetch_k)
                owned = [
                    h
                    for h in hits
                    if h.get("metadata", {}).get("run_id") == run_id
                ]
                hit_sids = extract_session_ids(owned)
                session_eval = evaluate_session_level(hit_sids, case, K_VALUES)
                turn_eval = evaluate_turn_level(hit_sids, case, K_VALUES)

                result = {
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "question_type_norm": case.question_type_norm,
                    "is_abstention": case.is_abstention,
                    "hit_count": len(owned),
                    "session": session_eval,
                    "turn": turn_eval,
                    "error": None,
                }

                if stored_ids:
                    await client.batch_delete(http, stored_ids)

            except Exception as e:
                print(f"  FAIL case={case.question_id} idx={idx}: {e}", flush=True)
                result = {
                    "question_id": case.question_id,
                    "question_type": case.question_type,
                    "question_type_norm": case.question_type_norm,
                    "is_abstention": case.is_abstention,
                    "hit_count": 0,
                    "session": {"rank": None, "top1": 0.0, "recall_at_5": 0.0, "mrr": 0.0, "new": {}},
                    "turn": None,
                    "error": str(e),
                }

            results.append(result)
            status = "✓" if not result.get("error") else "✗"
            rank_str = f"rank={result['session']['rank']}" if result["session"]["rank"] else "no hit"
            print(
                f"  [{idx + 1:>3}/{total}] {status} {case.question_id} "
                f"{rank_str} abst={case.is_abstention}",
                flush=True,
            )

            if (idx + 1) % 10 == 0:
                non_abs = [r for r in results if not r["is_abstention"]]
                n = max(len(non_abs), 1)
                top1 = sum(r["session"]["top1"] for r in non_abs) / n
                print(
                    f"    → progress {idx + 1}/{total} top1={top1:.4f} "
                    f"recall@5={sum(r['session']['recall_at_5'] for r in non_abs)/n:.4f}",
                    flush=True,
                )

    return results


async def run_multisession_eval(
    client: KnowWhereClient,
    cases: list[EvalCase],
    top_k: int,
) -> list[dict]:
    """Multi-session evaluation: index ALL sessions once, query ALL questions, score."""
    results = []
    non_abs_cases = [c for c in cases if not c.is_abstention]
    total = len(cases)

    async with aiohttp.ClientSession() as http:
        # Phase 1: Index all sessions
        print("\n  ── PHASE 1: Indexing all sessions ──")
        all_stored_ids = []
        case_run_ids = {}
        for idx, case in enumerate(non_abs_cases):
            run_id = f"lme-ms-{idx}-{case.question_id}"
            case_run_ids[case.question_id] = run_id
            try:
                stored_ids = await client.store_session_batch(http, run_id, case)
                all_stored_ids.extend(stored_ids)
                if (idx + 1) % 10 == 0 or idx == len(non_abs_cases) - 1:
                    print(
                        f"    Indexed {idx + 1}/{len(non_abs_cases)} cases "
                        f"({len(all_stored_ids)} total nodes)",
                        flush=True,
                    )
            except Exception as e:
                print(f"    FAIL indexing case={case.question_id}: {e}", flush=True)
        print(f"    Indexed {len(all_stored_ids)} nodes total\n")

        # Phase 2: Query all cases
        print("  ── PHASE 2: Querying all cases ──")
        fetched_hits = {}
        for idx, case in enumerate(cases):
            try:
                fetch_k = max(top_k * 4, 80)
                hits = await client.retrieve(http, case.question, fetch_k)
                fetched_hits[case.question_id] = hits
                if (idx + 1) % 10 == 0 or idx == total - 1:
                    print(f"    Queried {idx + 1}/{total} cases", flush=True)
            except Exception as e:
                print(f"    FAIL query case={case.question_id}: {e}", flush=True)
                fetched_hits[case.question_id] = []

        # Phase 3: Score
        print("\n  ── PHASE 3: Scoring ──")
        for idx, case in enumerate(cases):
            hits = fetched_hits.get(case.question_id, [])
            hit_sids = extract_session_ids(hits)
            session_eval = evaluate_session_level(hit_sids, case, K_VALUES)
            turn_eval = evaluate_turn_level(hit_sids, case, K_VALUES)

            result = {
                "question_id": case.question_id,
                "question_type": case.question_type,
                "question_type_norm": case.question_type_norm,
                "is_abstention": case.is_abstention,
                "hit_count": len(hits),
                "session": session_eval,
                "turn": turn_eval,
                "error": None,
            }
            results.append(result)
            if (idx + 1) % 10 == 0 or idx == len(cases) - 1:
                non_abs_done = [r for r in results if not r["is_abstention"]]
                n_done = max(len(non_abs_done), 1)
                top1_done = sum(r["session"]["top1"] for r in non_abs_done) / n_done
                print(
                    f"    Scored {idx + 1}/{len(cases)} | "
                    f"top1={top1_done:.4f} "
                    f"recall@5={sum(r['session']['recall_at_5'] for r in non_abs_done)/n_done:.4f}",
                    flush=True,
                )

        # Phase 4: Cleanup
        print("\n  ── PHASE 4: Cleanup ──")
        if all_stored_ids:
            unique_ids = list(set(all_stored_ids))
            print(f"    Deleting {len(unique_ids)} unique nodes...", flush=True)
            try:
                await client.batch_delete(http, unique_ids)
                print("    Cleanup done", flush=True)
            except Exception as e:
                print(f"    Cleanup partial: {e}", flush=True)

    return results


# ──────────────────────────────────────────────────────────────────────
# CLI
# ──────────────────────────────────────────────────────────────────────


def parse_args():
    p = argparse.ArgumentParser(
        description="LongMemEval Evaluation — cross-session metrics"
    )
    p.add_argument(
        "--dataset",
        required=True,
        help="Path to LongMemEval dataset (.json or .jsonl)",
    )
    p.add_argument(
        "--mode",
        choices=["multi", "percase"],
        default="multi",
        help="Evaluation mode: multi (index all, query all) or percase (isolated per question)",
    )
    p.add_argument(
        "--base-url",
        default=os.environ.get("KNOWWHERE_BASE_URL", "http://127.0.0.1:3737"),
        help="KnowWhere API base URL",
    )
    p.add_argument(
        "--api-key",
        default=os.environ.get("KNOWWHERE_API_KEY", ""),
        help="KnowWhere API key",
    )
    p.add_argument(
        "--top-k",
        type=int,
        default=int(os.environ.get("KNOWWHERE_TOP_K", "20")),
        help="Top-K for old-style recall@k metric (default 20)",
    )
    p.add_argument(
        "--max-cases",
        type=int,
        default=int(os.environ.get("KNOWWHERE_MAX_CASES", "0")),
        help="Limit number of cases (0 = all)",
    )
    p.add_argument(
        "--output",
        default="",
        help="Output report path (JSON). Default: auto-generated.",
    )
    p.add_argument(
        "--old-only",
        action="store_true",
        help="Only compute old metrics (skip NDCG, turn-level)",
    )
    p.add_argument(
        "--stratified",
        default="",
        help="Path to stratified case list (JSON). Dict: type->[qids] or flat list of qids. "
        "Restricts evaluation to only these cases, grouped by strata for per-type reporting.",
    )
    p.add_argument(
        "--report-dir",
        default="",
        help="Directory for output reports. Default: current directory.",
    )
    p.add_argument(
        "--legacy-report",
        action="store_true",
        help="Emit v1-format JSON report (old metrics only, no new_session/new_turn in per_type). "
        "Use for backward compatibility with scripts that parse the report.",
    )
    return p.parse_args()


async def main():
    args = parse_args()

    if not args.api_key:
        print("ERROR: KNOWWHERE_API_KEY is required")
        sys.exit(1)

    # Load dataset
    print(f"Loading dataset: {args.dataset}")
    cases = load_dataset(args.dataset, 0)
    all_cases = cases

    # Apply stratified filter if provided
    strata = {}
    if args.stratified:
        try:
            allowed_ids, strata_def = load_stratified_filter(args.stratified)
        except StratifiedLoadError as e:
            print(f"ERROR: {e}")
            sys.exit(1)

        # Validate stratified IDs against dataset
        try:
            matched_ids, missing_ids = validate_stratified_ids(allowed_ids, all_cases)
        except StratifiedValidationError as e:
            print(f"ERROR: {e}")
            sys.exit(1)

        if missing_ids:
            print(
                f"  WARNING: {len(missing_ids)} stratified IDs not found in dataset: "
                f"{list(missing_ids)[:5]}{'...' if len(missing_ids) > 5 else ''}"
            )

        cases, applied_strata = apply_stratified_filter(all_cases, matched_ids, strata_def)
        print(f"  Stratified filter: {len(cases)}/{len(all_cases)} cases selected")
        for qtype, qids in applied_strata.items():
            print(f"    {qtype}: {len(qids)} cases")
        strata = applied_strata
    elif args.max_cases > 0 and args.max_cases < len(cases):
        cases = cases[:args.max_cases]

    stats = dataset_stats(cases)
    print(f"  Loaded {stats['total']} cases")
    print(f"  Types: {stats['by_type']}")
    print(f"  Abstention: {stats['abstention']}")
    if stats["has_turn_labels"]:
        print(f"  Turn labels: YES ({stats['has_turn_labels']} cases)")
    else:
        print(f"  Turn labels: NO (session-level only)")

    # Setup client
    client = KnowWhereClient(args.base_url, args.api_key)
    print(f"\nKnowWhere API: {args.base_url}")
    print(f"Mode: {args.mode}")
    print(f"Top-K (old): {args.top_k}")
    if args.legacy_report:
        print(f"Report format: v1 (legacy)")

    # Run evaluation
    t0 = time.monotonic()
    if args.mode == "multi":
        results = await run_multisession_eval(client, cases, args.top_k)
    else:
        results = await run_percase_eval(client, cases, args.top_k)
    elapsed = time.monotonic() - t0

    # Aggregate
    summary = aggregate_results(results, K_VALUES, args.top_k)

    # Print report
    mode_desc = "multi-session (index all, query all)" if args.mode == "multi" else "per-case (isolated)"
    if strata:
        mode_desc += f" | stratified ({len(strata)} strata)"
    print_report(summary, stats, mode_desc)
    print(f"\n  Evaluation took {elapsed:.1f}s")

    # Save report
    output = args.output
    if not output:
        ts = time.strftime("%Y%m%d_%H%M%S")
        output = f"longmemeval_report_{args.mode}_{ts}.json"
    if args.report_dir:
        output = os.path.join(args.report_dir, os.path.basename(output))
    save_report(summary, stats, args.mode, output, strata=strata, legacy=args.legacy_report)

    # Print errors if any
    errors = [r for r in results if r.get("error")]
    if errors:
        print(f"\n  WARNING: {len(errors)} cases had errors:")
        for e in errors[:5]:
            print(f"    {e['question_id']}: {e['error']}")
        if len(errors) > 5:
            print(f"    ... and {len(errors) - 5} more")


if __name__ == "__main__":
    asyncio.run(main())
