# Data-residency / eligibility policy (legal-regulatory sovereignty).
# MUST stay equivalent to flcommon::select::Policy::check (Rust reference
# implementation). The scheduler counts native-vs-OPA disagreements per call
# (SelectResponse.policy_disagreements); the analysis reports that count.
package fl.residency

import rego.v1

default allow := false

allowed_jurisdictions := {"UG"}

allowed_zones := {"ug-central"}

# `reasons` is a partial set: every violated rule contributes one reason.
reasons contains "jurisdiction_not_permitted" if not input.jurisdiction in allowed_jurisdictions

reasons contains "zone_not_permitted" if not input.residency_zone in allowed_zones

reasons contains "license_not_active" if input.license_status != "active"

# Fail closed: a node is eligible only if NO rule produced a reason.
allow if count(reasons) == 0
