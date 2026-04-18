use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

use graph_core::InfluenceKindId;
use rustc_hash::FxHashMap;

pub trait Learnable {
    type Observation: Copy;

    fn initial() -> f32;
    fn clamp_range() -> (f32, f32);
    fn step(current: f32, obs: Self::Observation) -> f32;
}

pub struct PerKindLearnable<L: Learnable> {
    scales: FxHashMap<InfluenceKindId, AtomicU32>,
    _phantom: PhantomData<L>,
}

impl<L: Learnable> Default for PerKindLearnable<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: Learnable> PerKindLearnable<L> {
    pub fn new() -> Self {
        Self {
            scales: FxHashMap::default(),
            _phantom: PhantomData,
        }
    }

    pub fn register(&mut self, kind: InfluenceKindId) {
        self.scales
            .entry(kind)
            .or_insert_with(|| AtomicU32::new(L::initial().to_bits()));
    }

    pub fn observe(&self, kind: InfluenceKindId, obs: L::Observation) {
        let Some(atomic) = self.scales.get(&kind) else {
            return;
        };
        let current = f32::from_bits(atomic.load(Ordering::Relaxed));
        let next = L::step(current, obs);
        let (floor, ceil) = L::clamp_range();
        atomic.store(next.clamp(floor, ceil).to_bits(), Ordering::Relaxed);
    }

    pub fn current(&self, kind: InfluenceKindId) -> f32 {
        self.scales
            .get(&kind)
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)))
            .unwrap_or_else(L::initial)
    }

    pub fn reset(&self, kind: InfluenceKindId) {
        if let Some(atomic) = self.scales.get(&kind) {
            atomic.store(L::initial().to_bits(), Ordering::Relaxed);
        }
    }

    pub fn reset_all(&self) {
        for atomic in self.scales.values() {
            atomic.store(L::initial().to_bits(), Ordering::Relaxed);
        }
    }
}
