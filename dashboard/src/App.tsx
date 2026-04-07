import { useCallback, useEffect, useState } from 'react';
import {
  authMe,
  clearApiToken,
  dreamStatus,
  embedText,
  getApiToken,
  health,
  listEvents,
  recentNodes,
  setApiToken,
} from './api/knowwhere';
import { DreamStatus } from './components/DreamStatus';
import { GovernancePanel } from './components/GovernancePanel';
import { MemoryList } from './components/MemoryList';
import { MemoryStreamPanel } from './components/MemoryStreamPanel';
import { OverviewPanel } from './components/OverviewPanel';
import { SearchPanel } from './components/SearchPanel';
import { SubconsciousChatPanel } from './components/SubconsciousChatPanel';
import type {
  AuthContext as AuthContextType,
  DreamStatus as DreamStatusType,
  Event,
  FractalNode,
  HealthResponse,
  RetrievalProfile,
  ScoredNode,
} from './types';

type Tab = 'overview' | 'memories' | 'chat' | 'search' | 'governance';

const TABS: Tab[] = ['overview', 'memories', 'chat', 'search', 'governance'];
const FALLBACK_PROFILES: RetrievalProfile[] = ['user-facing'];

export default function App() {
  const [tab, setTab] = useState<Tab>('overview');
  const [results, setResults] = useState<ScoredNode[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [tokenInput, setTokenInput] = useState(getApiToken());
  const [tokenRefreshKey, setTokenRefreshKey] = useState(0);
  const [nodeCount, setNodeCount] = useState<number>(0);
  const [embedding, setEmbedding] = useState<number[] | null>(null);
  const [embeddingText, setEmbeddingText] = useState('');
  const [embeddingLoading, setEmbeddingLoading] = useState(false);
  const [overviewLoading, setOverviewLoading] = useState(true);
  const [overviewError, setOverviewError] = useState<string | null>(null);
  const [authData, setAuthData] = useState<AuthContextType | null>(null);
  const [healthData, setHealthData] = useState<HealthResponse | null>(null);
  const [dreamData, setDreamData] = useState<DreamStatusType | null>(null);
  const [eventsData, setEventsData] = useState<Event[]>([]);
  const [recentData, setRecentData] = useState<FractalNode[]>([]);
  const tokenAvailable = Boolean(getApiToken());
  const allowedProfiles = authData?.allowed_retrieval_profiles ?? FALLBACK_PROFILES;

  const refreshOverview = useCallback(async () => {
    const hasToken = Boolean(getApiToken());
    setOverviewLoading(true);
    setOverviewError(null);
    console.log('[dashboard] refreshOverview:start');
    try {
      const healthRes = await health();
      setNodeCount(healthRes.node_count);
      setHealthData(healthRes);
      if (!hasToken) {
        setAuthData(null);
        setDreamData(null);
        setEventsData([]);
        setRecentData([]);
        console.log('[dashboard] refreshOverview:token-missing');
        return;
      }
      const [authRes, dreamRes, eventsRes, recentRes] = await Promise.all([
        authMe(),
        dreamStatus(),
        listEvents({ limit: 20 }),
        recentNodes(20),
      ]);
      setAuthData(authRes);
      setDreamData(dreamRes);
      setEventsData(eventsRes);
      setRecentData(recentRes);
      console.log('[dashboard] refreshOverview:done', {
        nodeCount: healthRes.node_count,
        tokenKind: authRes.token_kind,
        profiles: authRes.allowed_retrieval_profiles,
      });
    } catch (err) {
      const message = `Overview-Load fehlgeschlagen: ${String(err)}`;
      console.error('[dashboard] refreshOverview:error', err);
      setOverviewError(message);
    } finally {
      setOverviewLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshOverview();
  }, [refreshOverview]);

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
      console.log('[dashboard] embed:start');
      const res = await embedText(embeddingText);
      setEmbedding(res.vector);
      setError(null);
      console.log('[dashboard] embed:done', { dimension: res.dimension });
    } catch (err) {
      console.error('[dashboard] embed:error', err);
      setError(String(err));
    } finally {
      setEmbeddingLoading(false);
    }
  }

  function saveToken() {
    setApiToken(tokenInput);
    setTokenRefreshKey((value) => value + 1);
    console.log('[dashboard] api token saved');
    void refreshOverview();
  }

  function removeToken() {
    clearApiToken();
    setTokenInput('');
    setTokenRefreshKey((value) => value + 1);
    setDreamData(null);
    setEventsData([]);
    setRecentData([]);
    setResults([]);
    setError(null);
    setAuthData(null);
    console.log('[dashboard] api token cleared');
  }

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100">
      <header className="border-b border-zinc-800 bg-zinc-900/70 px-6 py-4">
        <div className="flex items-center gap-3">
          <span className="text-xl">🧠</span>
          <h1 className="text-lg font-semibold">KnowWhere Dashboard</h1>
          <span className="rounded-full border border-zinc-700 px-2 py-0.5 text-xs text-zinc-300">
            {nodeCount.toLocaleString()} nodes
          </span>
        </div>
      </header>
      <div className="mx-auto flex w-full max-w-7xl gap-6 p-6">
        <aside className="w-80 shrink-0 space-y-4">
          <div className="space-y-2 rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
            <p className="text-xs uppercase tracking-wide text-zinc-500">Navigation</p>
            {TABS.map((entry) => (
              <button
                key={entry}
                type="button"
                onClick={() => setTab(entry)}
                className={`w-full rounded-lg px-3 py-2 text-left text-sm transition ${
                  tab === entry ? 'bg-zinc-100 text-zinc-900' : 'bg-zinc-800/80 text-zinc-200'
                }`}
              >
                {entry}
              </button>
            ))}
          </div>
          <div className="space-y-2 rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
            <p className="text-xs uppercase tracking-wide text-zinc-500">Embed Query</p>
            <form onSubmit={handleEmbed} className="space-y-2">
              <textarea
                rows={2}
                value={embeddingText}
                placeholder="Text fuer Embedding"
                onChange={(event) => setEmbeddingText(event.target.value)}
                className="w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm"
              />
              <button type="submit" disabled={embeddingLoading} className="w-full rounded-lg bg-blue-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-60">
                {embeddingLoading ? 'Embedding...' : 'Embed'}
              </button>
            </form>
            {embedding && <p className="text-xs text-emerald-300">Vector bereit: {embedding.length} Dimensionen</p>}
          </div>
          <div className="space-y-2 rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
            <p className="text-xs uppercase tracking-wide text-zinc-500">API Token</p>
            <input
              type="password"
              value={tokenInput}
              onChange={(event) => setTokenInput(event.target.value)}
              placeholder="Bearer Token (optional)"
              className="w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm"
            />
            <div className="flex gap-2">
              <button type="button" onClick={saveToken} className="flex-1 rounded-lg bg-zinc-100 px-3 py-2 text-xs font-medium text-zinc-900">
                Speichern
              </button>
              <button type="button" onClick={removeToken} className="flex-1 rounded-lg border border-zinc-700 px-3 py-2 text-xs">
                Loeschen
              </button>
            </div>
            {tokenAvailable && !authData && overviewLoading && (
              <p className="text-xs text-zinc-500">Token-Capabilities werden geladen...</p>
            )}
            {authData && (
              <div className="rounded-lg border border-zinc-800 bg-zinc-950/50 p-3 text-xs text-zinc-400">
                <p>Token-Typ: {authData.token_kind}</p>
                <p>Profile: {authData.allowed_retrieval_profiles.join(', ')}</p>
              </div>
            )}
          </div>
          <div className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
            <DreamStatus enabled={tokenAvailable} refreshKey={tokenRefreshKey} />
          </div>
          <button type="button" onClick={() => void refreshOverview()} className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm">
            Daten aktualisieren
          </button>
        </aside>
        <main className="min-w-0 flex-1">
          {error && (
            <div className="mb-4 rounded-xl border border-red-900 bg-red-950/60 p-4">
              <p className="text-sm text-red-300">{error}</p>
            </div>
          )}
          <section className="rounded-xl border border-zinc-800 bg-zinc-900/70">
            {tab === 'overview' && (
              <OverviewPanel
                loading={overviewLoading}
                error={overviewError}
                health={healthData}
                dream={dreamData}
                events={eventsData}
                recentNodes={recentData}
                tokenRequired={!tokenAvailable}
              />
            )}
            {tab === 'memories' && (
              <MemoryStreamPanel
                loading={overviewLoading}
                error={overviewError}
                events={eventsData}
                recentNodes={recentData}
                tokenRequired={!tokenAvailable}
              />
            )}
            {tab === 'search' && (
              <div className="space-y-4 p-4">
                <SearchPanel
                  onResults={handleResults}
                  onError={handleError}
                  embedding={embedding}
                  availableProfiles={allowedProfiles}
                  tokenRequired={!tokenAvailable}
                />
                <div className="rounded-xl border border-zinc-800 bg-zinc-950/50">
                  <MemoryList nodes={results} />
                </div>
              </div>
            )}
            {tab === 'chat' && (
              <SubconsciousChatPanel
                availableProfiles={allowedProfiles}
                tokenRequired={!tokenAvailable}
              />
            )}
            {tab === 'governance' && <GovernancePanel />}
          </section>
        </main>
      </div>
    </div>
  );
}
