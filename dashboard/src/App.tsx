import { useState, useCallback } from 'react';
import { SearchPanel } from './components/SearchPanel';
import { MemoryList } from './components/MemoryList';
import { DreamStatus } from './components/DreamStatus';
import { GovernancePanel } from './components/GovernancePanel';
import { embedText, health } from './api/knowwhere';
import type { ScoredNode } from './types';

type Tab = 'search' | 'governance';

export default function App() {
  const [tab, setTab] = useState<Tab>('search');
  const [results, setResults] = useState<ScoredNode[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [nodeCount, setNodeCount] = useState<number | null>(null);
  const [embedding, setEmbedding] = useState<number[] | null>(null);
  const [embeddingText, setEmbeddingText] = useState('');
  const [embeddingLoading, setEmbeddingLoading] = useState(false);

  // Load health on mount
  useState(() => {
    health()
      .then((h) => setNodeCount(h.node_count))
      .catch(() => {});
  });

  const handleResults = useCallback((nodes: ScoredNode[]) => {
    setResults(nodes);
    setError(null);
  }, []);

  const handleError = useCallback((msg: string) => {
    setError(msg);
    setResults([]);
  }, []);

  async function handleEmbed(e: React.FormEvent) {
    e.preventDefault();
    if (!embeddingText.trim()) return;
    setEmbeddingLoading(true);
    try {
      const res = await embedText(embeddingText);
      setEmbedding(res.vector);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setEmbeddingLoading(false);
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 flex flex-col">
      {/* Header */}
      <header className="bg-white border-b border-gray-200 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-3">
          <span className="text-xl">⚡</span>
          <h1 className="text-lg font-bold text-gray-800">KnowWhere</h1>
          {nodeCount !== null && (
            <span className="text-xs text-gray-400 bg-gray-100 px-2 py-0.5 rounded-full">
              {nodeCount.toLocaleString()} nodes
            </span>
          )}
        </div>

        <nav className="flex gap-1">
          {(['search', 'governance'] as Tab[]).map((t) => (
            <button
              key={t}
              className={`text-sm px-4 py-1.5 rounded-full transition ${
                tab === t
                  ? 'bg-indigo-100 text-indigo-700 font-medium'
                  : 'text-gray-500 hover:text-gray-700'
              }`}
              onClick={() => setTab(t)}
            >
              {t === 'search' ? '🔍 Search' : '🛡 Governance'}
            </button>
          ))}
        </nav>
      </header>

      <div className="flex flex-1 max-w-7xl mx-auto w-full gap-6 p-6">
        {/* Left sidebar */}
        <aside className="w-80 shrink-0 space-y-4">
          {/* Embed Panel */}
          <div className="bg-white rounded-lg border border-gray-200 shadow-sm p-4 space-y-3">
            <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide">
              Embed Query
            </h3>
            <form onSubmit={handleEmbed} className="space-y-2">
              <textarea
                className="w-full border border-gray-300 rounded px-3 py-2 text-sm resize-y"
                rows={2}
                placeholder="Text to embed..."
                value={embeddingText}
                onChange={(e) => setEmbeddingText(e.target.value)}
              />
              <button
                type="submit"
                className="w-full bg-gray-800 text-white text-sm py-1.5 rounded hover:bg-gray-900 disabled:opacity-50"
                disabled={embeddingLoading}
              >
                {embeddingLoading ? 'Embedding…' : 'Embed'}
              </button>
            </form>
            {embedding && (
              <p className="text-xs text-green-600 break-all">
                ✓ {embedding.length}d vector ready
              </p>
            )}
          </div>

          {tab === 'search' ? (
            <div className="bg-white rounded-lg border border-gray-200 shadow-sm p-4">
              <SearchPanel
                onResults={handleResults}
                onError={handleError}
                embedding={embedding}
              />
            </div>
          ) : (
            <div className="bg-white rounded-lg border border-gray-200 shadow-sm">
              <GovernancePanel />
            </div>
          )}

          {/* Dream Status */}
          <div className="bg-white rounded-lg border border-gray-200 shadow-sm">
            <div className="border-b border-gray-100 px-4 py-2">
              <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide">
                🌙 Dream Mode
              </h3>
            </div>
            <DreamStatus />
          </div>
        </aside>

        {/* Main content */}
        <main className="flex-1 min-w-0">
          {error && (
            <div className="bg-red-50 border border-red-200 rounded-lg p-4 mb-4">
              <p className="text-sm text-red-600">{error}</p>
            </div>
          )}

          <div className="bg-white rounded-lg border border-gray-200 shadow-sm">
            <div className="border-b border-gray-100 px-4 py-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold text-gray-700">
                Results
                {results.length > 0 && (
                  <span className="ml-2 text-gray-400 font-normal">
                    ({results.length})
                  </span>
                )}
              </h2>
              {results.length > 0 && (
                <span className="text-xs text-gray-400">
                  sorted by score × governance multiplier
                </span>
              )}
            </div>
            <MemoryList nodes={results} />
          </div>
        </main>
      </div>
    </div>
  );
}
