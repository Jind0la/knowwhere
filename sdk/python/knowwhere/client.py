"""KnowWhere Python SDK – HTTP client for the fractal memory service."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional

import requests


class KnowWhereError(Exception):
    """Raised when a KnowWhere API call fails."""

    def __init__(self, status_code: int, detail: str) -> None:
        self.status_code = status_code
        self.detail = detail
        super().__init__(f"KnowWhere API error {status_code}: {detail}")


@dataclass
class StoreNodeResponse:
    id: str
    message: str


@dataclass
class HealthResponse:
    status: str
    node_count: int


@dataclass
class EmbedResponse:
    vector: list[float]
    dimension: int
    provider: str


@dataclass
class DreamStatusResponse:
    last_run: Optional[str] = None
    cycle_count: int = 0


def _sanitize_error(text: str, status_code: int) -> str:
    """Replace HTML error pages with a clear message, truncate long text."""
    stripped = text.lstrip()
    if stripped.startswith(("<!DOCTYPE", "<html", "<HTML")):
        return (
            f"Server returned HTML instead of JSON (status {status_code},"
            " wrong port or server not running?)"
        )
    if len(text) > 500:
        return text[:500] + "..."
    return text


class KnowWhereClient:
    """Synchronous HTTP client for the KnowWhere memory service."""

    def __init__(
        self,
        base_url: str = "http://localhost:3737",
        api_key: Optional[str] = None,
        timeout: float = 10.0,
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
            raise KnowWhereError(
                resp.status_code,
                _sanitize_error(resp.text, resp.status_code),
            )
        return resp

    def is_alive(self, timeout: float = 2.0) -> bool:
        """Quick health probe – returns True if the server responds 200."""
        try:
            resp = self._session.get(
                f"{self.base_url}/health", timeout=timeout,
            )
            return resp.status_code == 200
        except Exception:
            return False

    def health(self) -> HealthResponse:
        resp = self._request("GET", "/health")
        data = resp.json()
        return HealthResponse(status=data["status"], node_count=data["node_count"])

    def embed(self, text: str) -> EmbedResponse:
        resp = self._request("POST", "/embed", json={"text": text})
        data = resp.json()
        return EmbedResponse(
            vector=data["vector"],
            dimension=data["dimension"],
            provider=data["provider"],
        )

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
        data = resp.json()
        return StoreNodeResponse(id=data["id"], message=data["message"])

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
        data = resp.json()
        return StoreNodeResponse(id=data["id"], message=data["message"])

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
        data = resp.json()
        return DreamStatusResponse(
            last_run=data.get("last_run"),
            cycle_count=data.get("cycle_count", 0),
        )

    def recent_nodes(self, limit: int = 20) -> list[dict[str, Any]]:
        resp = self._request("GET", f"/nodes/recent?limit={limit}")
        return resp.json()
