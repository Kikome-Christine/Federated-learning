//! Logic shared by the real system (participant / scheduler / coordinator)
//! and the deterministic simulator (flsim). Sharing this code is what makes
//! the simulated and production results comparable: both tiers run the SAME
//! selection and aggregation code, and differ only in the environment.
pub mod model;
pub mod rng;
pub mod select;
