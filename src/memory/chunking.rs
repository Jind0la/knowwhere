//! Smart Text Chunker for Context Management (WP3)
//!
//! Splits long text into semantically-coherent chunks with configurable overlap.
//! Designed to work with nomic-embed-text (8192 token context window) and
//! other embedding models. Each chunk preserves enough context to be independently
//! retrievable while maintaining parent-child relationships for fractal zoom.
//!
//! # Design Goals
//!
//! - **Semantic coherence**: Split on sentence/paragraph boundaries, not arbitrary positions
//! - **Context preservation**: Overlap between chunks prevents information loss at boundaries
//! - **Embedding-aware**: Chunks sized to fit within model context windows with margin
//! - **Zero data loss**: Every byte of input appears in at least one chunk
//! - **Metadata-rich**: Track chunk position, parent relationships, and boundaries

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// A single chunk produced by the TextChunker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    /// The chunk content (with overlap from previous chunk).
    pub content: String,
    /// 0-based index of this chunk within the parent text.
    pub chunk_index: usize,
    /// Total number of chunks the parent text was split into.
    pub total_chunks: usize,
    /// Character offset where this chunk starts in the original text.
    pub char_offset: usize,
    /// Character length of this chunk's non-overlap content.
    pub non_overlap_len: usize,
    /// ID of the parent node (set after storage).
    #[serde(skip)]
    pub parent_node_id: Option<Uuid>,
}

impl TextChunk {
    /// Build metadata HashMap for a FractalNode created from this chunk.
    pub fn to_metadata(&self) -> HashMap<String, Value> {
        let mut meta = HashMap::new();
        meta.insert(
            "chunk_index".to_string(),
            Value::Number(self.chunk_index.into()),
        );
        meta.insert(
            "total_chunks".to_string(),
            Value::Number(self.total_chunks.into()),
        );
        meta.insert(
            "char_offset".to_string(),
            Value::Number(self.char_offset.into()),
        );
        meta.insert(
            "non_overlap_len".to_string(),
            Value::Number(self.non_overlap_len.into()),
        );
        meta.insert("is_chunk".to_string(), Value::Bool(self.total_chunks > 1));
        if let Some(parent_id) = self.parent_node_id {
            meta.insert(
                "chunk_parent_id".to_string(),
                Value::String(parent_id.to_string()),
            );
        }
        meta
    }

    /// Returns true if this chunk has neighbors (not isolated).
    pub fn has_neighbors(&self) -> bool {
        self.total_chunks > 1
    }

    /// Returns the chunk index of the previous chunk, if any.
    pub fn prev_index(&self) -> Option<usize> {
        if self.chunk_index > 0 {
            Some(self.chunk_index - 1)
        } else {
            None
        }
    }

    /// Returns the chunk index of the next chunk, if any.
    pub fn next_index(&self) -> Option<usize> {
        if self.chunk_index + 1 < self.total_chunks {
            Some(self.chunk_index + 1)
        } else {
            None
        }
    }
}

/// Configuration for the TextChunker.
#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    /// Maximum characters per chunk (before overlap).
    /// Default: 6000 (fits within 8192 token context for English text,
    /// assuming ~4 chars/token → ~1500 tokens, well under the limit).
    pub max_chunk_chars: usize,

    /// Overlap characters between adjacent chunks.
    /// Default: 200 — enough to catch a sentence or two of context.
    pub overlap_chars: usize,

    /// Minimum characters for a chunk to be considered valid.
    /// If the last chunk is shorter than this, it's merged into the previous one.
    /// Default: 100 — one short sentence minimum.
    pub min_chunk_chars: usize,

    /// Whether to prefer sentence boundaries for splits.
    /// When true, the split point is adjusted to the nearest sentence end
    /// within a window of ±max_chunk_chars/10.
    /// Default: true.
    pub split_on_sentences: bool,

    /// Whether to prefer paragraph boundaries for splits.
    /// Takes precedence over sentence boundaries.
    /// Default: true.
    pub split_on_paragraphs: bool,

    /// Maximum window (in chars) to search for a better split point.
    /// Default: 500.
    pub split_window: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            max_chunk_chars: 6000,
            overlap_chars: 200,
            min_chunk_chars: 100,
            split_on_sentences: true,
            split_on_paragraphs: true,
            split_window: 500,
        }
    }
}

