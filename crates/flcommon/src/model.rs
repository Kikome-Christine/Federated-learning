//! The federated workload: logistic regression on a seeded, synthetic,
//! imbalanced "fraud" dataset, non-IID across institutions (each institution
//! has its own fraud prevalence). Pure Rust, no ML crates, fully deterministic.
//!


use crate::rng::{derive_seed, SplitMix64};

pub struct Dataset {
    pub x: Vec<f32>, // row-major, n * dim
    pub y: Vec<u8>,
    pub n: usize,
    pub dim: usize,
}

/// Deterministic per-institution fraud prevalence in [0.02, 0.12).
pub fn fraud_rate_for(id: &str) -> f64 {
    let h = derive_seed(0xF7A0D, id);
    0.02 + 0.10 * ((h % 1000) as f64 / 1000.0)
}

/// Generate a dataset. The first `min(dim, 16)` features are informative
/// (mean shift of +1.0 for the positive class); the rest are noise. The
/// shift is identical for every institution, so federation helps.
pub fn gen_dataset(seed: u64, n: usize, dim: usize, fraud_rate: f64) -> Dataset {
    let mut rng = SplitMix64::new(seed);
    let informative = dim.min(16);
    let mut x = Vec::with_capacity(n * dim);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        let label: u8 = if rng.next_f64() < fraud_rate { 1 } else { 0 };
        for j in 0..dim {
            let shift = if label == 1 && j < informative { 1.0 } else { 0.0 };
            x.push((rng.next_normal() + shift) as f32);
        }
        y.push(label);
    }
    Dataset { x, y, n, dim }
}

/// Dataset owned by institution `id` (same call in participant and in the
/// centralized baseline, so the baselines see identical data).
pub fn institution_dataset(data_seed: u64, id: &str, n: usize, dim: usize) -> Dataset {
    gen_dataset(derive_seed(data_seed, id), n, dim, fraud_rate_for(id))
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// Full-batch gradient descent. `w` has length dim + 1 (last element = bias).
/// Returns the training loss of the last epoch.
pub fn train_local(ds: &Dataset, w: &mut [f32], epochs: u32, lr: f32) -> f32 {
    let d = ds.dim;
    assert_eq!(w.len(), d + 1, "weight vector must be dim + 1");
    let mut last_loss = 0.0f32;
    for _ in 0..epochs {
        let mut grad = vec![0.0f32; d + 1];
        let mut loss = 0.0f32;
        for i in 0..ds.n {
            let xi = &ds.x[i * d..(i + 1) * d];
            let mut z = w[d];
            for j in 0..d {
                z += w[j] * xi[j];
            }
            let p = sigmoid(z);
            let y = ds.y[i] as f32;
            loss += -(y * p.max(1e-7).ln() + (1.0 - y) * (1.0 - p).max(1e-7).ln());
            let err = p - y;
            for j in 0..d {
                grad[j] += err * xi[j];
            }
            grad[d] += err;
        }
        let inv = 1.0 / (ds.n as f32);
        for j in 0..=d {
            w[j] -= lr * grad[j] * inv;
        }
        last_loss = loss * inv;
    }
    last_loss
}

/// Mean log-loss of `w` on `ds`.
pub fn eval_loss(ds: &Dataset, w: &[f32]) -> f32 {
    let d = ds.dim;
    assert_eq!(w.len(), d + 1);
    let mut loss = 0.0f32;
    for i in 0..ds.n {
        let xi = &ds.x[i * d..(i + 1) * d];
        let mut z = w[d];
        for j in 0..d {
            z += w[j] * xi[j];
        }
        let p = sigmoid(z);
        let y = ds.y[i] as f32;
        loss += -(y * p.max(1e-7).ln() + (1.0 - y) * (1.0 - p).max(1e-7).ln());
    }
    loss / (ds.n as f32)
}

/// FedAvg: sample-count-weighted mean of the given weight vectors.
/// Callers should sort inputs by participant id first so that floating-point
/// summation order (and therefore the result) is deterministic.
pub fn fedavg(updates: &[(Vec<f32>, u64)]) -> Option<Vec<f32>> {
    let total: u64 = updates.iter().map(|(_, n)| *n).sum();
    if updates.is_empty() || total == 0 {
        return None;
    }
    let len = updates[0].0.len();
    let mut out = vec![0.0f32; len];
    for (w, n) in updates {
        if w.len() != len {
            return None;
        }
        let f = (*n as f32) / (total as f32);
        for (o, v) in out.iter_mut().zip(w.iter()) {
            *o += f * *v;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_is_deterministic() {
        let a = gen_dataset(1, 50, 8, 0.1);
        let b = gen_dataset(1, 50, 8, 0.1);
        assert_eq!(a.y, b.y);
        assert_eq!(a.x, b.x);
    }

    #[test]
    fn training_reduces_loss() {
        let ds = gen_dataset(5, 400, 32, 0.3);
        let mut w = vec![0.0f32; 33];
        let before = eval_loss(&ds, &w);
        train_local(&ds, &mut w, 30, 0.5);
        let after = eval_loss(&ds, &w);
        assert!(after < before, "loss should fall: {before} -> {after}");
    }

    #[test]
    fn fedavg_weighted_mean() {
        let u = vec![(vec![1.0, 1.0], 1u64), (vec![3.0, 5.0], 3u64)];
        let m = fedavg(&u).unwrap();
        assert!((m[0] - 2.5).abs() < 1e-6);
        assert!((m[1] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn fedavg_rejects_empty_and_mismatched() {
        assert!(fedavg(&[]).is_none());
        let u = vec![(vec![1.0], 1u64), (vec![1.0, 2.0], 1u64)];
        assert!(fedavg(&u).is_none());
    }
}
