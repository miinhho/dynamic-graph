use crate::regime::DynamicsRegime;

use super::framework::Learnable;

pub(super) struct RegimeAlphaScale;

const MIN_SCALE: f32 = 0.1;
const MAX_SCALE: f32 = 1.0;
const SHRINK_FACTOR: f32 = 0.5;
const RECOVERY_FACTOR: f32 = 1.1;

impl Learnable for RegimeAlphaScale {
    type Observation = DynamicsRegime;

    fn initial() -> f32 {
        MAX_SCALE
    }

    fn clamp_range() -> (f32, f32) {
        (MIN_SCALE, MAX_SCALE)
    }

    fn step(current: f32, obs: DynamicsRegime) -> f32 {
        match obs {
            DynamicsRegime::Diverging => current * SHRINK_FACTOR,
            DynamicsRegime::Quiescent => current * RECOVERY_FACTOR,
            DynamicsRegime::Initializing
            | DynamicsRegime::Settling
            | DynamicsRegime::Oscillating
            | DynamicsRegime::LimitCycleSuspect => current,
        }
    }
}
