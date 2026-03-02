#!/usr/bin/env python3
"""KnowWhere SDK + LangChain integration example.

Prerequisites:
    1. Start the KnowWhere server:  cargo run
    2. Install the SDK:             pip install -e sdk/python
    3. Run this example:            python sdk/python/examples/langchain_example.py
"""

from knowwhere import KnowWhereClient
from knowwhere.langchain import KnowWhereMemory
from langchain_core.messages import HumanMessage, AIMessage


def main() -> None:
    client = KnowWhereClient(base_url="http://localhost:3737")

    # --- 1. Health Check ---
    print("=== Health Check ===")
    health = client.health()
    print(f"Status: {health.status}, Nodes: {health.node_count}")

    # --- 2. Store Session (full content + embedding) ---
    print("\n=== Store Sessions ===")
    s1 = client.store_session(
        content="Die App soll anonym sein, kein Login nötig",
        metadata={"project": "knowwhere", "topic": "architecture"},
    )
    print(f"Session 1: {s1.id} – {s1.message}")

    s2 = client.store_session(
        content="Wir nutzen Rust mit Axum für das Backend",
        metadata={"project": "knowwhere", "topic": "tech-stack"},
    )
    print(f"Session 2: {s2.id} – {s2.message}")

    s3 = client.store_session(
        content="Pointer-First bedeutet: externe Daten nie speichern, nur referenzieren",
        metadata={"project": "knowwhere", "topic": "principles"},
    )
    print(f"Session 3: {s3.id} – {s3.message}")

    # --- 3. Store External (Pointer-First: no raw data) ---
    print("\n=== Store External (Pointer-First) ===")
    ext = client.store_external(
        pointer="frigate://camera/front/2026-02-26T20:15.jpg",
        metadata={"source": "frigate", "camera": "front_door"},
        multimodal={
            "type": "Image",
            "pointer": "frigate://camera/front/2026-02-26T20:15.jpg",
            "embedding": [0.1, 0.2, 0.3, 0.4],
        },
    )
    print(f"External: {ext.id} – {ext.message}")

    # --- 4. Retrieve by ID ---
    print("\n=== Retrieve Node ===")
    node = client.retrieve(s1.id)
    print(f"Node content: {node.get('content', 'N/A')}")
    print(f"Node pointer: {node.get('original_pointer', 'None (session node)')}")

    # --- 5. Embed + Fractal Retrieve ---
    print("\n=== Fractal Search ===")
    embed_resp = client.embed("Welcher Tech-Stack wird verwendet?")
    print(f"Embedding dimension: {embed_resp.dimension} (provider: {embed_resp.provider})")

    results = client.retrieve_fractal(embed_resp.vector, top_k=3)
    for i, r in enumerate(results):
        label = r.get("content") or f"[pointer: {r.get('original_pointer', '?')}]"
        print(f"  #{i+1}: {label[:80]}")

    # --- 6. Dream Status ---
    print("\n=== Dream Status ===")
    dream = client.dream_status()
    print(f"Last run: {dream.last_run or 'never'}, Cycles: {dream.cycle_count}")

    # --- 7. LangChain Memory Integration ---
    print("\n=== LangChain Memory ===")
    memory = KnowWhereMemory(client=client)

    memory.add_message(HumanMessage(content="Was ist Pointer-First?"))
    memory.add_message(AIMessage(
        content="Pointer-First bedeutet: externe Daten werden nie gespeichert, "
                "nur als Pointer referenziert. Sessions werden vollständig gespeichert."
    ))
    print(f"Messages in memory: {len(memory.messages)}")

    context = memory.get_context_string("Pointer-First Prinzip")
    print(f"Retrieved context:\n{context[:200]}...")

    # --- 8. Final Health ---
    print("\n=== Final Health ===")
    health = client.health()
    print(f"Total nodes: {health.node_count}")
    print("\nDone! KnowWhere SDK + LangChain integration works.")


if __name__ == "__main__":
    main()
