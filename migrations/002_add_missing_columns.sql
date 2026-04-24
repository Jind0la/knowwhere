-- Migration 002: Add energy column and other missing columns
-- (Reconstructed from code analysis)

-- Add energy column for energy decay feature
ALTER TABLE memories ADD COLUMN IF NOT EXISTS energy DOUBLE PRECISION DEFAULT 1.0;
ALTER TABLE memories ADD COLUMN IF NOT EXISTS last_energy_update TIMESTAMPTZ DEFAULT NOW();

-- Add content_hash if not exists (from 009 migration)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS content_hash VARCHAR(64);

-- Add children_tier_ids if not exists (from 012 migration)
ALTER TABLE memories ADD COLUMN IF NOT EXISTS children_tier_ids UUID[];

-- Add original_pointer if referenced in code
ALTER TABLE memories ADD COLUMN IF NOT EXISTS original_pointer VARCHAR(255);

-- Add context_tier for tiered memory
ALTER TABLE memories ADD COLUMN IF NOT EXISTS context_tier VARCHAR(50) DEFAULT 'raw';

-- Ensure all needed indexes exist
CREATE INDEX IF NOT EXISTS idx_memories_energy ON memories(energy);
CREATE INDEX IF NOT EXISTS idx_memories_last_energy ON memories(last_energy_update);
