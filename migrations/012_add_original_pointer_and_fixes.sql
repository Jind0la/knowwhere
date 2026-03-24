-- Migration to add missing `original_pointer` column and fix SQLx type mismatches in the `memories` table.

-- Step 1: Add `original_pointer` column
ALTER TABLE memories
ADD COLUMN original_pointer VARCHAR(255);

-- Step 2: Fix SQLx type mismatches (example adjustments, please customize accordingly)
-- These fixes depend on the current schema and types. Here are placeholders:

-- Example adjustment for `timestamp` type
ALTER TABLE memories
ALTER COLUMN created_at SET DATA TYPE TIMESTAMP WITHOUT TIME ZONE;

-- Example adjustment for `user_id` to match an expected type
ALTER TABLE memories
ALTER COLUMN user_id SET DATA TYPE INTEGER;

-- Additional adjustments can be added here as necessary.