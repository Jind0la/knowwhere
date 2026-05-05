#!/usr/bin/env python3
"""Spike: Compare prompt variants for structured claims extraction from consolidation content.

Tests 3 variants × 5 inputs against llama3.2 via Ollama.
Measures: claims_extracted, specificity_score, format_compliance.
"""

import json, time, urllib.request, sys

OLLAMA = "http://127.0.0.1:11434"

# ─── Test Cases ────────────────────────────────────────────────
TEST_CASES = [
    {
        "id": "multi-decision",
        "content": """[user] Docker for KnowWhere deployment keeps crashing on my M1 Mac. The VM overhead is 4+ GB RAM just for Docker Desktop.
[assistant] That's a known issue with Docker on Apple Silicon. The LinuxKit VM adds significant overhead.
[user] What are the alternatives?
[assistant] Three options: 1) Colima — lighter VM, 2) Podman — daemonless, 3) Native macOS binary with launchd.
[user] Let's go native. Docker was causing too many problems anyway — container instability, port conflicts, the works.
[assistant] Decision noted: Kill Docker, move to native macOS deployment with launchd auto-start. This frees ~7.6 GB disk space and eliminates the LinuxKit VM overhead entirely.
[user] Also, we should switch the embedding model. OpenAI embeddings are too expensive and slow — $0.13 per 1K tokens.
[assistant] What model did you have in mind?
[user] nomic-embed-text-v2-moe. It's 768-dim, runs on Ollama locally, and the MTEB scores are competitive. Half the storage of OpenAI's 1536-dim.
[assistant] We'll need to re-embed all existing nodes. That's about 2,400 nodes.
[user] Do it. Add a POST /nodes/reembed_all endpoint.
[assistant] Got it. Two decisions: 1) Kill Docker → native launchd, 2) OpenAI → nomic-embed-text-v2-moe with reembed_all endpoint."""
    },
    {
        "id": "bug-investigation",
        "content": """[user] The retrieval scores are terrible. Decision queries like "why did we kill Docker?" return zero relevant results.
[assistant] Let me trace the scoring pipeline. The issue might be in trust_tier().
[user] Check it.
[assistant] Found it. In src/memory/fractal_node.rs:231, trust_tier() penalizes ALL Consolidation-source nodes as TRUST_DERIVED (0.88×). That includes Decision nodes. Meanwhile, raw conversation turns get TRUST_PRIMARY (1.18×).
[user] So our most valuable content is being scored lower than random chat?
[assistant] Exactly. The fix is two-fold: 1) Decision nodes should return TRUST_PRIMARY before the Consolidation check, 2) Add a memory_type_multiplier() — Decision ×1.5, Episodic ×0.85.
[user] What's the combined effect?
[assistant] Decision node = 1.18 × 1.5 = 1.77× vs episodic = 1.18 × 0.85 = 1.00×. A 77% relative advantage. But there's a catch — MemoryType::Decision exists everywhere EXCEPT in MemoryType::parse(). All Decision nodes were being silently converted to Episodic on store.
[user] So the scoring fix was dead code?
[assistant] For 24 hours, yes. Fixed now — parse() recognizes 'decision'."""
    },
    {
        "id": "architecture-planning",
        "content": """[user] We need to decide: should KnowWhere use a single flat vector store or the fractal L2→L1→L0 hierarchy?
[assistant] The fractal tree gives us logarithmic search — at 2,995 nodes it's indistinguishable from flat, but at 2,995,000 nodes it's essential.
[user] But the code has TWO tree representations: FractalNode.children (Vec<FractalNode>) and FractalNode.children_tier_ids (Vec<Uuid>). Only one is populated by consolidation, the other by retrieval. They never meet.
[assistant] That's the bridge gap. expand_fractal() on the StorageBackend trait bridges them — it reads children_tier_ids and loads the actual nodes.
[user] Is that the right fix or a band-aid?
[assistant] It's the correct architecture. The tier_ids are stored pointers (cheap to store), expand_fractal() resolves them on retrieval (lazy loading). This follows the Pointer-First principle.
[user] OK, wire it into the HTTP endpoint. POST /retrieve_fractal should call expand_fractal() before returning.
[assistant] Done. Also adding Bridge-Expansion: when a node has no children_tier_ids, traverse parent_tier_id backward and expand siblings. This gives +33% yield at top_k=3.
[user] Good. Decision: fractal zoom goes into the HTTP path, with bridge-expansion fallback."""
    },
    {
        "id": "claims-extraction-design",
        "content": """[user] The consolidation summaries are missing decisions. When I ask "why did we do X?", the retrieval returns narrative prose, not the actual decision rationale.
[assistant] The root cause is that consolidation produces prose summaries, not structured claims. The LLM writes "Key decisions made and WHY: an endpoint was created" — everything is buried in one sentence.
[user] How do we fix this?
[assistant] We need structured claims extraction. After the narrative summary, force the LLM to output a machine-readable claims block: ---CLAIMS--- with claim/reason pairs. Each claim becomes a separate Decision node with its own embedding.
[user] What's the risk?
[assistant] Small models like llama3.2 might ignore the structured format. We should consider Ollama's native JSON schema support as a fallback — it constrains token generation via GBNF grammar, so the model physically cannot produce invalid output.
[user] Try the ---CLAIMS--- approach first. If it fails, switch to JSON schema.
[assistant] Decision: Implement structured claims extraction with ---CLAIMS--- format for Phase 1, with JSON schema as fallback. Each claim gets a MemoryType::Decision node linked to its L1 parent via parent_tier_id."""
    },
    {
        "id": "simple-chat",
        "content": """[user] What's the weather like?
[assistant] I don't have access to weather data, but you can check your local weather app.
[user] OK. How many nodes are in KnowWhere right now?
[assistant] Currently 2,296 nodes based on the health endpoint.
[user] Thanks. That's all I needed."""
    },
]

