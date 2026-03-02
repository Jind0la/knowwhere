"""KnowWhere Python SDK – pointer-first fractal memory for AI agents."""

from knowwhere.client import KnowWhereClient, KnowWhereError

__all__ = ["KnowWhereClient", "KnowWhereError"]
__version__ = "0.1.0"


def __getattr__(name: str):
    """Lazy-load KnowWhereMemory so langchain_core is only imported when needed."""
    if name == "KnowWhereMemory":
        from knowwhere.langchain import KnowWhereMemory
        return KnowWhereMemory
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