impl ChunkerConfig {
    /// Configuration optimized for nomic-embed-text with 8192 token context.
    /// Leaves margin for the search_document prefix and embedding overhead.
    pub fn for_nomic_8192() -> Self {
        Self {
            max_chunk_chars: 6000,
            overlap_chars: 200,
            min_chunk_chars: 100,
            split_on_sentences: true,
            split_on_paragraphs: true,
            split_window: 500,
        }
    }

    /// Configuration optimized for models with 2048 token context.
    /// More aggressive splitting to stay within the smaller window.
    pub fn for_small_ctx() -> Self {
        Self {
            max_chunk_chars: 1500,
            overlap_chars: 150,
            min_chunk_chars: 80,
            split_on_sentences: true,
            split_on_paragraphs: true,
            split_window: 300,
        }
    }

    /// Configuration for very long documents (32k context models).
    pub fn for_large_ctx() -> Self {
        Self {
            max_chunk_chars: 24000,
            overlap_chars: 400,
            min_chunk_chars: 200,
            split_on_sentences: true,
            split_on_paragraphs: true,
            split_window: 1000,
        }
    }
}

/// The smart text chunker.
///
/// Splits text into chunks using sentence/paragraph boundary detection
/// with configurable overlap. Handles edge cases like very short texts,
/// texts with no clear boundaries, and multi-paragraph content.
#[derive(Debug, Clone)]
pub struct TextChunker {
    config: ChunkerConfig,
}

