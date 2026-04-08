use smallvec::SmallVec;

type InlineScalars = SmallVec<[f32; 4]>;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StateVector {
    values: InlineScalars,
}

impl StateVector {
    pub fn new(values: Vec<f32>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn first(&self) -> Option<f32> {
        self.values.first().copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn l2_norm(&self) -> f32 {
        self.values
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt()
    }

    pub fn scaled(&self, factor: f32) -> Self {
        Self::new(self.values.iter().map(|value| value * factor).collect())
    }

    pub fn add(&self, other: &Self) -> Self {
        let len = self.len().max(other.len());
        let mut values = Vec::with_capacity(len);

        for index in 0..len {
            let lhs = self.values.get(index).copied().unwrap_or_default();
            let rhs = other.values.get(index).copied().unwrap_or_default();
            values.push(lhs + rhs);
        }

        Self::new(values)
    }

    pub fn sub(&self, other: &Self) -> Self {
        let len = self.len().max(other.len());
        let mut values = Vec::with_capacity(len);

        for index in 0..len {
            let lhs = self.values.get(index).copied().unwrap_or_default();
            let rhs = other.values.get(index).copied().unwrap_or_default();
            values.push(lhs - rhs);
        }

        Self::new(values)
    }

    pub fn clamp_magnitude(&self, max_norm: f32) -> Self {
        let norm = self.l2_norm();
        if norm <= max_norm || norm == 0.0 {
            return self.clone();
        }

        self.scaled(max_norm / norm)
    }

    pub fn distance(&self, other: &Self) -> f32 {
        match (self.values.as_slice(), other.values.as_slice()) {
            ([lhs], [rhs]) => return (lhs - rhs).abs(),
            ([], []) => return 0.0,
            _ => {}
        }

        let len = self.len().max(other.len());
        let mut sum = 0.0;

        for index in 0..len {
            let lhs = self.values.get(index).copied().unwrap_or_default();
            let rhs = other.values.get(index).copied().unwrap_or_default();
            let diff = lhs - rhs;
            sum += diff * diff;
        }

        sum.sqrt()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SignalVector {
    values: InlineScalars,
}

impl SignalVector {
    pub fn new(values: Vec<f32>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn first(&self) -> Option<f32> {
        self.values.first().copied()
    }

    pub fn l2_norm(&self) -> f32 {
        self.values
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt()
    }

    pub fn scaled(&self, factor: f32) -> Self {
        Self::new(self.values.iter().map(|value| value * factor).collect())
    }

    pub fn add(&self, other: &Self) -> Self {
        let len = self.values.len().max(other.values.len());
        let mut values = Vec::with_capacity(len);

        for index in 0..len {
            let lhs = self.values.get(index).copied().unwrap_or_default();
            let rhs = other.values.get(index).copied().unwrap_or_default();
            values.push(lhs + rhs);
        }

        Self::new(values)
    }

    pub fn clamp_magnitude(&self, max_norm: f32) -> Self {
        let norm = self.l2_norm();
        if norm <= max_norm || norm == 0.0 {
            return self.clone();
        }

        self.scaled(max_norm / norm)
    }

    pub fn saturated_tanh(&self) -> Self {
        Self::new(self.values.iter().map(|value| value.tanh()).collect())
    }

    pub fn saturated_softsign(&self) -> Self {
        Self::new(
            self.values
                .iter()
                .map(|value| value / (1.0 + value.abs()))
                .collect(),
        )
    }

    pub fn component_max(&self, other: &Self) -> Self {
        let len = self.values.len().max(other.values.len());
        let mut values = Vec::with_capacity(len);

        for index in 0..len {
            let lhs = self.values.get(index).copied().unwrap_or_default();
            let rhs = other.values.get(index).copied().unwrap_or_default();
            values.push(lhs.max(rhs));
        }

        Self::new(values)
    }
}

impl From<SignalVector> for StateVector {
    fn from(value: SignalVector) -> Self {
        Self {
            values: value.values,
        }
    }
}

impl From<&SignalVector> for StateVector {
    fn from(value: &SignalVector) -> Self {
        Self {
            values: value.values.clone(),
        }
    }
}
