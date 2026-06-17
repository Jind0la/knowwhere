#!/usr/bin/env python3
"""Compare DeepSeek vs Ollama summarizer output quality.

Usage: python3 scripts/compare_summarizers.py [--samples 5]
"""

import json, os, sys, time

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")
DEEPSEEK_URL = "https://api.deepseek.com/v1/chat/completions"
DEEPSEEK_MODEL = "deepseek-chat"

SYSTEM_PROMPT = "You are a concise summarizer. Output exactly one sentence."
USER_PROMPT_TEMPLATE = """Summarize in ONE sentence (≤25 words). If this is about a person: state their key preferences, facts, or life changes. If this is technical: state the decision made and the reason. Be specific — name exact things (technologies, activities, preferences). No preamble.

{}"""

# Sample texts from KnowWhere state.json (real content)
SAMPLE_TEXTS = [
    "user: I've been thinking about my career and purpose lately, and I'm feeling a bit lost. I've been doing software engineering for 5 years but I'm not sure if this is what I want long-term. I've been exploring product management and it seems interesting.",
    "user: I like the revised schedule, but I think I should also block out time for my workouts and walks. Can you add 1 hour every morning at 7am for exercise?",
    "user: What measures have been taken to address the negative impacts of the fishing industry on marine ecosystems? I'm curious about fishing quotas specifically.",
    "user: Let's use Rust for the backend. We'll go with Axum for the web framework and SQLx for database access. No ORM — raw SQL is fine.",
    "user: I prefer working in the mornings. My peak productivity is 6am-11am. Afternoons are for meetings and admin stuff.",
    "user: The consolidation pipeline should run every 4 hours, not every hour. We're burning too much compute on it.",
    "user: My favorite coffee is a flat white with oat milk. I usually get it from the café on Dorstener Straße.",
    "user: I've decided to switch from VS Code to Cursor. The AI integration is just better for my workflow.",
]


def get_api_key(env_var, zshrc_key):
    key = os.environ.get(env_var, "")
    if key:
        return key
    zshrc = os.path.expanduser("~/.zshrc")
    if os.path.exists(zshrc):
        with open(zshrc) as f:
            for line in f:
                if zshrc_key in line and "export" in line:
                    parts = line.split("=", 1)
                    if len(parts) == 2:
                        return parts[1].strip().strip('"').strip("'")
    return ""


def summarize_ollama(session, text):
    """Summarize via Ollama LocalSummarizer-style API call."""
    import requests
    resp = session.post(
        f"{OLLAMA_URL}/api/generate",
        json={
            "model": os.environ.get("OLLAMA_SUMMARIZER_MODEL", "qwen2.5:3b"),
            "system": SYSTEM_PROMPT,
            "prompt": USER_PROMPT_TEMPLATE.format(text),
            "stream": False,
            "options": {"temperature": 0.0, "num_predict": 50},
        },
        timeout=120,
    )
    if resp.status_code != 200:
        return f"ERROR: HTTP {resp.status_code}"
    return resp.json().get("response", "").strip()


def summarize_deepseek(session, api_key, text):
    """Summarize via DeepSeek API."""
    import requests
    resp = session.post(
        DEEPSEEK_URL,
        headers={"Authorization": f"Bearer {api_key}"},
        json={
            "model": DEEPSEEK_MODEL,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": USER_PROMPT_TEMPLATE.format(text)},
            ],
            "temperature": 0.0,
            "max_tokens": 50,
        },
        timeout=120,
    )
    if resp.status_code != 200:
        return f"ERROR: HTTP {resp.status_code}"
    data = resp.json()
    return (
        data.get("choices", [{}])[0]
        .get("message", {})
        .get("content", "")
        .strip()
        .strip('"')
        .strip("'")
    )


def main():
    import requests

    deepseek_key = get_api_key("DEEPSEEK_API_KEY", "DEEPSEEK_API_KEY")
    if not deepseek_key:
        print("WARNING: DEEPSEEK_API_KEY not found — skipping DeepSeek comparison")
        deepseek_key = None

    # Check Ollama
    session = requests.Session()
    try:
        r = session.get(f"{OLLAMA_URL}/api/tags", timeout=5)
        ollama_available = r.status_code == 200
    except Exception:
        ollama_available = False

    if not ollama_available:
        print("WARNING: Ollama not reachable — starting it...")
        os.system("open -a Ollama 2>/dev/null")
        time.sleep(5)
        try:
            r = session.get(f"{OLLAMA_URL}/api/tags", timeout=5)
            ollama_available = r.status_code == 200
        except Exception:
            pass

    if not ollama_available:
        print("ERROR: Ollama not available. Start it first.")
        print("  ollama serve")
        sys.exit(1)

    # Ensure model is loaded
    model = os.environ.get("OLLAMA_SUMMARIZER_MODEL", "qwen2.5:3b")
    print(f"Pulling model {model} if needed...")
    os.system(f"ollama pull {model} 2>/dev/null")

    print(f"\n{'='*70}")
    print(f"Summarizer Comparison: Ollama ({model}) vs DeepSeek (deepseek-chat)")
    print(f"{'='*70}")

    for i, text in enumerate(SAMPLE_TEXTS):
        print(f"\n--- Sample {i+1} ---")
        print(f"Input: {text[:100]}...")

        # Ollama
        t0 = time.time()
        ollama_output = summarize_ollama(session, text)
        ollama_time = time.time() - t0
        print(f"  Ollama    ({ollama_time:.1f}s): {ollama_output}")

        # DeepSeek
        if deepseek_key:
            t0 = time.time()
            deepseek_output = summarize_deepseek(session, deepseek_key, text)
            deepseek_time = time.time() - t0
            print(f"  DeepSeek  ({deepseek_time:.1f}s): {deepseek_output}")
        else:
            deepseek_output = "N/A"
            deepseek_time = 0
            print(f"  DeepSeek: SKIPPED (no API key)")

    print(f"\n{'='*70}")
    print("Done. Compare outputs above for quality assessment.")
    print("Key questions:")
    print("  1. Are summaries equally concise?")
    print("  2. Does DeepSeek hallucinate facts not in the input?")
    print("  3. Is the quality difference worth the API cost?")


if __name__ == "__main__":
    main()
