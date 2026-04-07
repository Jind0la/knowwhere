import { useState } from 'react';
import type { MemoryType, ScoredNode } from '../types';

const MEMORY_TYPE_COLORS: Record<MemoryType, string> = {
  episodic: '#6366f1',
  semantic: '#22c55e',
  preference: '#f59e0b',
  procedural: '#ef4444',
  meta: '#a855f7',
};

const MEMORY_TYPE_LABELS: Record<MemoryType, string> = {
  episodic: 'Episodic',
  semantic: 'Semantic',
  preference: 'Preference',
  procedural: 'Procedural',
  meta: 'Meta',
};

function trustTone(trustTier: string) {
  if (trustTier === 'primary') return 'border-emerald-300 text-emerald-700 bg-emerald-50';
  if (trustTier === 'derived') return 'border-amber-300 text-amber-700 bg-amber-50';
  return 'border-slate-300 text-slate-700 bg-slate-50';
}

interface MemoryListProps {
  nodes: ScoredNode[];
  loading?: boolean;
  onNodeClick?: (node: ScoredNode) => void;
}

export function MemoryList({ nodes, loading, onNodeClick }: MemoryListProps) {
  const [selected, setSelected] = useState<string | null>(null);

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8 text-gray-400">
        <span className="animate-pulse">Loading memories…</span>
      </div>
    );
  }

  if (nodes.length === 0) {
    return (
      <div className="p-8 text-center text-gray-500">
        No memories found. Try a different query.
      </div>
    );
  }

  return (
    <ul className="divide-y divide-gray-100">
      {nodes.map((node) => {
        const isSelected = selected === node.id;
        const color = MEMORY_TYPE_COLORS[node.memory_type] ?? '#6b7280';
        const label = MEMORY_TYPE_LABELS[node.memory_type] ?? node.memory_type;

        return (
          <li
            key={node.id}
            className={`p-4 cursor-pointer hover:bg-gray-50 transition ${
              isSelected ? 'bg-indigo-50' : ''
            }`}
            onClick={() => {
              setSelected(isSelected ? null : node.id);
              onNodeClick?.(node);
            }}
          >
            <div className="flex items-start gap-3">
              {/* Score badge */}
              <span className="text-xs font-mono text-gray-400 mt-0.5">
                {(node.score * 100).toFixed(1)}
              </span>

              {/* Memory type pill */}
              <span
                className="text-xs px-2 py-0.5 rounded-full text-white shrink-0"
                style={{ backgroundColor: color }}
              >
                {label}
              </span>

              <span className={`text-xs px-2 py-0.5 rounded-full border shrink-0 ${trustTone(node.trust_tier)}`}>
                {node.trust_tier}
              </span>

              <span className="text-xs px-2 py-0.5 rounded-full border border-gray-300 text-gray-500 shrink-0">
                {node.retrieval_profile}
              </span>

              {/* Governance status */}
              {node.governance_passed !== undefined && (
                <span
                  className={`text-xs px-2 py-0.5 rounded-full shrink-0 ${
                    node.governance_passed
                      ? 'bg-green-100 text-green-700'
                      : 'bg-red-100 text-red-700'
                  }`}
                >
                  {node.governance_passed ? '✓ passed' : '✗ blocked'}
                </span>
              )}

              {/* Content */}
              <div className="flex-1 min-w-0">
                <p className="text-sm text-gray-800 truncate">
                  {node.content ?? node.original_pointer ?? '(no content)'}
                </p>
                {node.confidence !== undefined && (
                  <p className="text-xs text-gray-400 mt-1">
                    confidence: {(node.confidence * 100).toFixed(0)}%
                  </p>
                )}
              </div>
            </div>

            {/* Expanded view */}
            {isSelected && (
              <div className="mt-3 pl-8 text-xs text-gray-600 space-y-1">
                <p>ID: {node.id}</p>
                <p>Created: {new Date(node.created_at).toLocaleString()}</p>
                {node.sensitivity && <p>Sensitivity: {node.sensitivity}</p>}
                {node.score_debug && (
                  <div className="mt-2">
                    <p className="font-medium">Score-Debug:</p>
                    <p>{node.score_debug.explanation}</p>
                  </div>
                )}
                {node.governance_issues.length > 0 && (
                  <div className="mt-2">
                    <p className="font-medium text-red-600">Governance issues:</p>
                    <ul className="list-disc list-inside">
                      {node.governance_issues.map((issue, i) => (
                        <li key={i}>{issue.description}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {Object.keys(node.metadata).length > 0 && (
                  <div className="mt-2">
                    <p className="font-medium">Metadata:</p>
                    <pre className="bg-gray-100 p-2 rounded overflow-auto">
                      {JSON.stringify(node.metadata, null, 2)}
                    </pre>
                  </div>
                )}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
