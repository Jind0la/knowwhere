import type { DreamStatus, Event, FractalNode, HealthResponse } from '../types';

interface OverviewPanelProps {
  health: HealthResponse | null;
  dream: DreamStatus | null;
  events: Event[];
  recentNodes: FractalNode[];
  loading: boolean;
  error: string | null;
  tokenRequired: boolean;
}

function statCard(label: string, value: string, hint: string) {
  return (
    <article className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
      <p className="text-xs uppercase tracking-wide text-zinc-400">{label}</p>
      <p className="mt-1 text-2xl font-semibold text-zinc-100">{value}</p>
      <p className="mt-1 text-xs text-zinc-500">{hint}</p>
    </article>
  );
}

function formatTs(value: string | null) {
  if (!value) return '—';
  return new Date(value).toLocaleString();
}

export function OverviewPanel(props: OverviewPanelProps) {
  const eventTypes = new Set(props.events.map((entry) => entry.event_type)).size;
  const dreamRuns = props.dream?.consolidations_run ?? 0;
  if (props.loading) return <div className="p-6 text-sm text-zinc-400">Lade Overview...</div>;
  if (props.tokenRequired) {
    return <div className="p-6 text-sm text-zinc-400">API-Token speichern, um Overview-Daten zu laden.</div>;
  }
  if (props.error) return <div className="p-6 text-sm text-red-300">{props.error}</div>;
  return (
    <section className="space-y-4 p-4">
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        {statCard('Node Count', String(props.health?.node_count ?? 0), 'Gesamte gespeicherte Memories')}
        {statCard('Recent Nodes', String(props.recentNodes.length), 'Neueste Eintraege aus /nodes/recent')}
        {statCard('Events (24h)', String(props.events.length), 'Event-Log Aktivitaet')}
        {statCard('Event Types', String(eventTypes), 'Verteilte Event-Arten im Feed')}
      </div>
      <div className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
        <p className="text-xs uppercase tracking-wide text-zinc-400">Dream Mode</p>
        <div className="mt-2 grid gap-3 md:grid-cols-3">
          <p className="text-sm text-zinc-300">Aktiv: {props.dream?.active ? 'Ja' : 'Nein'}</p>
          <p className="text-sm text-zinc-300">Runs: {dreamRuns}</p>
          <p className="text-sm text-zinc-300">Letzter Lauf: {formatTs(props.dream?.last_run ?? null)}</p>
        </div>
      </div>
    </section>
  );
}
