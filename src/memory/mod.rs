pub mod dream;
pub mod fractal_node;

#[cfg(test)]
mod tests;

pub use dream::DreamMode;
pub use fractal_node::{cosine_similarity, FractalNode, Relation};