# ─── Variant A: Text-based ---CLAIMS--- (current) ──────────────
def variant_a_prompt(content: str) -> dict:
    return {
        "model": "llama3.2",
        "messages": [
            {"role": "system", "content": "You are a concise summarizer."},
            {"role": "user", "content": (
                f"Summarize in 2-3 sentences (max 200 words). "
                f"Sentence 1: key decisions made and WHY — be specific, name trade-offs. "
                f"Sentence 2: important facts. "
                f"Sentence 3: entities and timestamps. "
                f"No preamble.\n\n"
                f"After your summary, add a claims block:\n"
                f"---CLAIMS---\n"
                f"- claim: <what was decided or key takeaway>\n"
                f"  reason: <why — rationale, constraint, or evidence>\n"
                f"- claim: <next claim, if any>\n"
                f"  reason: <why>\n"
                f"---END---\n\n"
                f"IMPORTANT: Extract at least ONE claim. "
                f"A claim can be: a decision, a realization, a key finding, "
                f"or a stated preference. If truly nothing is decided, "
                f"extract the single most important factual assertion as a claim.\n\n"
                f"{content}"
            )},
        ],
        "stream": False,
        "options": {"temperature": 0.0, "seed": 42, "num_predict": 300},
    }


# ─── Variant B: JSON Schema Structured Output ──────────────────
CLAIMS_SCHEMA = {
    "type": "object",
    "properties": {
        "summary": {
            "type": "string",
            "description": "2-3 sentence narrative summary of the conversation"
        },
        "claims": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "claim": {"type": "string", "description": "What was decided, realized, or established"},
                    "reason": {"type": "string", "description": "Why — rationale, evidence, constraint, or trade-off"}
                },
                "required": ["claim", "reason"]
            },
            "description": "Structured claims extracted from the conversation"
        }
    },
    "required": ["summary", "claims"]
}

def variant_b_prompt(content: str) -> dict:
    return {
        "model": "llama3.2",
        "messages": [
            {"role": "system", "content": (
                "You are a forensic decision extractor. Your job is to identify "
                "explicit decisions, realizations, and key findings in conversations. "
                "Always extract at least 2 claims. Each claim must be a specific, "
                "concrete statement — never generic. Output valid JSON only."
            )},
            {"role": "user", "content": (
                f"Extract structured decisions and key facts from this conversation. "
                f"Return a JSON object with a summary and an array of claims. "
                f"Each claim must have a 'claim' (what was decided/established) "
                f"and a 'reason' (why — rationale, evidence, or trade-off). "
                f"Extract at least 2 claims. Be specific — name entities, technologies, "
                f"and concrete decisions.\n\n"
                f"{content}"
            )},
        ],
        "stream": False,
        "options": {"temperature": 0.0, "seed": 42, "num_predict": 400},
        "format": CLAIMS_SCHEMA,
    }


