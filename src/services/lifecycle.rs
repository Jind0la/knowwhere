use std::sync::Arc;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::memory::dream::energy_decay::{
    CompressionResult, DecayResult, EnergyDecayWorker, MemoryEnergyInfo,
};

#[derive(Clone)]
pub struct LifecycleService {
    pool: Arc<PgPool>,
}

impl LifecycleService {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    fn worker(&self) -> EnergyDecayWorker<'_> {
        EnergyDecayWorker::with_defaults(self.pool.as_ref())
    }

    pub async fn boost_energy(&self, id: Uuid, boost: i32) -> Result<()> {
        self.worker().boost_energy(id, boost).await
    }

    pub async fn list_low_energy(&self, limit: i32) -> Result<Vec<MemoryEnergyInfo>> {
        self.worker().find_low_energy_memories(limit).await
    }

    pub async fn apply_decay(&self) -> Result<DecayResult> {
        self.worker().apply_decay().await
    }

    pub async fn compress_cluster(&self, memory_ids: &[Uuid]) -> Result<CompressionResult> {
        self.worker().compress_cluster(memory_ids).await
    }
}