impl TextChunker {
    /// Create a new TextChunker with the given configuration.
    pub fn new(config: ChunkerConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration.
    pub fn default() -> Self {
        Self::new(ChunkerConfig::default())
    }

    /// Check if the given text needs chunking.
    /// Returns true if text length exceeds max_chunk_chars + overlap.
    pub fn needs_chunking(&self, text: &str) -> bool {
        text.len() > (self.config.max_chunk_chars + self.config.overlap_chars)
    }

    /// Split text into chunks with boundary detection and overlap.
    ///
    /// Algorithm:
    /// 1. If text is short enough, return a single chunk.
    /// 2. Find natural split points (paragraph > sentence > word boundary).
    /// 3. Split with overlap between adjacent chunks.
    /// 4. If the last chunk is too short, merge into the previous one.
    pub fn chunk(&self, text: &str) -> Vec<TextChunk> {
        if text.is_empty() {
            return vec![];
        }

        if !self.needs_chunking(text) {
            return vec![TextChunk {
                content: text.to_string(),
                chunk_index: 0,
                total_chunks: 1,
                char_offset: 0,
                non_overlap_len: text.len(),
                parent_node_id: None,
            }];
        }

        let split_points = self.find_split_points(text);
        let mut chunks = Vec::new();

        let mut start = 0usize;
        for (i, &split_at) in split_points.iter().enumerate() {
            // Add overlap from previous chunk end
            let overlap_start = if i > 0 {
                let prev_end = split_points[i - 1];
                prev_end.saturating_sub(self.config.overlap_chars)
            } else {
                0
            };

            let chunk_end = split_at;
            let chunk_content = &text[overlap_start..chunk_end.min(text.len())];
            let non_overlap = if i == 0 {
                chunk_end // First chunk: no overlap
            } else if i == split_points.len() - 1 {
                // Last chunk: from after previous overlap to end
                let prev_end = split_points[i - 1];
                chunk_end.saturating_sub(prev_end)
            } else {
                // Middle chunks: from previous split point to current
                let prev_end = split_points[i - 1];
                chunk_end.saturating_sub(prev_end)
            };

            chunks.push(TextChunk {
                content: chunk_content.to_string(),
                chunk_index: i,
                total_chunks: 0, // Will be set after all chunks are created
                char_offset: overlap_start,
                non_overlap_len: non_overlap,
                parent_node_id: None,
            });

            start = chunk_end;
        }

        // Handle remaining text after the last split point
        if start < text.len() {
            let remaining = &text[start..];
            if !remaining.trim().is_empty() {
                let overlap_start = if !split_points.is_empty() {
                    split_points
                        .last()
                        .copied()
                        .unwrap_or(start)
                        .saturating_sub(self.config.overlap_chars)
                } else {
                    start
                };

                let chunk_content = &text[overlap_start..];
                let non_overlap = text.len().saturating_sub(start);

                chunks.push(TextChunk {
                    content: chunk_content.to_string(),
                    chunk_index: chunks.len(),
                    total_chunks: 0,
                    char_offset: overlap_start,
                    non_overlap_len: non_overlap,
                    parent_node_id: None,
                });
            }
        }

        // Merge trailing stub chunks into the previous one
        self.merge_stub_chunks(&mut chunks);

        // Set total_chunks for all
        let total = chunks.len();
        for chunk in &mut chunks {
            chunk.total_chunks = total;
        }

        chunks
    }

    /// Find natural split points in the text.
    ///
    /// Searches for paragraph breaks first, then sentence boundaries,
    /// then falls back to word boundaries. Each split point is at most
    /// max_chunk_chars characters from the previous one.
    fn find_split_points(&self, text: &str) -> Vec<usize> {
        let mut points = Vec::new();
        let mut cursor = 0usize;
        let text_len = text.len();

        while cursor + self.config.max_chunk_chars < text_len {
            let ideal_split = cursor + self.config.max_chunk_chars;
            let search_end_raw = (ideal_split + self.config.split_window).min(text_len);
            let search_start_raw = if cursor > self.config.split_window {
                ideal_split.saturating_sub(self.config.split_window)
            } else {
                cursor
            };

            // Ensure byte indices land on valid UTF-8 character boundaries.
            // Multi-byte characters (e.g. CJK) can straddle byte-index calculations.
            let search_start = text.floor_char_boundary(search_start_raw);
            let search_end = text.ceil_char_boundary(search_end_raw);

            let split_at =
                self.find_best_split(&text[search_start..search_end], search_start, ideal_split);

            points.push(split_at);
            cursor = split_at;
        }

        points
    }

    /// Find the best split point within a window, preferring natural boundaries.
    fn find_best_split(&self, window: &str, window_offset: usize, ideal_split: usize) -> usize {
        if self.config.split_on_paragraphs {
            if let Some(offset) = self.find_paragraph_boundary(window, window_offset, ideal_split) {
                return offset;
            }
        }

        if self.config.split_on_sentences {
            if let Some(offset) = self.find_sentence_boundary(window, window_offset, ideal_split) {
                return offset;
            }
        }

        // Fallback: find a word boundary (space) near the ideal split
        self.find_word_boundary(window, window_offset, ideal_split)
    }

    /// Find a paragraph boundary (double newline or heading) near the ideal split.
    fn find_paragraph_boundary(&self, window: &str, offset: usize, ideal: usize) -> Option<usize> {
        // Find double newlines
        let window_ideal = ideal.saturating_sub(offset);
        let search_start = window_ideal.saturating_sub(self.config.split_window / 2);

        // Look for \n\n or \n# (markdown heading) from the left side
        if let Some(pos) = window[search_start..].find("\n\n") {
            let abs_pos = offset + search_start + pos;
            return Some(abs_pos + 2); // After the double newline
        }

        // Look for \n# (markdown heading)
        if let Some(pos) = window[search_start..].find("\n#") {
            let abs_pos = offset + search_start + pos;
            return Some(abs_pos); // Before the heading (heading starts new chunk)
        }

        // Look for a single newline (paragraph boundary) near ideal
        if let Some(pos) = window[search_start..].find('\n') {
            let abs_pos = offset + search_start + pos;
            if (abs_pos as isize - ideal as isize).abs() < (self.config.split_window / 4) as isize {
                return Some(abs_pos + 1);
            }
        }

        None
    }

    /// Find a sentence boundary near the ideal split.
    fn find_sentence_boundary(&self, window: &str, offset: usize, ideal: usize) -> Option<usize> {
        let window_ideal = ideal.saturating_sub(offset);
        let search_start = window_ideal.saturating_sub(self.config.split_window / 2);

        let sentence_ends = [". ", "! ", "? ", ".\n", "!\n", "?\n"];

        // Find the sentence end closest to the ideal split (from left)
        let mut best: Option<(usize, isize)> = None;

        for end_marker in &sentence_ends {
            let mut search_pos = search_start;
            while let Some(pos) = window[search_pos..].find(end_marker) {
                let abs_pos = offset + search_pos + pos + end_marker.len();
                let distance = (abs_pos as isize - ideal as isize).abs();

                match best {
                    None => best = Some((abs_pos, distance)),
                    Some((_, best_dist)) if distance < best_dist => {
                        best = Some((abs_pos, distance));
                    }
                    _ => {}
                }

                search_pos += pos + 1;
                if search_pos >= window.len() {
                    break;
                }
            }
        }

        // Also check for sentence endings at the start of the window
        // (for the case where the ideal split falls right after a sentence)
        if window_ideal < window.len() && window_ideal > 0 {
            let at_ideal = &window[window_ideal.saturating_sub(3)..window_ideal.min(window.len())];
            for end_marker in &sentence_ends {
                if at_ideal.ends_with(&end_marker[..end_marker.len().saturating_sub(1)]) {
                    return Some(ideal + 1); // After the punctuation + space
                }
            }
        }

        best.map(|(pos, _)| pos)
    }

    /// Fallback: find a word boundary (space) near the ideal split.
    fn find_word_boundary(&self, window: &str, offset: usize, ideal: usize) -> usize {
        let window_ideal = ideal.saturating_sub(offset);

        // Try to find a space to the left of the ideal split (prefer not breaking words)
        let before = &window[..window_ideal.min(window.len())];
        if let Some(pos) = before.rfind(' ') {
            offset + pos + 1 // After the space
        } else if window_ideal < window.len() {
            // Try to find a space to the right
            if let Some(pos) = window[window_ideal..].find(' ') {
                offset + window_ideal + pos + 1
            } else {
                ideal // Just cut at ideal position
            }
        } else {
            ideal
        }
    }

    /// Merge stub chunks (too short last chunk) into the previous one.
    fn merge_stub_chunks(&self, chunks: &mut Vec<TextChunk>) {
        if chunks.len() < 2 {
            return;
        }

        // Check if the last chunk is too short
        let last_idx = chunks.len() - 1;
        if chunks[last_idx].non_overlap_len < self.config.min_chunk_chars {
            // Merge last chunk into the previous one
            if let Some(last) = chunks.pop() {
                if let Some(prev) = chunks.last_mut() {
                    // Extend previous chunk to include the stub
                    let merged_content = format!(
                        "{}{}",
                        prev.content,
                        &last.content[last.content.len().saturating_sub(last.non_overlap_len)..]
                    );
                    prev.content = merged_content;
                    prev.non_overlap_len += last.non_overlap_len;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_text() {
        let chunker = TextChunker::default();
        let chunks = chunker.chunk("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_short_text_single_chunk() {
        let chunker = TextChunker::default();
        let text = "This is a short text.";
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].total_chunks, 1);
    }

    #[test]
    fn test_needs_chunking() {
        let chunker = TextChunker::default();
        let short = "x".repeat(1000);
        assert!(!chunker.needs_chunking(&short));

        let long = "x".repeat(7000);
        assert!(chunker.needs_chunking(&long));
    }

    #[test]
    fn test_long_text_chunked() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 100,
            overlap_chars: 20,
            min_chunk_chars: 30,
            split_on_sentences: false,
            split_on_paragraphs: false,
            split_window: 20,
        });

        let text = "x".repeat(350);
        let chunks = chunker.chunk(&text);
        assert!(
            chunks.len() >= 3,
            "Expected at least 3 chunks, got {}",
            chunks.len()
        );

        // Verify chunk ordering
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
            assert_eq!(chunk.total_chunks, chunks.len());
        }

        // Verify total coverage: all original text should be covered by non-overlap portions
        let total_non_overlap: usize = chunks.iter().map(|c| c.non_overlap_len).sum();
        assert_eq!(
            total_non_overlap,
            text.len(),
            "Total non-overlap ({}) should equal text length ({})",
            total_non_overlap,
            text.len()
        );
    }

