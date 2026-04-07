import { useEffect, useState } from 'react';
import { subconsciousChat } from '../api/knowwhere';
import type { RetrievalProfile, SubconsciousSource } from '../types';

type ChatRole = 'user' | 'assistant';

interface ChatMessage {
  id: string;
  role: ChatRole;
  text: string;
  sources: SubconsciousSource[];
}

interface SubconsciousChatPanelProps {
  availableProfiles?: RetrievalProfile[];
  tokenRequired?: boolean;
}

const DEFAULT_PROFILES: RetrievalProfile[] = ['user-facing'];

function nowId() {
  return `${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
}

function clip(value: string) {
  const chars = Array.from(value);
  return chars.length > 180 ? `${chars.slice(0, 180).join('')}...` : value;
}

function addMessage(list: ChatMessage[], next: ChatMessage) {
  return [...list, next];
}

export function SubconsciousChatPanel({
  availableProfiles = DEFAULT_PROFILES,
  tokenRequired = false,
}: SubconsciousChatPanelProps) {
  const [input, setInput] = useState('');
  const [saving, setSaving] = useState(false);
  const [profile, setProfile] = useState<RetrievalProfile>('user-facing');
  const [includeDebug, setIncludeDebug] = useState(true);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);

  useEffect(() => {
    if (!availableProfiles.includes(profile)) {
      setProfile(availableProfiles[0] ?? 'user-facing');
    }
  }, [availableProfiles, profile]);

  if (tokenRequired) {
    return (
      <section className="space-y-4 p-4">
        <header className="rounded-xl border border-zinc-800 bg-zinc-950/50 p-4">
          <h2 className="text-sm font-semibold text-zinc-100">Subconscious Chat</h2>
          <p className="mt-1 text-xs text-zinc-400">Fuer Chat brauchst du einen API-Token, weil der Endpoint geschuetzt ist.</p>
        </header>
      </section>
    );
  }

  async function handleSend(event: React.FormEvent) {
    event.preventDefault();
    if (!input.trim()) return;
    const question = input.trim();
    setError(null);
    setInput('');
    setSending(true);
    console.log('[chat] send:start', { question, profile, includeDebug, saving });
    setMessages((prev) => addMessage(prev, { id: nowId(), role: 'user', text: question, sources: [] }));
    try {
      const response = await subconsciousChat({
        message: question,
        top_k: 5,
        max_depth: 3,
        governance_enabled: true,
        persist: saving,
        retrieval_profile: profile,
        include_debug: includeDebug,
      });
      console.log('[chat] send:done', { sources: response.sources.length, stored: response.stored });
      setMessages((prev) => addMessage(prev, {
        id: nowId(),
        role: 'assistant',
        text: response.answer,
        sources: response.sources,
      }));
    } catch (reason) {
      console.error('[chat] send:error', reason);
      setError(`Chat-Anfrage fehlgeschlagen: ${String(reason)}`);
    } finally {
      setSending(false);
    }
  }

  return (
    <section className="space-y-4 p-4">
      <header className="rounded-xl border border-zinc-800 bg-zinc-950/50 p-4">
        <h2 className="text-sm font-semibold text-zinc-100">Subconscious Chat</h2>
        <p className="mt-1 text-xs text-zinc-400">Frage KnowWhere direkt. Antwort basiert auf gefundenen Memories.</p>
      </header>
      {error && <p className="rounded-lg border border-red-900 bg-red-950/50 p-3 text-sm text-red-300">{error}</p>}
      <div className="space-y-3 rounded-xl border border-zinc-800 bg-zinc-950/30 p-3">
        {messages.length === 0 && <p className="text-sm text-zinc-500">Noch kein Verlauf. Stelle deine erste Frage.</p>}
        {messages.map((message) => (
          <article key={message.id} className={`rounded-lg border p-3 ${message.role === 'user' ? 'border-blue-900 bg-blue-950/30' : 'border-zinc-800 bg-zinc-900/80'}`}>
            <p className="mb-2 text-xs uppercase tracking-wide text-zinc-400">{message.role}</p>
            <pre className="whitespace-pre-wrap text-sm text-zinc-100">{message.text}</pre>
            {message.sources.length > 0 && (
              <ul className="mt-3 space-y-2">
                {message.sources.map((source) => (
                  <li key={source.id} className="rounded-md border border-zinc-700 bg-zinc-950/80 p-2 text-xs text-zinc-300">
                    <div className="flex flex-wrap items-center gap-2">
                      <p className="uppercase tracking-wide text-zinc-500">{source.memory_type}</p>
                      <span className="rounded-full border border-zinc-700 px-2 py-0.5 text-[10px] uppercase tracking-wide text-blue-300">
                        {source.retrieval_profile}
                      </span>
                      <span className="rounded-full border border-zinc-700 px-2 py-0.5 text-[10px] uppercase tracking-wide text-emerald-300">
                        {source.trust_tier}
                      </span>
                    </div>
                    <p className="mt-1">{clip(source.snippet)}</p>
                    {source.score_debug && (
                      <p className="mt-2 text-[11px] text-zinc-500">{source.score_debug.explanation}</p>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </article>
        ))}
      </div>
      <form onSubmit={handleSend} className="space-y-3 rounded-xl border border-zinc-800 bg-zinc-950/50 p-4">
        <textarea
          rows={3}
          value={input}
          onChange={(event) => setInput(event.target.value)}
          className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm"
          placeholder="Was bewegt dich gerade? Frag dein Unterbewusstsein..."
        />
        <div className="flex items-center gap-3">
          <button type="submit" disabled={sending} className="rounded-lg bg-blue-500 px-4 py-2 text-sm font-medium text-white disabled:opacity-60">
            {sending ? 'Denke nach...' : 'Senden'}
          </button>
          <label className="flex items-center gap-2 text-xs text-zinc-400">
            <input type="checkbox" checked={saving} onChange={(event) => setSaving(event.target.checked)} />
            Verlauf in KnowWhere speichern
          </label>
          <label className="flex items-center gap-2 text-xs text-zinc-400">
            <input type="checkbox" checked={includeDebug} onChange={(event) => setIncludeDebug(event.target.checked)} />
            Score-Debug anzeigen
          </label>
        </div>
        <div className="flex flex-wrap gap-2">
          {availableProfiles.map((entry) => (
            <button
              key={entry}
              type="button"
              className={`rounded-full border px-2 py-1 text-xs ${
                profile === entry ? 'border-blue-500 bg-blue-500/10 text-blue-300' : 'border-zinc-700 text-zinc-400'
              }`}
              onClick={() => setProfile(entry)}
            >
              {entry}
            </button>
          ))}
        </div>
        <p className="text-xs text-zinc-500">
          Erlaubte Profile laut Token: {availableProfiles.join(', ')}
        </p>
        <p className="text-xs text-zinc-500">
          Standardmaessig aus, damit Chat-Fragen und Antwortkompositionen nicht versehentlich spaetere Retrievals verunreinigen.
        </p>
      </form>
    </section>
  );
}
