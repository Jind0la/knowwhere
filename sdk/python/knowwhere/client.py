"""KnowWhere Python SDK – Client and LangChain Memory integration."""

from __future__ import annotations

from typing import Any, Optional

import requests
from pydantic import BaseModel, Field
from langchain_core.chat_history import BaseChatMessageHistory
from langchain_core.messages import BaseMessage, HumanMessage, AIMessage


class KnowWhereError(Exception):
    """Raised when a KnowWhere API call fails."""

    def __init__(self, status_code: int, detail: str) -> None:
        self.status_code = status_code
        self.detail = detail
        super().__init__(f"KnowWhere API error {status_code}: {detail}")


class StoreNodeResponse(BaseModel):
    id: str
    message: str


class HealthResponse(BaseModel):
    status: str
    node_count: int


class EmbedResponse(BaseModel):
    vector: list[float]
    dimension: int
    provider: str


class DreamStatusResponse(BaseModel):
    last_run: Optional[str] = None
    cycle_count: int = 0


class KnowWhereClient:
    """Synchronous HTTP client for the KnowWhere memory service."""

    def __init__(
        self,
        base_url: str = "http://localhost:3000",
        api_key: Optional[str] = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout = timeout
        self._session = requests.Session()
        if api_key:
            self._session.headers["Authorization"] = f"Bearer {api_key}"

    def _request(
        self,
        method: str,
        path: str,
        json: Optional[dict] = None,
    ) -> requests.Response:
        url = f"{self.base_url}{path}"
        resp = self._session.request(
            method, url, json=json, timeout=self.timeout,
        )
        if resp.status_code >= 400:
            raise KnowWhereError(resp.status_code, resp.text)
        return resp

    def health(self) -> HealthResponse:
        resp = self._request("GET", "/health")
        return HealthResponse(**resp.json())

    def embed(self, text: str) -> EmbedResponse:
        resp = self._request("POST", "/embed", json={"text": text})
        return EmbedResponse(**resp.json())

    def store_session(
        self,
        content: str,
        metadata: Optional[dict[str, Any]] = None,
        vector: Optional[list[float]] = None,
    ) -> StoreNodeResponse:
        payload: dict[str, Any] = {"content": content}
        if metadata:
            payload["metadata"] = metadata
        if vector:
            payload["vector"] = vector
        resp = self._request("POST", "/store_session", json=payload)
        return StoreNodeResponse(**resp.json())

    def store_external(
        self,
        pointer: str,
        metadata: Optional[dict[str, Any]] = None,
        vector: Optional[list[float]] = None,
        multimodal: Optional[dict[str, Any]] = None,
    ) -> StoreNodeResponse:
        """Pointer-First: stores only a pointer, never raw data."""
        payload: dict[str, Any] = {"pointer": pointer}
        if metadata:
            payload["metadata"] = metadata
        if vector:
            payload["vector"] = vector
        if multimodal:
            payload["multimodal"] = multimodal
        resp = self._request("POST", "/store_external", json=payload)
        return StoreNodeResponse(**resp.json())

    def retrieve(self, node_id: str) -> dict[str, Any]:
        resp = self._request("GET", f"/retrieve/{node_id}")
        return resp.json()

    def retrieve_fractal(
        self,
        query_vector: list[float],
        top_k: int = 5,
        max_depth: int = 3,
    ) -> list[dict[str, Any]]:
        payload = {
            "query_vector": query_vector,
            "top_k": top_k,
            "max_depth": max_depth,
        }
        resp = self._request("POST", "/retrieve_fractal", json=payload)
        return resp.json()

    def dream_status(self) -> DreamStatusResponse:
        resp = self._request("GET", "/dream/status")
        return DreamStatusResponse(**resp.json())

    def recent_nodes(self, limit: int = 20) -> list[dict[str, Any]]:
        resp = self._request("GET", f"/nodes/recent?limit={limit}")
        return resp.json()


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
        role = "Human" if isinstance(message, HumanMessage) else "AI"
        try:
            self.client.store_session(
                content=f"{role}: {content}",
                metadata={"source": "langchain", "role": role.lower()},
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
