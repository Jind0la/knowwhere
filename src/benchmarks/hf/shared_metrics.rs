use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvalMetrics {
    pub total: usize,
    pub exact_match: f64,
    pub abstention_accuracy: f64,
    pub recall_at_5: f64,
    pub mrr: f64,
}

#[derive(Debug, Clone, Default)]
pub struct EvalCounters {
    pub total: usize,
    pub retrieval_total: usize,
    pub exact_hits: usize,
    pub abstention_total: usize,
    pub abstention_hits: usize,
    pub recall_hits: usize,
    pub reciprocal_rank_sum: f64,
}

impl EvalCounters {
    pub fn to_metrics(&self) -> EvalMetrics {
        let total = self.total.max(1) as f64;
        let retrieval_total = self.retrieval_total.max(1) as f64;
        let abst_total = self.abstention_total.max(1) as f64;
        EvalMetrics {
            total: self.total,
            exact_match: self.exact_hits as f64 / total,
            abstention_accuracy: self.abstention_hits as f64 / abst_total,
            recall_at_5: self.recall_hits as f64 / retrieval_total,
            mrr: self.reciprocal_rank_sum / retrieval_total,
        }
    }

    pub fn register_exact(&mut self, is_hit: bool) {
        self.total += 1;
        if is_hit {
            self.exact_hits += 1;
        }
    }

    pub fn register_abstention(&mut self, expected: bool, predicted: bool) {
        if !expected {
            return;
        }
        self.abstention_total += 1;
        if predicted {
            self.abstention_hits += 1;
        }
    }

    pub fn register_rank(&mut self, rank: Option<usize>) {
        self.retrieval_total += 1;
        if let Some(pos) = rank {
            self.recall_hits += 1;
            self.reciprocal_rank_sum += 1.0 / pos as f64;
        }
    }
}