    #[test]
    fn test_sentence_boundary_split() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 60,
            overlap_chars: 10,
            min_chunk_chars: 15,
            split_on_sentences: true,
            split_on_paragraphs: false,
            split_window: 40,
        });

        // Longer text to force multiple chunks
        let text = "First sentence here. Second sentence goes on a bit longer now. Third sentence continues with more words. Fourth sentence also adds content. Fifth and final one.";
        let chunks = chunker.chunk(&text);

        // Should split at sentence boundaries
        assert!(
            chunks.len() >= 2,
            "Expected multiple chunks with sentence splitting, got {}: {:?}",
            chunks.len(),
            chunks.iter().map(|c| &c.content).collect::<Vec<_>>()
        );
        for chunk in &chunks {
            println!("Chunk {}: '{}'", chunk.chunk_index, chunk.content);
        }
    }

    #[test]
    fn test_paragraph_boundary_split() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 60,
            overlap_chars: 10,
            min_chunk_chars: 15,
            split_on_sentences: true,
            split_on_paragraphs: true,
            split_window: 30,
        });

        let text =
            "First paragraph content.\n\nSecond paragraph with more text.\n\nThird paragraph here.";
        let chunks = chunker.chunk(&text);

        assert!(
            chunks.len() >= 2,
            "Expected multiple chunks with paragraph splitting"
        );
        // Each chunk should start near a paragraph boundary
        for chunk in &chunks {
            println!(
                "Chunk {} (offset={}, non_overlap={}): '{}'",
                chunk.chunk_index, chunk.char_offset, chunk.non_overlap_len, chunk.content
            );
        }
    }

    #[test]
    fn test_chunk_metadata() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 100,
            overlap_chars: 20,
            min_chunk_chars: 30,
            split_on_sentences: false,
            split_on_paragraphs: false,
            split_window: 20,
        });

        let text = "x".repeat(350);
        let chunks = chunker.chunk(&text);

        for chunk in &chunks {
            let meta = chunk.to_metadata();
            assert_eq!(
                meta["chunk_index"].as_i64().unwrap(),
                chunk.chunk_index as i64
            );
            assert_eq!(
                meta["total_chunks"].as_i64().unwrap(),
                chunk.total_chunks as i64
            );
            assert_eq!(
                meta["char_offset"].as_i64().unwrap(),
                chunk.char_offset as i64
            );
            assert_eq!(meta["is_chunk"].as_bool().unwrap(), chunk.has_neighbors());
        }
    }

    #[test]
    fn test_has_neighbors() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 100,
            overlap_chars: 20,
            min_chunk_chars: 30,
            split_on_sentences: false,
            split_on_paragraphs: false,
            split_window: 20,
        });

        let text = "x".repeat(350);
        let chunks = chunker.chunk(&text);

        assert!(chunks[0].has_neighbors());
        assert_eq!(chunks[0].prev_index(), None);
        assert_eq!(chunks[0].next_index(), Some(1));

        let mid = chunks.len() / 2;
        assert!(chunks[mid].has_neighbors());
        assert_eq!(chunks[mid].prev_index(), Some(mid - 1));
        assert_eq!(chunks[mid].next_index(), Some(mid + 1));

        let last = chunks.len() - 1;
        assert!(chunks[last].has_neighbors());
        assert_eq!(chunks[last].prev_index(), Some(last - 1));
        assert_eq!(chunks[last].next_index(), None);
    }

    #[test]
    fn test_stub_chunk_merged() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 100,
            overlap_chars: 20,
            min_chunk_chars: 50, // High min to force merging
            split_on_sentences: false,
            split_on_paragraphs: false,
            split_window: 20,
        });

        // Text that would produce a small final chunk
        let text = "x".repeat(230); // 100 + 100 + 30 → last chunk is 30 < 50 min
        let chunks = chunker.chunk(&text);

        // Should have merged the stub, so fewer chunks
        assert!(
            chunks.len() <= 2,
            "Expected ≤2 chunks after merging, got {}",
            chunks.len()
        );

        // Total non-overlap should still cover all text
        let total_non_overlap: usize = chunks.iter().map(|c| c.non_overlap_len).sum();
        assert_eq!(total_non_overlap, text.len());
    }

    #[test]
    fn test_nomic_8192_config() {
        let config = ChunkerConfig::for_nomic_8192();
        assert_eq!(config.max_chunk_chars, 6000);
        assert_eq!(config.overlap_chars, 200);
        assert!(config.split_on_sentences);
        assert!(config.split_on_paragraphs);
    }

    #[test]
    fn test_small_ctx_config() {
        let config = ChunkerConfig::for_small_ctx();
        assert_eq!(config.max_chunk_chars, 1500);
        // Small ctx should produce more chunks for the same text
        let chunker = TextChunker::new(config);
        let text = "x".repeat(5000);
        let chunks = chunker.chunk(&text);
        assert!(chunks.len() >= 3, "Small ctx should produce many chunks");
    }

    #[test]
    fn test_chunk_preserves_content() {
        let chunker = TextChunker::new(ChunkerConfig {
            max_chunk_chars: 100,
            overlap_chars: 20,
            min_chunk_chars: 30,
            split_on_sentences: false,
            split_on_paragraphs: false,
            split_window: 20,
        });

        let text = "IMPORTANT: The secret code is XYZ-123. END.";
        let chunks = chunker.chunk(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }
}
