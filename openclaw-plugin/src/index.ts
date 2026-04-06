/**
 * KnowWhere Memory Plugin for OpenClaw
 * =====================================
 * Fractal memory layer that:
 *   1. BEFORE each prompt → retrieves relevant memories from KnowWhere
 *   2. AFTER each agent run → stores the session transcript to KnowWhere
 *   3. DURING compaction → stores the full transcript to KnowWhere
 *
 * Install: copy to ~/.openclaw/extensions/knowwhere/ or install via npm
 * Enable:  openclaw plugins enable knowwhere
 * Config:  add to openclaw.json plugins.entries.knowwhere.config
 */

import { definePluginEntry } from "openclaw/plugin-sdk/plugin-entry";

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

interface KnowWhereConfig {
  endpoint: string;
  apiKey: string;
  autoRecall: boolean;
  autoCapture: boolean;
  topK: number;
  storeOnCompaction: boolean;
}

interface ScoredMemory {
  id: string;
  content: string;
  score: number;
}

const DEFAULT_CONFIG: KnowWhereConfig = {
  endpoint: "http://127.0.0.1:3737",
  apiKey: "",
  autoRecall: true,
  autoCapture: true,
  topK: 5,
  storeOnCompaction: true,
};

// ─────────────────────────────────────────────────────────────────────────────
// KnowWhere API Client
// ─────────────────────────────────────────────────────────────────────────────

function makeHeaders(apiKey: string): Record<string, string> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (apiKey) headers["Authorization"] = `Bearer ${apiKey}`;
  return headers;
}

async function kwRetrieve(
  endpoint: string,
  apiKey: string,
  query: string,
  topK: number
): Promise<ScoredMemory[]> {
  const url = `${endpoint}/retrieve_fractal`;
  const body = JSON.stringify({ query_text: query, top_k: topK });

  try {
    const res = await fetch(url, {
      method: "POST",
      headers: makeHeaders(apiKey),
      body,
    });
    if (!res.ok) return [];

    const data = await res.json() as ScoredMemory[];
    return data.filter((n) => n.content);
  } catch {
    return [];
  }
}

async function kwStore(
  endpoint: string,
  apiKey: string,
  content: string,
  metadata?: Record<string, unknown>
): Promise<void> {
  const url = `${endpoint}/store_session`;
  const body = JSON.stringify({ content, ...(metadata ? { metadata } : {}) });

  try {
    await fetch(url, {
      method: "POST",
      headers: makeHeaders(apiKey),
      body,
    });
  } catch {
    // Best effort
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

function extractQueryFromEvent(event: {
  prompt?: string;
  messages?: unknown[];
}): string {
  // 1. Try the prompt field directly
  if (event.prompt) return event.prompt.slice(0, 500);

  // 2. Try first user message in the messages array
  if (Array.isArray(event.messages)) {
    for (const m of event.messages as Array<{ role?: string; content?: unknown }>) {
      if (m.role === "user") {
        const text =
          typeof m.content === "string"
            ? m.content
            : JSON.stringify(m.content ?? "");
        if (text) return text.slice(0, 500);
      }
    }
  }

  return "";
}

function serializeMessages(messages: unknown[]): string {
  return messages
    .map((m) => {
      if (typeof m === "object" && m !== null && "role" in m && "content" in m) {
        const msg = m as { role: string; content: unknown };
        const text =
          typeof msg.content === "string"
            ? msg.content
            : JSON.stringify(msg.content ?? "");
        return `[${msg.role}] ${text}`;
      }
      return String(m);
    })
    .join("\n")
    .slice(0, 4000);
}

function formatMemoriesForPrompt(memories: ScoredMemory[]): string {
  return memories
    .map((m, i) => `[Memory ${i + 1}]\n${m.content}`)
    .join("\n\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Plugin Entry
// ─────────────────────────────────────────────────────────────────────────────

export default definePluginEntry({
  id: "knowwhere",
  kind: "memory",
  name: "KnowWhere Memory",
  description:
    "Fractal memory layer for OpenClaw — retrieves and stores session context across sessions. Powered by KnowWhere (Axum + USearch + BM25).",

  register(api) {
    const log = (msg: string) =>
      api.logger?.info(`[knowwhere] ${msg}`) ??
      console.error(`[knowwhere] ${msg}`);

    const cfg = { ...DEFAULT_CONFIG, ...(api.pluginConfig ?? {}) } as KnowWhereConfig;

    // ── before_prompt_build ─────────────────────────────────────────────────
    // Fires before each LLM call. Retrieve relevant memories and inject them.
    api.on("before_prompt_build", async (event) => {
      if (!cfg.autoRecall) return;

      const query = extractQueryFromEvent(event);
      if (!query.trim()) return;

      const memories = await kwRetrieve(cfg.endpoint, cfg.apiKey, query, cfg.topK);
      if (memories.length === 0) return;

      log(`retrieved ${memories.length} memories for: "${query.slice(0, 60)}"`);

      // prependContext is per-turn and not cached — good for query-relevant memories
      return {
        prependContext: `

## Relevant Memories
${formatMemoriesForPrompt(memories)}
`,
      };
    });

    // ── agent_end ───────────────────────────────────────────────────────────
    // Fires after each agent run completes. Store the full transcript.
    api.on("agent_end", async (event) => {
      if (!cfg.autoCapture) return;

      const transcript = serializeMessages(event.messages);
      if (transcript.length < 20) return;

      log(`agent_end: storing ${transcript.length} chars`);
      await kwStore(cfg.endpoint, cfg.apiKey, transcript, {
        source: "openclaw:agent_end",
        role: "mixed",
        derivation: "agent_transcript",
        retrieval_visibility: "internal",
        trust_tier: "derived",
        success: event.success,
      });
    });

    // ── before_compaction ───────────────────────────────────────────────────
    // Fires before context compaction. Store the full pre-compaction transcript
    // so no conversation history is lost.
    api.on("before_compaction", async (event) => {
      if (!cfg.storeOnCompaction) return;
      if (!event.sessionFile) return;

      try {
        const { readFile } = await import("node:fs/promises");
        const content = await readFile(event.sessionFile, "utf-8");

        // Parse JSONL lines — each line is a message object
        const lines = content.trim().split("\n").filter(Boolean);
        const messages = lines
          .map((line) => {
            try { return JSON.parse(line); }
            catch { return null; }
          })
          .filter(Boolean);

        const transcript = serializeMessages(messages);
        if (transcript.length < 20) return;

        log(`before_compaction: storing ${transcript.length} chars from ${event.sessionFile}`);
        await kwStore(cfg.endpoint, cfg.apiKey, transcript, {
          source: "openclaw:before_compaction",
          role: "mixed",
          derivation: "agent_transcript",
          retrieval_visibility: "internal",
          trust_tier: "derived",
          messageCount: event.messageCount,
          tokenCount: event.tokenCount,
        });
      } catch (e) {
        log(`before_compaction read error: ${e}`);
      }
    });

    log("registered: before_prompt_build, agent_end, before_compaction");
  },
});
