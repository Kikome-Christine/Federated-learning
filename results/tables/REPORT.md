# Analysis report (generated)

## Hypothesis verdicts (mechanical, per pre-registered decision rules)

| tier   | hyp      | scope          | statement                                        | result                                                             | verdict      |
|:-------|:---------|:---------------|:-------------------------------------------------|:-------------------------------------------------------------------|:-------------|
| prod   | H1       | all conditions | policy_resource admits 0 ineligible participants | 0 violations (ground truth), 0 (attestation audit) over 540 rounds | supported    |
| prod   | H1-check | resource_only  | policy-blind baseline DOES violate when f>0      | 0 violations                                                       | CHECK DESIGN |
| prod   | H1-check | random         | policy-blind baseline DOES violate when f>0      | 0 violations                                                       | CHECK DESIGN |
| sim    | H1       | all conditions | policy_resource admits 0 ineligible participants | 0 violations (ground truth), 0 (attestation audit) over 540 rounds | supported    |
| sim    | H1-check | resource_only  | policy-blind baseline DOES violate when f>0      | 0 violations                                                       | CHECK DESIGN |
| sim    | H1-check | random         | policy-blind baseline DOES violate when f>0      | 0 violations                                                       | CHECK DESIGN |

## Sim-vs-production / cross-tier comparison (tolerance +-20 %)

| cond_a       | cond_b      | metric                     |   value_a |    value_b |   rel_diff | same_direction   | reproduces   |
|:-------------|:------------|:---------------------------|----------:|-----------:|-----------:|:-----------------|:-------------|
| prod:default | sim:default | p99_policy                 | 51.1643   | 135.863    |   1.65543  | True             | False        |
| prod:default | sim:default | p99_ratio_vs_random        |  1.01998  |   0.588575 |  -0.422956 | False            | False        |
| prod:default | sim:default | p99_ratio_vs_resource_only |  0.891493 |   0.998751 |   0.120313 | True             | True         |
