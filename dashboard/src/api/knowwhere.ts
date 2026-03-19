/**
 * KnowWhere REST API client.
 *
 * Calls the KnowWhere backend running at the configured base URL.
 * During development, Vite proxies /api → http://localhost:3000.
 */

const BASE = '/api';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
    ...init,
  });

  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(`${res.status} ${res.statusText}: ${text}`);
  }

  // Handle empty responses
  const contentType = res.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    return res.json() as Promise<T>;
  }
  return undefined as T;
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

export async function health() {
  return request<{ status: string; node_count: number }>('/health');
}

// ---------------------------------------------------------------------------
// Embedding
// ---------------------------------------------------------------------------

export async function embedText(text: string) {
  return request<{ vector: number[]; dimension: number; provider: string }>('/embed', {
    method: 'POST',
    body: JSON.stringify({ text }),
  });
}

// ---------------------------------------------------------------------------
// Memory — Store
// ---------------------------------------------------------------------------

export async function storeSession(body: {
  content: string;
  vector?: number[];
  metadata?: Record<string, unknown>;
  memory_type?: string;
  source?: string;
  importance?: number;
  sensitivity?: string;
}) {
  return request<{ id: string; message: string }>('/store_session', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export async function storeExternal(body: {
  pointer: string;
  vector?: number[];
  metadata?: Record<string, unknown>;
  multimodal?: unknown;
  memory_type?: string;
  source?: string;
  importance?: number;
  sensitivity?: string;
}) {
  return request<{ id: string; message: string }>('/store_external', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Memory — Retrieve
// ---------------------------------------------------------------------------

export async function retrieveNode(id: string) {
  return request<unknown>(`/retrieve/${id}`);
}

export async function retrieveFractal(body: {
  query_vector: number[];
  query_text?: string;
  top_k?: number;
  max_depth?: number;
  governance_enabled?: boolean;
  memory_type_filter?: string;
}) {
  return request<unknown[]>('/retrieve_fractal', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Memory — Manage
// ---------------------------------------------------------------------------

export async function deleteNode(id: string) {
  return request<{ id: string; message: string }>(`/nodes/${id}`, {
    method: 'DELETE',
  });
}

export async function recentNodes(limit = 20) {
  return request<unknown[]>(`/nodes/recent?limit=${limit}`);
}

export async function purgeDummyNodes() {
  return request<{ removed: number; message: string }>('/nodes/purge_dummy', {
    method: 'POST',
  });
}

// ---------------------------------------------------------------------------
// Governance
// ---------------------------------------------------------------------------

export async function getGovernancePolicy() {
  return request<{
    min_confidence: number;
    max_age_days: number | null;
    blocked_sensitivities: string[];
    supersession_enabled: boolean;
    conflict_check_enabled: boolean;
    recency_boost_enabled: boolean;
    recency_penalty_after_days: number;
  }>('/governance/policy');
}

export async function updateGovernancePolicy(body: {
  min_confidence?: number;
  max_age_days?: number;
  blocked_sensitivities?: string[];
  supersession_enabled?: boolean;
  conflict_check_enabled?: boolean;
  recency_boost_enabled?: boolean;
  recency_penalty_after_days?: number;
  preset?: string;
}) {
  return request<{ message: string; policy: unknown }>('/governance/policy', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Dream Mode
// ---------------------------------------------------------------------------

export async function dreamStatus() {
  return request<{
    active: boolean;
    phase: string;
    memories_processed: number;
    consolidations_run: number;
    last_run: string | null;
  }>('/dream/status');
}

// ---------------------------------------------------------------------------
// Events (Layer 0)
// ---------------------------------------------------------------------------

export async function listEvents(params?: { after_id?: string; limit?: number }) {
  const qs = new URLSearchParams();
  if (params?.after_id) qs.set('after_id', params.after_id);
  if (params?.limit) qs.set('limit', String(params.limit));
  const query = qs.toString() ? `?${qs.toString()}` : '';
  return request<unknown[]>(`/events${query}`);
}