# ─── Variant C: JSON Schema + Few-Shot Examples ────────────────
def variant_c_prompt(content: str) -> dict:
    return {
        "model": "llama3.2",
        "messages": [
            {"role": "system", "content": (
                "You are a forensic decision extractor. Your job is to identify "
                "explicit decisions, realizations, and key findings in conversations. "
                "Always extract at least 2 claims. Each claim must be a specific, "
                "concrete statement — never generic. Output valid JSON only."
            )},
            {"role": "user", "content": (
                f"EXAMPLE 1:\n"
                f"Conversation: The team discussed switching from Docker to native "
                f"deployment because Docker used 4+ GB RAM on M1 and had container "
                f"instability. They decided to use launchd instead.\n"
                f"Output: {{\"summary\": \"Team decided to switch from Docker to native "
                f"macOS deployment with launchd due to 4GB+ RAM overhead and container "
                f"instability on M1.\", \"claims\": [{{\"claim\": \"Docker deployment was "
                f"replaced with native macOS launchd\", \"reason\": \"Docker Desktop used "
                f"4+ GB RAM via LinuxKit VM on M1 and had container instability\"}}]}}\n\n"
                f"EXAMPLE 2:\n"
                f"Conversation: The team found that Decision nodes were being scored "
                f"lower than Episodic nodes because trust_tier() penalized all "
                f"Consolidation-source nodes. They fixed it by making Decision nodes "
                f"TRUST_PRIMARY and adding a 1.5x memory_type multiplier.\n"
                f"Output: {{\"summary\": \"Team discovered and fixed a retrieval scoring "
                f"bug where Decision nodes were penalized vs Episodic nodes.\", "
                f"\"claims\": [{{\"claim\": \"Decision nodes now return TRUST_PRIMARY "
                f"before Consolidation check\", \"reason\": \"They were being penalized "
                f"as TRUST_DERIVED (0.88x) which ranked them below raw chat (1.18x)\"}}, "
                f"{{\"claim\": \"Added memory_type_multiplier with Decision at 1.5x\", "
                f"\"reason\": \"To give Decision nodes a 77% relative advantage over "
                f"Episodic (1.18 x 1.5 = 1.77x vs 1.18 x 0.85 = 1.00x)\"}}]}}\n\n"
                f"Now extract from this conversation:\n\n"
                f"{content}"
            )},
        ],
        "stream": False,
        "options": {"temperature": 0.0, "seed": 42, "num_predict": 500},
        "format": CLAIMS_SCHEMA,
    }


# ─── Evaluation ────────────────────────────────────────────────
def call_ollama(payload: dict) -> dict:
    """Call Ollama /api/chat and return parsed response."""
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{OLLAMA}/api/chat",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = json.loads(resp.read().decode("utf-8"))
            return {"ok": True, "latency": time.perf_counter() - started, "raw": raw}
    except Exception as e:
        return {"ok": False, "error": str(e), "latency": time.perf_counter() - started}


def is_specific_claim(claim: str) -> bool:
    """A claim is 'specific' if it names a concrete decision, not a generic escape."""
    lower = claim.lower().strip()
    # Escape hatches
    if "no decision" in lower:
        return False
    if "not provide" in lower:
        return False
    if "no explicit" in lower:
        return False
    if lower in ("", "none", "n/a"):
        return False
    # Must have substance: at least a verb + noun
    words = claim.split()
    if len(words) < 4:
        return False
    return True


def parse_variant_a(raw_text: str) -> dict:
    """Parse text-based ---CLAIMS--- block."""
    summary = ""
    claims = []
    
    if "---CLAIMS---" in raw_text:
        parts = raw_text.split("---CLAIMS---", 1)
        summary = parts[0].strip()
        block = parts[1]
        if "---END---" in block:
            block = block.split("---END---")[0]
        
        current_claim = None
        for line in block.strip().split("\n"):
            line = line.strip()
            if line.startswith("- claim:") or line.startswith("- claim "):
                if current_claim and current_claim[0]:
                    claims.append({"claim": current_claim[0], "reason": current_claim[1]})
                claim_text = line.split(":", 1)[-1].strip() if ":" in line else line.split(" ", 2)[-1].strip()
                current_claim = [claim_text, ""]
            elif line.startswith("reason:") and current_claim:
                current_claim[1] = line.split(":", 1)[-1].strip()
        
        if current_claim and current_claim[0]:
            claims.append({"claim": current_claim[0], "reason": current_claim[1]})
    else:
        summary = raw_text
    
    return {"summary": summary, "claims": claims}


