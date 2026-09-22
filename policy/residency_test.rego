package fl.residency_test

import rego.v1

import data.fl.residency

good := {"jurisdiction": "UG", "residency_zone": "ug-central", "license_status": "active"}

test_eligible_node_allowed if residency.allow with input as good

test_foreign_jurisdiction_denied if not residency.allow with input as object.union(good, {"jurisdiction": "XX"})

test_foreign_zone_denied if not residency.allow with input as object.union(good, {"residency_zone": "xx-1"})

test_revoked_license_denied if not residency.allow with input as object.union(good, {"license_status": "revoked"})

test_missing_fields_fail_closed if not residency.allow with input as {}
