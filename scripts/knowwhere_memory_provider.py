"""KnowWhere memory provider for AMB (Agent Memory Benchmark).

Implements the MemoryProvider interface for KnowWhere v0.5.
Calls local KnowWhere server at KNOWWHERE_ENDPOINT.

Usage:
    uv run amb run --dataset personamem --domain 32k --memory knowwhere
"""

import os, time, requests
from pathlib import Path
from .base import MemoryProvider
from ..models import Document


class KnowWhereMemoryProvider(MemoryProvider):
    name = "knowwhere"
    description = (
        "KnowWhere — fractal memory with structured decision claims, "
        "cross-encoder reranking (bge-reranker-v2-m3), and entity tracking. "
        "Rust/Axum, PostgreSQL+pgvector, local deployment on M1."
    )
    kind = "local"
    provider = "knowwhere"
    link = "https://github.com/Jind0la/knowwhere"
    concurrency = 1  # Single-threaded for local server stability

    def __init__(self):
        self.endpoint = os.environ.get(
            "KNOWWHERE_ENDPOINT", "http://127.0.0.1:3737"
        )
        self.api_key = os.environ.get(
            "KNOWWHERE_API_KEY", "kw_testkey_12345"
        )
        self._headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {self.api_key}",
        }

    def ingest(self, documents: list[Document]) -> None:
        """Ingest documents into KnowWhere via /store_external."""
        total = len(documents)
        ingested = 0
        for doc in documents:
            try:
                payload = {
                    "content": doc.content,
                    "external_id": doc.id,
                    "source": "amb_benchmark",
                    "memory_type": "semantic",
                    "metadata": {
                        "user_id": doc.user_id or "",
                        "timestamp": doc.timestamp or "",
                    },
                }
                resp = requests.post(
                    f"{self.endpoint}/store_external",
                    json=payload,
                    headers=self._headers,
                    timeout=30,
                )
                if resp.status_code in (200, 201):
                    ingested += 1
                else:
                    print(
                        f"  [knowwhere] ingest {doc.id[:20]}: "
                        f"HTTP {resp.status_code} {resp.text[:100]}"
                    )
            except Exception as e:
                print(f"  [knowwhere] ingest {doc.id[:20]}: ERROR {e}")
        print(f"  [knowwhere] ingested {ingested}/{total} documents")

    def retrieve(
        self,
        query: str,
        k: int = 10,
        user_id: str | None = None,
        query_timestamp: str | None = None,
    ) -> tuple[list[Document], dict | None]:
        """Retrieve top-k documents from KnowWhere via /retrieve_fractal."""
        payload = {
            "query_text": query,
            "top_k": k,
        }
        try:
            resp = requests.post(
                f"{self.endpoint}/retrieve_fractal",
                json=payload,
                headers=self._headers,
                timeout=30,
            )
            resp.raise_for_status()
            data = resp.json()
        except Exception as e:
            print(f"  [knowwhere] retrieve ERROR: {e}")
            return [], None

        # Filter out meta-nodes (reflect, memory-context)
        nodes = [
            n for n in data
            if not (n.get("content") or "").strip().startswith("<knowwhere_")
        ]

        documents = []
        for node in nodes[:k]:
            content = node.get("content") or ""
            memory_type = node.get("memory_type", "")
            doc_id = node.get("id", "")

            # Use the raw node ID for scoring, like other providers do
            documents.append(
                Document(
                    id=doc_id,
                    content=content,
                    user_id=user_id,
                )
            )

        return documents, {"raw_nodes": len(nodes), "returned_docs": len(documents)}

    def cleanup(self) -> None:
        """Optional cleanup after benchmark — nothing to do."""
        pass

    def prepare(
        self,
        store_dir: Path,
        unit_ids: set[str] | None = None,
        reset: bool = True,
    ) -> None:
        """Verify KnowWhere server is reachable before starting."""
        try:
            resp = requests.get(
                f"{self.endpoint}/health",
                headers=self._headers,
                timeout=5,
            )
            health = resp.json()
            print(
                f"  [knowwhere] server healthy — {health.get('node_count', '?')} nodes"
            )
        except Exception as e:
            print(f"  [knowwhere] WARNING: server not reachable: {e}")


# Register with AMB
def register():
    """Called by AMB to register this provider."""
    from . import REGISTRY
    REGISTRY["knowwhere"] = KnowWhereMemoryProvider