def parse_variant_bc(raw_response: dict) -> dict:
    """Parse JSON schema response."""
    content = raw_response.get("message", {}).get("content", "")
    try:
        return json.loads(content)
    except json.JSONDecodeError:
        return {"summary": content, "claims": []}


def evaluate(variant_name: str, result: dict, parsed: dict) -> dict:
    """Compute quality metrics for a variant run."""
    claims = parsed.get("claims", [])
    n_claims = len(claims)
    n_specific = sum(1 for c in claims if is_specific_claim(c.get("claim", "")))
    
    return {
        "variant": variant_name,
        "ok": result["ok"],
        "latency": round(result.get("latency", 0), 2),
        "claims_extracted": n_claims,
        "specific_claims": n_specific,
        "specificity_score": round(n_specific / max(n_claims, 1), 2),
        "format_compliance": n_claims > 0,
        "claims": [{"claim": c.get("claim", "")[:100], "reason": c.get("reason", "")[:100]} for c in claims],
    }


# ─── Main ──────────────────────────────────────────────────────
def main():
    variants = [
        ("A (Text ---CLAIMS---)", variant_a_prompt, parse_variant_a),
        ("B (JSON Schema)", variant_b_prompt, parse_variant_bc),
        ("C (JSON Schema + Few-Shot)", variant_c_prompt, parse_variant_bc),
    ]
    
    results = {}
    
    for case in TEST_CASES:
        print(f"\n{'='*60}")
        print(f"TEST: {case['id']}")
        print(f"{'='*60}")
        
        for vname, prompt_fn, parse_fn in variants:
            print(f"\n  [{vname}]", end=" ", flush=True)
            
            payload = prompt_fn(case["content"])
            result = call_ollama(payload)
            
            if not result["ok"]:
                print(f"FAILED: {result['error']}")
                continue
            
            parsed = parse_fn(result["raw"] if vname.startswith("A") else result["raw"])
            metrics = evaluate(vname, result, parsed)
            
            key = f"{case['id']}/{vname}"
            results[key] = metrics
            
            print(f"latency={metrics['latency']}s | claims={metrics['claims_extracted']} | specific={metrics['specific_claims']} | score={metrics['specificity_score']}")
            
            # Show first claim
            if metrics["claims"]:
                c = metrics["claims"][0]
                print(f"    → {c['claim'][:80]}")
            else:
                print(f"    → (no claims extracted)")
    
    # ─── Summary ────────────────────────────────────────────
    print(f"\n{'='*60}")
    print(f"SUMMARY")
    print(f"{'='*60}")
    
    print(f"\n{'Variant':<35} {'Avg Claims':>10} {'Specificity':>10} {'Compliance':>10} {'Avg Latency':>10}")
    print("-" * 80)
    
    for vname in ["A (Text ---CLAIMS---)", "B (JSON Schema)", "C (JSON Schema + Few-Shot)"]:
        variant_results = [r for k, r in results.items() if k.endswith(vname) and r["ok"]]
        if not variant_results:
            continue
        
        avg_claims = sum(r["claims_extracted"] for r in variant_results) / len(variant_results)
        avg_specific = sum(r["specific_claims"] for r in variant_results) / len(variant_results)
        avg_specificity = sum(r["specificity_score"] for r in variant_results) / len(variant_results)
        avg_latency = sum(r["latency"] for r in variant_results) / len(variant_results)
        compliance = sum(1 for r in variant_results if r["format_compliance"]) / len(variant_results)
        
        print(f"{vname:<35} {avg_claims:>10.1f} {avg_specificity:>10.2f} {compliance:>10.0%} {avg_latency:>9.2f}s")
    
    # Save raw results
    with open("/tmp/claims_spike_results.json", "w") as f:
        json.dump(results, f, indent=2, default=str)
    print(f"\nRaw results saved to /tmp/claims_spike_results.json")


if __name__ == "__main__":
    main()
