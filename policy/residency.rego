# Data-residency / eligibility policy (legal-regulatory sovereignty).
# Enforces absolute compliance checks matching Section 19 of the Uganda DPPA (2019).
# Must stay structurally equivalent to flcommon::select::Policy::check.
package fl.residency

import rego.v1

default allow := false

# Permitted regulatory boundaries
allowed_jurisdictions := {"UG"}
allowed_zones := {"ug-central"}

# Adversarial Filtering Engine: Tracks every non-compliant violation attribute
reasons contains "jurisdiction_not_permitted" if not input.jurisdiction in allowed_jurisdictions

reasons contains "zone_not_permitted" if not input.residency_zone in allowed_zones

reasons contains "license_not_active" if input.license_status != "active"

# Countering Spoofed/Missing Claims: Missing or forged flags trigger immediate rejection
reasons contains "missing_secure_attestation" if not input.has_valid_attestation

# Countering Stale Claims: Stale residency attributes cannot be used for training validation
reasons contains "stale_attestation_bounds" if input.attestation_is_stale == true

# Fail Closed Protocol: A node is granted entry only if zero reasons are compiled
allow if count(reasons) == 0

