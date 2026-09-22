//! Participant selection (paper Section III-D).
//!
//!  1. Regulatory eligibility is a HARD FILTER applied BEFORE scoring
//!     (strategy `policy_resource` only).
//!  2. Remaining candidates get a composite suitability score from min-max
//!     NORMALISED capability, bandwidth, latency and availability.
//!  3. The top-k are selected; ties broken by id (deterministic).
//!


use crate::rng::SplitMix64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Random,
    ResourceOnly,
    PolicyResource,
}

impl Strategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "random" => Some(Strategy::Random),
            "resource_only" => Some(Strategy::ResourceOnly),
            "policy_resource" => Some(Strategy::PolicyResource),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Random => "random",
            Strategy::ResourceOnly => "resource_only",
            Strategy::PolicyResource => "policy_resource",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NodeView {
    pub id: String,
    pub reachable: bool,
    pub jurisdiction: String,
    pub license_active: bool,
    pub residency_zone: String,
    pub bench_mops: f64,
    pub bandwidth_mbps: f64,
    pub rtt_ms: f64,
    pub availability: f64, // EWMA of past success in [0, 1]
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub allowed_jurisdictions: Vec<String>,
    pub allowed_zones: Vec<String>,
}

impl Policy {
    /// Native reference implementation of the residency policy. The Rego
    /// version in policy/residency.rego must stay equivalent; the scheduler
    /// counts disagreements between the two at run time.
    /// Returns Err(reason) if the node is not eligible.
    pub fn check(&self, n: &NodeView) -> Result<(), String> {
        if !self.allowed_jurisdictions.iter().any(|j| j == &n.jurisdiction) {
            return Err("jurisdiction_not_permitted".to_string());
        }
        if !self.allowed_zones.iter().any(|z| z == &n.residency_zone) {
            return Err("zone_not_permitted".to_string());
        }
        if !n.license_active {
            return Err("license_not_active".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Weights {
    pub compute: f64,
    pub bandwidth: f64,
    pub latency: f64,
    pub availability: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Weights { compute: 0.35, bandwidth: 0.25, latency: 0.25, availability: 0.15 }
    }
}

#[derive(Debug, Default)]
pub struct Selection {
    /// (index into `nodes`, score) in selection order.
    pub selected: Vec<(usize, f64)>,
    /// (index into `nodes`, reason) for every node removed before ranking.
    pub excluded: Vec<(usize, String)>,
}

fn minmax(vals: &[f64]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in vals {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    (lo, hi)
}

fn norm(v: f64, lo: f64, hi: f64) -> f64 {
    if (hi - lo).abs() < 1e-12 {
        1.0
    } else {
        (v - lo) / (hi - lo)
    }
}

/// Select up to `k` nodes.
/// `verdicts[i]` is the policy verdict for `nodes[i]` (Ok = eligible). It is
/// consulted ONLY for `Strategy::PolicyResource`, and must have the same
/// length as `nodes` in that case.
pub fn select(
    nodes: &[NodeView],
    verdicts: &[Result<(), String>],
    k: usize,
    strategy: Strategy,
    w: &Weights,
    rng: &mut SplitMix64,
) -> Selection {
    let mut out = Selection::default();

    // ---- Random: uniform over ALL configured nodes, no probing, no policy.
    if strategy == Strategy::Random {
        let mut idx: Vec<usize> = (0..nodes.len()).collect();
        rng.shuffle(&mut idx);
        for i in idx.into_iter().take(k) {
            out.selected.push((i, 0.0));
        }
        return out;
    }

    // ---- Build the scoring pool.
    let mut pool: Vec<usize> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if !n.reachable {
            out.excluded.push((i, "unreachable".to_string()));
            continue;
        }
        if strategy == Strategy::PolicyResource {
            if let Err(reason) = &verdicts[i] {
                out.excluded.push((i, reason.clone()));
                continue;
            }
        }
        pool.push(i);
    }
    if pool.is_empty() {
        return out;
    }

    // ---- Composite score on min-max normalised features.
    let (c_lo, c_hi) = minmax(&pool.iter().map(|&i| nodes[i].bench_mops).collect::<Vec<_>>());
    let (b_lo, b_hi) = minmax(&pool.iter().map(|&i| nodes[i].bandwidth_mbps).collect::<Vec<_>>());
    let (l_lo, l_hi) = minmax(&pool.iter().map(|&i| nodes[i].rtt_ms).collect::<Vec<_>>());
    let (a_lo, a_hi) = minmax(&pool.iter().map(|&i| nodes[i].availability).collect::<Vec<_>>());

    let mut scored: Vec<(usize, f64)> = pool
        .iter()
        .map(|&i| {
            let n = &nodes[i];
            let s = w.compute * norm(n.bench_mops, c_lo, c_hi)
                + w.bandwidth * norm(n.bandwidth_mbps, b_lo, b_hi)
                + w.latency * (1.0 - norm(n.rtt_ms, l_lo, l_hi))
                + w.availability * norm(n.availability, a_lo, a_hi);
            (i, s)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| nodes[a.0].id.cmp(&nodes[b.0].id))
    });
    out.selected = scored.into_iter().take(k).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, jur: &str, mops: f64, rtt: f64) -> NodeView {
        NodeView {
            id: id.into(),
            reachable: true,
            jurisdiction: jur.into(),
            license_active: true,
            residency_zone: if jur == "UG" { "ug-central".into() } else { "xx-1".into() },
            bench_mops: mops,
            bandwidth_mbps: 50.0,
            rtt_ms: rtt,
            availability: 1.0,
        }
    }

    fn policy() -> Policy {
        Policy { allowed_jurisdictions: vec!["UG".into()], allowed_zones: vec!["ug-central".into()] }
    }

    fn verdicts(nodes: &[NodeView]) -> Vec<Result<(), String>> {
        nodes.iter().map(|n| policy().check(n)).collect()
    }

    #[test]
    fn policy_is_hard_filter_even_for_best_node() {
        let nodes = vec![node("fast_foreign", "XX", 9999.0, 1.0), node("slow_ok", "UG", 10.0, 200.0)];
        let mut rng = SplitMix64::new(1);
        let s = select(&nodes, &verdicts(&nodes), 1, Strategy::PolicyResource, &Weights::default(), &mut rng);
        assert_eq!(s.selected.len(), 1);
        assert_eq!(nodes[s.selected[0].0].id, "slow_ok");
        assert_eq!(s.excluded.len(), 1);
    }

    #[test]
    fn resource_only_ignores_policy() {
        let nodes = vec![node("fast_foreign", "XX", 9999.0, 1.0), node("slow_ok", "UG", 10.0, 200.0)];
        let mut rng = SplitMix64::new(1);
        let s = select(&nodes, &verdicts(&nodes), 1, Strategy::ResourceOnly, &Weights::default(), &mut rng);
        assert_eq!(nodes[s.selected[0].0].id, "fast_foreign");
    }

    #[test]
    fn unreachable_nodes_excluded_from_scored_strategies() {
        let mut nodes = vec![node("a", "UG", 100.0, 10.0), node("b", "UG", 200.0, 10.0)];
        nodes[1].reachable = false;
        let mut rng = SplitMix64::new(1);
        let s = select(&nodes, &verdicts(&nodes), 2, Strategy::PolicyResource, &Weights::default(), &mut rng);
        assert_eq!(s.selected.len(), 1);
        assert_eq!(nodes[s.selected[0].0].id, "a");
    }

    #[test]
    fn random_is_seed_deterministic() {
        let nodes: Vec<NodeView> = (0..8).map(|i| node(&format!("n{i}"), "UG", 100.0, 10.0)).collect();
        let v = verdicts(&nodes);
        let a = select(&nodes, &v, 4, Strategy::Random, &Weights::default(), &mut SplitMix64::new(9));
        let b = select(&nodes, &v, 4, Strategy::Random, &Weights::default(), &mut SplitMix64::new(9));
        assert_eq!(a.selected.iter().map(|x| x.0).collect::<Vec<_>>(), b.selected.iter().map(|x| x.0).collect::<Vec<_>>());
    }

    #[test]
    fn k_larger_than_pool_returns_pool() {
        let nodes = vec![node("a", "UG", 1.0, 1.0)];
        let mut rng = SplitMix64::new(1);
        let s = select(&nodes, &verdicts(&nodes), 5, Strategy::PolicyResource, &Weights::default(), &mut rng);
        assert_eq!(s.selected.len(), 1);
    }
}
