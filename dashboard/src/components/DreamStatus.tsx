import { useState, useEffect } from 'react';
import { dreamStatus } from '../api/knowwhere';
import type { DreamStatus } from '../types';

interface DreamStatusProps {
  enabled: boolean;
  refreshKey: number;
}

export function DreamStatus(props: DreamStatusProps) {
  const [status, setStatus] = useState<DreamStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    if (!props.enabled) return;
    setLoading(true);
    setError(null);
    try {
      console.log('[dream] load:start');
      const data = await dreamStatus();
      setStatus(data);
      console.log('[dream] load:done');
    } catch (err) {
      console.error('[dream] load:error', err);
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (!props.enabled) {
      setStatus(null);
      setError(null);
      return;
    }
    void load();
    const interval = setInterval(load, 30_000); // refresh every 30s
    return () => clearInterval(interval);
  }, [props.enabled, props.refreshKey]);

  if (!props.enabled) {
    return <p className="p-3 text-sm text-gray-400">API-Token speichern, um Dream Mode zu laden.</p>;
  }

  if (loading && !status) {
    return <p className="text-sm text-gray-400 p-3">Loading Dream Mode status…</p>;
  }

  if (error) {
    return (
      <div className="p-3">
        <p className="text-xs text-red-500 mb-2">Error: {error}</p>
        <button
          onClick={load}
          className="text-xs text-indigo-600 hover:underline"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!status) return null;

  return (
    <div className="p-4 space-y-2">
      <div className="flex items-center gap-2">
        <span
          className={`w-2 h-2 rounded-full ${
            status.active ? 'bg-green-400 animate-pulse' : 'bg-gray-300'
          }`}
        />
        <span className="text-sm font-medium text-gray-700">
          Dream Mode {status.active ? 'Active' : 'Idle'}
        </span>
        <span className="text-xs text-gray-400 ml-auto">{status.phase}</span>
      </div>

      <div className="grid grid-cols-2 gap-2 text-xs text-gray-600">
        <div>
          <span className="text-gray-400">Processed:</span>{' '}
          {status.memories_processed.toLocaleString()}
        </div>
        <div>
          <span className="text-gray-400">Consolidations:</span>{' '}
          {status.consolidations_run.toLocaleString()}
        </div>
        <div className="col-span-2">
          <span className="text-gray-400">Last run:</span>{' '}
          {status.last_run
            ? new Date(status.last_run).toLocaleString()
            : 'never'}
        </div>
      </div>

      <button
        onClick={load}
        className="text-xs text-indigo-600 hover:underline mt-1"
        disabled={loading}
      >
        {loading ? 'Refreshing…' : 'Refresh'}
      </button>
    </div>
  );
}
