"""KnowWhere LangChain integration – chat history backed by fractal retrieval."""

from __future__ import annotations

from typing import Any

from langchain_core.chat_history import BaseChatMessageHistory
from langchain_core.messages import BaseMessage, HumanMessage

from knowwhere.client import KnowWhereClient


class KnowWhereMemory(BaseChatMessageHistory):
    """LangChain-compatible chat history backed by KnowWhere fractal retrieval.

    Stores every message as a session node and retrieves relevant
    context via embedding similarity + fractal zooming.
    """

    def __init__(
        self,
        client: KnowWhereClient,
        top_k: int = 5,
        max_depth: int = 3,
    ) -> None:
        self.client = client
        self.top_k = top_k
        self.max_depth = max_depth
        self._messages: list[BaseMessage] = []

    @property
    def messages(self) -> list[BaseMessage]:
        return list(self._messages)

    def add_message(self, message: BaseMessage) -> None:
        self._messages.append(message)
        content = message.content if isinstance(message.content, str) else str(message.content)
        role = "user" if isinstance(message, HumanMessage) else "assistant"
        metadata = {"source": "langchain", "role": role}
        if role == "assistant":
            metadata["derivation"] = "assistant_output"
            metadata["retrieval_visibility"] = "internal"
            metadata["trust_tier"] = "derived"
        else:
            metadata["derivation"] = "user_input"
            metadata["trust_tier"] = "primary"
        try:
            self.client.store_session(
                content=f"{role.upper()}: {content}",
                metadata=metadata,
            )
        except Exception as exc:
            print(f"[KnowWhereMemory] store error: {exc}")

    def clear(self) -> None:
        self._messages = []

    def search_context(self, query: str) -> list[dict[str, Any]]:
        """Retrieve relevant past context for a query via fractal search."""
        embed_resp = self.client.embed(query)
        return self.client.retrieve_fractal(
            embed_resp.vector,
            top_k=self.top_k,
            max_depth=self.max_depth,
        )

    def get_context_string(self, query: str) -> str:
        """Retrieve relevant past context as formatted string."""
        try:
            nodes = self.search_context(query)
            parts: list[str] = []
            for node in nodes:
                if node.get("content"):
                    parts.append(node["content"])
                elif node.get("original_pointer"):
                    parts.append(f"[pointer: {node['original_pointer']}]")
            return "\n---\n".join(parts)
        except Exception as exc:
            print(f"[KnowWhereMemory] search error: {exc}")
            return ""
