import type { Event, FractalNode } from '../types';

interface MemoryStreamPanelProps {
  recentNodes: FractalNode[];
  events: Event[];
  loading: boolean;
  error: string | null;
  tokenRequired: boolean;
}

function truncate(content: string | null, fallback: string | null) {
  const text = content ?? fallback ?? '(ohne Inhalt)';
  return text.length > 140 ? `${text.slice(0, 140)}...` : text;
}

function ts(value: string) {
  return new Date(value).toLocaleString();
}

export function MemoryStreamPanel(props: MemoryStreamPanelProps) {
  if (props.loading) return <div className="p-6 text-sm text-zinc-400">Lade Memory Stream...</div>;
  if (props.tokenRequired) {
    return <div className="p-6 text-sm text-zinc-400">API-Token speichern, um Memory- und Event-Daten zu laden.</div>;
  }
  if (props.error) return <div className="p-6 text-sm text-red-300">{props.error}</div>;
  return (
    <section className="grid gap-4 p-4 xl:grid-cols-2">
      <article className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
        <h3 className="text-sm font-semibold text-zinc-100">Neueste Memories</h3>
        <ul className="mt-3 space-y-3">
          {props.recentNodes.map((node) => (
            <li key={node.id} className="rounded-lg border border-zinc-800 bg-zinc-950/60 p-3">
              <p className="text-xs uppercase tracking-wide text-zinc-500">{node.memory_type}</p>
              <p className="mt-1 text-sm text-zinc-200">{truncate(node.content, node.original_pointer)}</p>
              <p className="mt-2 text-xs text-zinc-500">{ts(node.created_at)}</p>
            </li>
          ))}
        </ul>
      </article>
      <article className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4">
        <h3 className="text-sm font-semibold text-zinc-100">Event Feed</h3>
        <ul className="mt-3 space-y-3">
          {props.events.map((event) => (
            <li key={event.id} className="rounded-lg border border-zinc-800 bg-zinc-950/60 p-3">
              <p className="text-xs uppercase tracking-wide text-zinc-500">{event.event_type}</p>
              <p className="mt-1 break-all text-xs text-zinc-400">ID: {event.id}</p>
              <p className="mt-1 text-xs text-zinc-500">{ts(event.created_at)}</p>
            </li>
          ))}
        </ul>
      </article>
    </section>
  );
}
