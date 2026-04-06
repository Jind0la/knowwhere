import { useState } from 'react';
import { retrieveFractal, storeSession } from '../api/knowwhere';
import type { MemoryType, RetrievalProfile, ScoredNode } from '../types';

interface SearchPanelProps {
  onResults: (nodes: ScoredNode[]) => void;
  onError: (msg: string) => void;
  embedding: number[] | null;
  tokenRequired?: boolean;
}

const MEMORY_TYPES: MemoryType[] = [
  'episodic',
  'semantic',
  'preference',
  'procedural',
  'meta',
];

const PROFILE_OPTIONS: RetrievalProfile[] = ['user-facing', 'agent-debug', 'full-fidelity'];

export function SearchPanel({ onResults, onError, embedding, tokenRequired = false }: SearchPanelProps) {
  const [queryText, setQueryText] = useState('');
  const [topK, setTopK] = useState(5);
  const [maxDepth, setMaxDepth] = useState(3);
  const [governanceEnabled, setGovernanceEnabled] = useState(true);
  const [typeFilter, setTypeFilter] = useState<MemoryType | ''>('');
  const [profile, setProfile] = useState<RetrievalProfile>('user-facing');
  const [includeDebug, setIncludeDebug] = useState(true);
  const [loading, setLoading] = useState(false);

  const [storeContent, setStoreContent] = useState('');
  const [storeType, setStoreType] = useState<MemoryType>('episodic');
  const [storing, setStoring] = useState(false);

  if (tokenRequired) {
    return (
      <div className="rounded-xl border border-zinc-800 bg-zinc-950/50 p-4">
        <p className="text-sm text-zinc-300">Search braucht einen API-Token, weil Retrieval und Speichern geschuetzte Routen verwenden.</p>
      </div>
    );
  }

  async function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    if (!embedding) {
      onError('No embedding available. Embed a query first.');
      return;
    }
    setLoading(true);
    try {
      console.log('[search] start', { profile, includeDebug, topK, maxDepth });
      const results = (await retrieveFractal({
        query_vector: embedding,
        query_text: queryText || undefined,
        top_k: topK,
        max_depth: maxDepth,
        governance_enabled: governanceEnabled,
        memory_type_filter: typeFilter || undefined,
        retrieval_profile: profile,
        include_debug: includeDebug,
      })) as ScoredNode[];
      console.log('[search] done', { count: results.length });
      onResults(results);
    } catch (err) {
      console.error('[search] error', err);
      onError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleStore(e: React.FormEvent) {
    e.preventDefault();
    if (!storeContent.trim()) return;
    setStoring(true);
    try {
      console.log('[search] store:start', { storeType });
      await storeSession({
        content: storeContent,
        memory_type: storeType,
      });
      setStoreContent('');
      console.log('[search] store:done');
      onResults([]);
    } catch (err) {
      console.error('[search] store:error', err);
      onError(String(err));
    } finally {
      setStoring(false);
    }
  }

  return (
    <div className="space-y-6">
      {/* Search */}
      <form onSubmit={handleSearch} className="space-y-3">
        <h2 className="text-sm font-semibold text-gray-700 uppercase tracking-wide">
          Search Memory
        </h2>

        <textarea
          className="w-full border border-gray-300 rounded px-3 py-2 text-sm resize-y"
          rows={3}
          placeholder="Query text (optional — vector search is always used)..."
          value={queryText}
          onChange={(e) => setQueryText(e.target.value)}
        />

        <div className="flex flex-wrap gap-3">
          <label className="text-xs text-gray-600 flex items-center gap-1">
            Top K:
            <input
              type="number"
              className="w-14 border border-gray-300 rounded px-2 py-1 text-sm"
              value={topK}
              min={1}
              max={100}
              onChange={(e) => setTopK(Number(e.target.value))}
            />
          </label>

          <label className="text-xs text-gray-600 flex items-center gap-1">
            Max Depth:
            <input
              type="number"
              className="w-14 border border-gray-300 rounded px-2 py-1 text-sm"
              value={maxDepth}
              min={0}
              max={10}
              onChange={(e) => setMaxDepth(Number(e.target.value))}
            />
          </label>

          <label className="text-xs text-gray-600 flex items-center gap-1 cursor-pointer">
            <input
              type="checkbox"
              checked={governanceEnabled}
              onChange={(e) => setGovernanceEnabled(e.target.checked)}
            />
            Governance
          </label>
          <label className="text-xs text-gray-600 flex items-center gap-1 cursor-pointer">
            <input
              type="checkbox"
              checked={includeDebug}
              onChange={(e) => setIncludeDebug(e.target.checked)}
            />
            Score-Debug
          </label>
        </div>

        <div className="flex flex-wrap gap-2">
          <span className="text-xs text-gray-500 pt-1">Profil:</span>
          {PROFILE_OPTIONS.map((entry) => (
            <button
              key={entry}
              type="button"
              className={`text-xs px-2 py-1 rounded-full border transition ${
                profile === entry
                  ? 'bg-blue-100 border-blue-400 text-blue-700'
                  : 'border-gray-300 text-gray-500 hover:border-gray-400'
              }`}
              onClick={() => setProfile(entry)}
            >
              {entry}
            </button>
          ))}
        </div>

        <div className="flex flex-wrap gap-2">
          <span className="text-xs text-gray-500 pt-1">Filter:</span>
          {(['', ...MEMORY_TYPES] as const).map((t) => (
            <button
              key={t || 'all'}
              type="button"
              className={`text-xs px-2 py-1 rounded-full border transition ${
                typeFilter === t
                  ? 'bg-indigo-100 border-indigo-400 text-indigo-700'
                  : 'border-gray-300 text-gray-500 hover:border-gray-400'
              }`}
              onClick={() => setTypeFilter(t)}
            >
              {t || 'All'}
            </button>
          ))}
        </div>

        <button
          type="submit"
          className="w-full bg-indigo-600 text-white text-sm py-2 rounded hover:bg-indigo-700 disabled:opacity-50"
          disabled={loading || !embedding}
        >
          {loading ? 'Searching…' : 'Search'}
        </button>
      </form>

      {/* Store */}
      <form onSubmit={handleStore} className="space-y-3 border-t pt-4">
        <h2 className="text-sm font-semibold text-gray-700 uppercase tracking-wide">
          Store Memory
        </h2>

        <textarea
          className="w-full border border-gray-300 rounded px-3 py-2 text-sm resize-y"
          rows={3}
          placeholder="Content to store..."
          value={storeContent}
          onChange={(e) => setStoreContent(e.target.value)}
        />

        <div className="flex flex-wrap gap-2">
          <span className="text-xs text-gray-500 pt-1">Type:</span>
          {MEMORY_TYPES.map((t) => (
            <button
              key={t}
              type="button"
              className={`text-xs px-2 py-1 rounded-full border transition ${
                storeType === t
                  ? 'bg-indigo-100 border-indigo-400 text-indigo-700'
                  : 'border-gray-300 text-gray-500 hover:border-gray-400'
              }`}
              onClick={() => setStoreType(t)}
            >
              {t}
            </button>
          ))}
        </div>

        <button
          type="submit"
          className="w-full bg-green-600 text-white text-sm py-2 rounded hover:bg-green-700 disabled:opacity-50"
          disabled={storing || !storeContent.trim()}
        >
          {storing ? 'Storing…' : 'Store'}
        </button>
      </form>
    </div>
  );
}
