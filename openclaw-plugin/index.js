/**
 * KnowWhere Memory Plugin for OpenClaw
 * =====================================
 * Fractal memory layer that works across all OpenClaw modes:
 *   - Embedded (openclaw agent --local):   imports session files on startup
 *   - Gateway (Telegram, Discord, etc.):    captures messages via message_received
 *   - Interactive resets:                  saves session on /reset via before_reset
 *
 * Install: copy to ~/.openclaw/extensions/knowwhere/
 * Enable:  openclaw plugins enable knowwhere
 * Config:  openclaw.json → plugins.entries.knowwhere.config
 */

import { readFile } from "node:fs/promises";
import { readdir, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { definePluginEntry } from "openclaw/plugin-sdk/plugin-entry";

// ─────────────────────────────────────────────────────────────────────────────
// Default configuration
// ─────────────────────────────────────────────────────────────────────────────

const KW_TIMEOUT_MS = 5000; // Ollama coldstart can take 1–3 s

const DEFAULT_CONFIG = {
  endpoint: "http://127.0.0.1:3737",
  apiKey: "",
  autoRecall: true,
  autoCapture: true,
  topK: 5,
  /** Import sessions from the last N days on gateway startup */
  importLookbackDays: 7,
  /** Skip sessions smaller than this (likely heartbeat/noise) */
  minSessionSizeBytes: 200,
};

// ─────────────────────────────────────────────────────────────────────────────
// KnowWhere API Client
// ─────────────────────────────────────────────────────────────────────────────

function makeHeaders(apiKey) {
  const headers = { "Content-Type": "application/json" };
  if (apiKey) headers["Authorization"] = `Bearer ${apiKey}`;
  return headers;
}

async function fetchWithTimeout(url, opts, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...opts, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

async function kwRetrieve(endpoint, apiKey, query, topK) {
  const url = `${endpoint}/retrieve_fractal`;
  const body = JSON.stringify({ query_text: query, top_k: topK });
  try {
    const res = await fetchWithTimeout(
      url,
      { method: "POST", headers: makeHeaders(apiKey), body },
      KW_TIMEOUT_MS
    );
    if (!res.ok) return [];
    return await res.json();
  } catch (e) {
    if (e?.name !== "AbortError") {
      console.error(`[knowwhere] retrieve error: ${e}`);
    }
    return [];
  }
}

async function kwStore(endpoint, apiKey, content, metadata) {
  const url = `${endpoint}/store_session`;
  const body = JSON.stringify({ content, ...(metadata ? { metadata } : {}) });
  try {
    const res = await fetchWithTimeout(
      url,
      { method: "POST", headers: makeHeaders(apiKey), body },
      KW_TIMEOUT_MS
    );
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      console.error(`[knowwhere] store error ${res.status}: ${text || res.statusText}`);
    }
  } catch (e) {
    if (e?.name !== "AbortError") {
      console.error(`[knowwhere] store error: ${e}`);
    }
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Message serialization helpers
// ─────────────────────────────────────────────────────────────────────────────

const MEMORY_SECTION = "## Relevant Memories";

function stripMemorySections(text) {
  const idx = text.indexOf(MEMORY_SECTION);
  return idx !== -1 ? text.slice(0, idx).trimEnd() : text;
}

function extractText(raw) {
  if (typeof raw === "string") return raw;
  if (Array.isArray(raw)) {
    return raw
      .filter((part) => part && part.type === "text")
      .map((part) => part.text ?? "")
      .join("\n");
  }
  if (raw && typeof raw === "object") return JSON.stringify(raw);
  return String(raw ?? "");
}

function formatMemoriesForPrompt(memories) {
  return memories
    .map((m, i) => `[Memory ${i + 1}]\n${m.content}`)
    .join("\n\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Session file utilities
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Parse a session JSONL file and extract conversation messages.
 * Returns an array of { role, content } objects suitable for storage.
 * @param {string} filepath
 * @returns {Promise<Array<{role:string,content:string}>>}
 */
async function parseSessionFile(filepath) {
  const raw = await readFile(filepath, "utf-8");
  const lines = raw.trim().split("\n");
  const messages = [];

  for (const line of lines) {
    if (!line.trim()) continue;
    try {
      const entry = JSON.parse(line);
      if (entry.type !== "message") continue;
      const msg = entry.message;
      if (!msg || msg.role !== "user") continue;

      let text = extractText(msg.content);
      // Strip Relevant Memories injected by prependContext
      text = stripMemorySections(text);
      if (text.length < 5) continue;

      messages.push({ role: msg.role, content: text });
    } catch {
      // Skip malformed lines
    }
  }

  return messages;
}

/**
 * Find all session files for a given agent directory modified within lookbackDays.
 * @param {string} agentDir  e.g. ~/.openclaw/agents/main
 * @param {number} lookbackDays
 * @returns {Promise<string[]>} absolute file paths
 */
async function findRecentSessions(agentDir, lookbackDays) {
  const sessionsDir = join(agentDir, "sessions");
  const cutoff = Date.now() - lookbackDays * 24 * 60 * 60 * 1000;
  const results = [];

  let files;
  try {
    files = await readdir(sessionsDir);
  } catch {
    return results;
  }

  for (const fname of files) {
    if (!fname.endsWith(".jsonl") || fname.includes(".reset.")) continue;
    const fpath = join(sessionsDir, fname);
    try {
      const { mtime, size } = await stat(fpath);
      if (mtime.getTime() > cutoff && size > DEFAULT_CONFIG.minSessionSizeBytes) {
        results.push(fpath);
      }
    } catch {
      // Skip inaccessible files
    }
  }

  return results.sort((a, b) => stat(a).then(s => s.mtime).catch(() => 0) -
                              stat(b).then(s => s.mtime).catch(() => 0));
}

/**
 * Import all recent session files for a given agent.
 * @param {string} endpoint
 * @param {string} apiKey
 * @param {string} agentId  e.g. "main"
 * @param {number} lookbackDays
 * @returns {Promise<number>} total nodes stored
 */
async function importRecentSessions(endpoint, apiKey, agentId, lookbackDays) {
  const home = process.env.HOME ?? "";
  const agentDir = resolve(home, ".openclaw", "agents", agentId);
  const sessions = await findRecentSessions(agentDir, lookbackDays);

  let total = 0;
  for (const fpath of sessions) {
    try {
      const messages = await parseSessionFile(fpath);
      const fname = fpath.split("/").pop();

      for (const msg of messages) {
        await kwStore(endpoint, apiKey, msg.content, {
          source: "openclaw:import",
          session_file: fname,
          agent: agentId,
          role: msg.role,
        });
        total++;
      }
    } catch (e) {
      console.error(`[knowwhere] import error for ${fpath}: ${e}`);
    }
  }

  return total;
}

/**
 * Extract messages from an active session file (sessionId → filepath mapping
 * is maintained via session_start hook).
 * @param {string} filepath
 * @param {string} sessionId
 * @returns {Promise<Array<{role:string,content:string}>>}
 */
async function extractSessionContent(filepath, sessionId) {
  return parseSessionFile(filepath);
}

// ─────────────────────────────────────────────────────────────────────────────
// In-memory session → filepath map (populated by session_start)
// ─────────────────────────────────────────────────────────────────────────────

/** @type {Map<string, string>} sessionId → absolute session file path */
const sessionFileMap = new Map();

// ─────────────────────────────────────────────────────────────────────────────
// Plugin Entry
// ─────────────────────────────────────────────────────────────────────────────

export default definePluginEntry({
  id: "knowwhere",
  kind: "memory",
  name: "KnowWhere Memory",
  description:
    "Fractal memory layer for OpenClaw — retrieves and stores session context across sessions. " +
    "Powered by KnowWhere (Axum + USearch + BM25).",

  register(api) {
    const log = (msg) =>
      api.logger?.info(`[knowwhere] ${msg}`) ??
      console.error(`[knowwhere] ${msg}`);

    const cfg = { ...DEFAULT_CONFIG, ...(api.pluginConfig ?? {}) };

    // ── before_prompt_build ─────────────────────────────────────────────────
    // Fires before EVERY LLM call — the primary recall hook for all modes.
    // Returns: { prependContext: string }
    api.on("before_prompt_build", async (event) => {
      if (!cfg.autoRecall) return;

      // Extract the most recent user message as the search query
      const messages = event.messages ?? [];
      let query = "";

      for (let i = messages.length - 1; i >= 0; i--) {
        const m = messages[i];
        if (m && m.role === "user") {
          query = extractText(m.content);
          break;
        }
      }

      // Fallback: prompt string
      if (!query && event.prompt) {
        query = String(event.prompt).slice(0, 500);
      }

      if (!query.trim()) return;

      const memories = await kwRetrieve(
        cfg.endpoint,
        cfg.apiKey,
        query,
        cfg.topK
      );
      if (!memories || memories.length === 0) return;

      log(`retrieved ${memories.length} memories for: "${query.slice(0, 60)}..."`);

      return {
        prependContext: `

## Relevant Memories
${formatMemoriesForPrompt(memories)}
`,
      };
    });

    // ── message_received ────────────────────────────────────────────────────
    // Fires for incoming messages via channels (Telegram, Discord, etc.)
    // when OpenClaw runs in gateway mode (daemon).
    // Does NOT fire in embedded --local mode.
    //
    // Event: { from: string, content: string, timestamp: number, metadata: {...} }
    api.on("message_received", async (event) => {
      if (!cfg.autoCapture) return;

      const rawContent = event.content ?? "";
      const content = stripMemorySections(extractText(rawContent));
      if (content.length < 10) return;

      const channel =
        event.metadata?.originatingChannel ??
        event.metadata?.provider ??
        event.metadata?.surface ??
        "unknown";

      log(`message_received: storing ${content.length} chars from ${event.from ?? "?"} via ${channel}`);

      await kwStore(cfg.endpoint, cfg.apiKey, content, {
        source: "openclaw:message_received",
        sender: event.from ?? "unknown",
        channel,
        timestamp: event.timestamp
          ? new Date(event.timestamp).toISOString()
          : undefined,
      });
    });

    // ── session_start ──────────────────────────────────────────────────────
    // Fires when a session is resumed (NOT on first creation in embedded mode).
    // Maps sessionId → sessionFile so we can read the file on session_end.
    //
    // Event: { sessionId: string, sessionKey: string, resumedFrom?: string }
    api.on("session_start", async (event) => {
      if (!event.sessionId || !event.sessionKey) return;

      // Derive the session file path from sessionId.
      // OpenClaw stores sessions at: ~/.openclaw/agents/<agentId>/sessions/<sessionId>.jsonl
      // The agentId comes from the context.
      const agentId = event.agentId ?? "main";
      const sessionFile = resolve(
        process.env.HOME ?? "",
        ".openclaw",
        "agents",
        agentId,
        "sessions",
        `${event.sessionId}.jsonl`
      );

      sessionFileMap.set(event.sessionId, sessionFile);
      log(`session_start: mapped ${event.sessionId} → ${sessionFile.split("/").pop()}`);
    });

    // ── before_reset ────────────────────────────────────────────────────────
    // Fires when /new or /reset clears a session, BEFORE messages are lost.
    // This is the primary storage hook for embedded mode — it fires at the
    // END of each openclaw agent --local command run.
    //
    // Note: In embedded mode, there is no session_end hook that fires between
    // commands. before_reset fires when the session is explicitly cleared,
    // which happens at the end of each interactive turn.
    //
    // The session content is reconstructed from the session file (mapped via
    // session_start) or read directly from the messages in the current context.
    api.on("before_reset", async (event) => {
      if (!cfg.autoCapture) return;

      log(`before_reset: sessionId=${event.sessionId ?? "unknown"} — storing session`);

      // Try to read from session file if we have it mapped
      if (event.sessionId && sessionFileMap.has(event.sessionId)) {
        const filepath = sessionFileMap.get(event.sessionId);
        try {
          const messages = await parseSessionFile(filepath);
          let stored = 0;
          for (const msg of messages) {
            await kwStore(cfg.endpoint, cfg.apiKey, msg.content, {
              source: "openclaw:before_reset",
              session_id: event.sessionId,
              role: msg.role,
            });
            stored++;
          }
          log(`before_reset: stored ${stored} messages from ${filepath.split("/").pop()}`);
        } catch (e) {
          log(`before_reset: failed to read session file: ${e}`);
        }
      }

      // Fallback: if no session file mapped, log the reset event for audit
      if (!event.sessionId || !sessionFileMap.has(event.sessionId)) {
        log(`before_reset: no session file mapped for session (sessionId=${event.sessionId ?? "unknown"})`);
      }
    });

    // ── gateway_start ────────────────────────────────────────────────────────
    // Fires once when the OpenClaw gateway daemon starts (launchd or openclaw run).
    // Import recent sessions from the last N days so the agent starts with
    // context from past sessions.
    //
    // This is essential for embedded mode: when you run `openclaw agent --local`,
    // the gateway is already running as a daemon, so gateway_start already fired
    // and imported past sessions before your --local command runs.
    api.on("gateway_start", async () => {
      if (!cfg.autoCapture) return;

      const days = cfg.importLookbackDays ?? 7;
      log(`gateway_start: importing sessions from the last ${days} days...`);

      try {
        // Import for the main agent
        const count = await importRecentSessions(
          cfg.endpoint,
          cfg.apiKey,
          "main",
          days
        );
        log(`gateway_start: imported ${count} messages from main agent sessions`);
      } catch (e) {
        log(`gateway_start: import failed: ${e}`);
      }
    });

    // ── session_end ─────────────────────────────────────────────────────────
    // Fires when a session transitions away (resumed by a new session, channel
    // closes, etc.). In gateway mode with Telegram/Discord, this fires when
    // switching topics/chats.
    //
    // Event: { sessionId: string, sessionKey: string, messageCount: number }
    api.on("session_end", async (event) => {
      if (!cfg.autoCapture) return;

      log(`session_end: sessionId=${event.sessionId}, messageCount=${event.messageCount ?? 0}`);

      // Try to read and store the ending session
      if (event.sessionId && sessionFileMap.has(event.sessionId)) {
        const filepath = sessionFileMap.get(event.sessionId);
        try {
          const messages = await parseSessionFile(filepath);
          let stored = 0;
          for (const msg of messages) {
            await kwStore(cfg.endpoint, cfg.apiKey, msg.content, {
              source: "openclaw:session_end",
              session_id: event.sessionId,
              role: msg.role,
            });
            stored++;
          }
          log(`session_end: stored ${stored} messages`);
        } catch (e) {
          log(`session_end: failed to read session file: ${e}`);
        }
        sessionFileMap.delete(event.sessionId);
      }
    });

    log(
      "registered: before_prompt_build, message_received, session_start, " +
      "before_reset, gateway_start, session_end"
    );
  },
});
