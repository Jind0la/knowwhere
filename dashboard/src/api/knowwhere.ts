/**
 * KnowWhere REST API client.
 *
 * Calls the KnowWhere backend running at the configured base URL.
 * During development, Vite proxies /api → http://localhost:3737.
 */

import type {
  AuthContext,
  DreamStatus,
  Event,
  FractalNode,
  GovernancePolicy,
  HealthResponse,
  RetrieveFractalRequest,
  ScoredNode,
  SubconsciousChatRequest,
  SubconsciousChatResponse,
  StoreNodeResponse,
  StoreSessionRequest,
  UpdatePolicyRequest,
} from '../types';

const BASE = '/api';
const TOKEN_KEY = 'knowwhere_api_token';

function bearerToken() {
  const token = localStorage.getItem(TOKEN_KEY);
  return token ? `Bearer ${token}` : undefined;
}

export function setApiToken(token: string) {
  localStorage.setItem(TOKEN_KEY, token.trim());
}

export function clearApiToken() {
  localStorage.removeItem(TOKEN_KEY);
}

export function getApiToken() {
  return localStorage.getItem(TOKEN_KEY) ?? '';
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  console.log('[api] request', path, init?.method ?? 'GET');
  try {
    const auth = bearerToken();
    const res = await fetch(`${BASE}${path}`, {
      headers: {
        'Content-Type': 'application/json',
        ...(auth ? { Authorization: auth } : {}),
        ...init?.headers,
      },
      ...init,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new Error(`${res.status} ${res.statusText}: ${text}`);
    }
    const contentType = res.headers.get('content-type') ?? '';
    if (contentType.includes('application/json')) {
      return res.json() as Promise<T>;
    }
    return undefined as T;
  } catch (error) {
    console.error('[api] request failed', path, error);
    throw error;
  }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

export async function health(): Promise<HealthResponse> {
  return request<HealthResponse>('/health');
}

export async function authMe(): Promise<AuthContext> {
  return request<AuthContext>('/auth/me');
}

// ---------------------------------------------------------------------------
// Embedding
// ---------------------------------------------------------------------------

export async function embedText(text: string): Promise<{ vector: number[]; dimension: number; provider: string }> {
  return request<{ vector: number[]; dimension: number; provider: string }>('/embed', {
    method: 'POST',
    body: JSON.stringify({ text }),
  });
}

// ---------------------------------------------------------------------------
// Memory — Store
// ---------------------------------------------------------------------------

export async function storeSession(body: StoreSessionRequest): Promise<StoreNodeResponse> {
  return request<StoreNodeResponse>('/store_session', {
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

export async function retrieveFractal(body: RetrieveFractalRequest): Promise<ScoredNode[]> {
  return request<ScoredNode[]>('/retrieve_fractal', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export async function subconsciousChat(body: SubconsciousChatRequest): Promise<SubconsciousChatResponse> {
  return request<SubconsciousChatResponse>('/chat/subconscious', {
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

export async function recentNodes(limit = 20): Promise<FractalNode[]> {
  return request<FractalNode[]>(`/nodes/recent?limit=${limit}`);
}

export async function purgeDummyNodes() {
  return request<{ removed: number; message: string }>('/nodes/purge_dummy', {
    method: 'POST',
  });
}

// ---------------------------------------------------------------------------
// Governance
// ---------------------------------------------------------------------------

export async function getGovernancePolicy(): Promise<GovernancePolicy> {
  return request<GovernancePolicy>('/governance/policy');
}

export async function updateGovernancePolicy(body: UpdatePolicyRequest): Promise<{ message: string; policy: GovernancePolicy }> {
  return request<{ message: string; policy: GovernancePolicy }>('/governance/policy', {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Dream Mode
// ---------------------------------------------------------------------------

export async function dreamStatus(): Promise<DreamStatus> {
  return request<DreamStatus>('/dream/status');
}

// ---------------------------------------------------------------------------
// Events (Layer 0)
// ---------------------------------------------------------------------------

export async function listEvents(params?: { after_id?: string; limit?: number }): Promise<Event[]> {
  const qs = new URLSearchParams();
  if (params?.after_id) qs.set('after_id', params.after_id);
  if (params?.limit) qs.set('limit', String(params.limit));
  const query = qs.toString() ? `?${qs.toString()}` : '';
  return request<Event[]>(`/events${query}`);
}
