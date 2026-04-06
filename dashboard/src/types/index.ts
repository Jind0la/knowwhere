// Memory Types — matching the KnowWhere backend type system

export type MemoryType = 'episodic' | 'semantic' | 'preference' | 'procedural' | 'meta';

export type Sensitivity = 'normal' | 'low' | 'high' | 'restricted';

export type MemoryStatus = 'active' | 'draft' | 'archived' | 'deleted' | 'superseded' | 'stale';

export type ConflictState = 'none' | 'pending' | 'resolved';

export type MemorySource =
  | 'conversation'
  | 'document'
  | 'import'
  | 'manual'
  | 'consolidation';

export type RetrievalProfile = 'user-facing' | 'agent-debug' | 'full-fidelity';

// ---------------------------------------------------------------------------
// Core Types
// ---------------------------------------------------------------------------

export interface FractalNode {
  id: string;
  memory_type: MemoryType;
  source: MemorySource;
  vector: number[];
  content: string | null;
  original_pointer: string | null;
  metadata: Record<string, unknown>;
  weight: number;
  confidence: number;
  sensitivity: Sensitivity;
  superseded_by: string | null;
  conflict_state: ConflictState;
  provenance: Record<string, unknown>;
  importance: number;
  status: MemoryStatus;
  access_count: number;
  created_at: string;
  last_accessed: string;
}

export interface ScoredNode {
  score: number;
  id: string;
  memory_type: MemoryType;
  source?: MemorySource;
  content: string | null;
  original_pointer: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
  retrieval_profile: RetrievalProfile;
  trust_tier: string;
  score_debug?: RetrievalScoreDebug;
  confidence?: number;
  sensitivity?: Sensitivity;
  governance_passed?: boolean;
  governance_issues: ValidationIssue[];
}

export interface RetrievalScoreDebug {
  profile: RetrievalProfile;
  trust_tier: string;
  base_score: number;
  multiplier: number;
  final_score: number;
  explanation: string;
}

export interface ValidationIssue {
  issue_type: IssueType;
  description: string;
  score_impact: number;
}

export type IssueType =
  | 'low_confidence'
  | 'superseded'
  | 'sensitivity_blocked'
  | 'stale'
  | 'unresolved_conflict'
  | 'invalid_status'
  | 'irrelevant';

// ---------------------------------------------------------------------------
// Governance Policy
// ---------------------------------------------------------------------------

export interface GovernancePolicy {
  min_confidence: number;
  max_age_days: number | null;
  blocked_sensitivities: Sensitivity[];
  supersession_enabled: boolean;
  conflict_check_enabled: boolean;
  recency_boost_enabled: boolean;
  recency_penalty_after_days: number;
}

export interface UpdatePolicyRequest {
  min_confidence?: number;
  max_age_days?: number;
  blocked_sensitivities?: Sensitivity[];
  supersession_enabled?: boolean;
  conflict_check_enabled?: boolean;
  recency_boost_enabled?: boolean;
  recency_penalty_after_days?: number;
  preset?: 'default' | 'strict' | 'lenient';
}

// ---------------------------------------------------------------------------
// Dream Mode
// ---------------------------------------------------------------------------

export interface DreamStatus {
  active: boolean;
  phase: string;
  memories_processed: number;
  consolidations_run: number;
  last_run: string | null;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

export interface Event {
  id: string;
  event_type: string;
  payload: Record<string, unknown>;
  created_at: string;
}

// ---------------------------------------------------------------------------
// API Request/Response Types
// ---------------------------------------------------------------------------

export interface StoreSessionRequest {
  content: string;
  vector?: number[];
  metadata?: Record<string, unknown>;
  memory_type?: MemoryType;
  source?: MemorySource;
  importance?: number;
  sensitivity?: Sensitivity;
}

export interface StoreNodeResponse {
  id: string;
  message: string;
}

export interface RetrieveFractalRequest {
  query_vector?: number[];
  query_text?: string;
  top_k?: number;
  max_depth?: number;
  governance_enabled?: boolean;
  memory_type_filter?: MemoryType;
  retrieval_profile?: RetrievalProfile;
  include_debug?: boolean;
}

export interface HealthResponse {
  status: string;
  node_count: number;
}

export interface SubconsciousChatRequest {
  message: string;
  top_k?: number;
  max_depth?: number;
  governance_enabled?: boolean;
  persist?: boolean;
  retrieval_profile?: RetrievalProfile;
  include_debug?: boolean;
}

export interface SubconsciousSource {
  id: string;
  score: number;
  memory_type: MemoryType;
  created_at: string;
  snippet: string;
  retrieval_profile: RetrievalProfile;
  trust_tier: string;
  score_debug?: RetrievalScoreDebug;
}

export interface SubconsciousChatResponse {
  answer: string;
  sources: SubconsciousSource[];
  stored: boolean;
}
