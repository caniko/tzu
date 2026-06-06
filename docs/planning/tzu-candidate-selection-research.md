# Tzu Candidate Selection Research Dossier

## Goal And Trigger

This research started from a selection-risk question: if `tzu` picks only the
current highest-scoring candidate each round, it can converge on the nearest
local maximum and discard another candidate basin that would have led to a
better plan after more validation or refinement. The initial suggestion was to
retain candidates by percentile, such as candidates scoring at or above the
95th percentile, instead of always selecting the single current winner.

The follow-up question was broader: whether Elo is the right selection system at
all, and which alternative adaptation should guide `tzu`.

This dossier records the evidence and architecture recommendation. It is not an
implementation plan, but it is meant to inform the next implementation phases
for `tzu`'s harness planner, candidate metadata, persistence, and CLI
observability.

Target repository: `/data/nvme0/can/Projects/tzu`.

## Core Thesis

`tzu` should not replace "winner-takes-all Elo" with another single selector.
It should move to a staged frontier architecture:

1. Hard validation gates decide which candidates are admissible.
2. Deterministic multi-candidate generation creates a real population.
3. Candidates receive a structured score vector and stable descriptors.
4. Pareto filtering and quality-diversity cells retain a capped frontier.
5. A verifier-first scalar is used only at the execution boundary to choose the
   single champion required by the current runner.
6. Successive halving and bandit allocation are deferred until validation
   outcome telemetry exists.

This preserves exploration without pretending that every tradeoff can be
honestly collapsed into one early score.

## Current Reality

The README describes `tzu` as a local-first planning harness that owns problem
specs, candidate plan sketches, task DAGs, validation, persistence, policy, and
run reports
([README.md:3](/data/nvme0/can/Projects/tzu/README.md:3)). It also says
`tzu plan` builds an immutable problem spec, seeds candidate sketches, validates
them, scores valid candidates, selects a champion, and persists the selected
plan plus harness metadata
([README.md:73](/data/nvme0/can/Projects/tzu/README.md:73)).

The implementation has the right skeleton, but not yet the search machinery:

- `GenericDomainAdapter::seed_candidates` emits a one-element candidate list
  ([crates/tzu-core/src/lib.rs:478](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:478)).
- `CodingDomainAdapter::seed_candidates` also emits a one-element candidate list
  ([crates/tzu-core/src/lib.rs:584](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:584)).
- `CandidateRating` contains `elo`, `visits`, `wins`, and `losses`, but the
  current rating update only changes `elo`
  ([crates/tzu-core/src/lib.rs:228](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:228),
  [crates/tzu-core/src/lib.rs:781](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:781)).
- `select_candidate` filters to valid candidates and chooses the maximum
  `rating.elo`
  ([crates/tzu-core/src/lib.rs:795](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:795)).
- `HarnessPlanMetadata` can store candidates and matches, but the planner
  currently stores an empty `matches` list
  ([crates/tzu-core/src/lib.rs:276](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:276),
  [crates/tzu-core/src/lib.rs:430](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:430)).
- The runner appends only summary planning-run metadata to `ProjectState`, then
  saves the whole state JSON
  ([crates/tzu-runner/src/lib.rs:145](/data/nvme0/can/Projects/tzu/crates/tzu-runner/src/lib.rs:145),
  [crates/tzu-runner/src/lib.rs:477](/data/nvme0/can/Projects/tzu/crates/tzu-runner/src/lib.rs:477)).
- SQLite/Postgres tables already exist for `plan_candidates`, `plan_matches`,
  `obligations`, `agent_runs`, and `validator_runs`, but search found no insert
  path for `plan_candidates` or `plan_matches`
  ([crates/tzu-runner/src/lib.rs:323](/data/nvme0/can/Projects/tzu/crates/tzu-runner/src/lib.rs:323),
  [crates/tzu-runner/src/lib.rs:333](/data/nvme0/can/Projects/tzu/crates/tzu-runner/src/lib.rs:333)).
- CLI status currently exposes the selected candidate and candidate count, but
  not a retained frontier
  ([crates/tzu-cli/src/main.rs:101](/data/nvme0/can/Projects/tzu/crates/tzu-cli/src/main.rs:101)).

So the current `elo` field is not Elo in the meaningful historical sense. It is
a scalar placeholder. That is not wrong as scaffolding, but it should not be
treated as a rating system.

## Lessons From AlphaProof Nexus

The local `alphaproof-nexus-results` checkout is useful evidence, but with an
important limitation. Its README states that it contains only successful outputs
([alphaproof-nexus-results/README.md:24](/data/nvme0/can/Projects/tzu/alphaproof-nexus-results/README.md:24)).
It cannot calibrate `tzu` thresholds, beam widths, or percentile cutoffs because
it does not contain full successful and failed search traces.

The AlphaProof Nexus paper is more informative for architecture. It describes:

- a generation-validation loop backed by Lean checking;
- a population database for validated sketches;
- rater subagents that rank previous attempts;
- Elo aggregation from those match results;
- P-UCB sampling to drive evolutionary search;
- failure modes where high-scoring sketches hid core difficulty in `sorry`
  lemmas or hallucinated unavailable results.

Primary source: "Advancing Mathematics Research with AI-Driven Formal Proof
Search", arXiv:2605.22763v1:
https://arxiv.org/html/2605.22763v1

The central lesson for `tzu` is not "use Elo". AlphaProof Nexus used Elo as one
component after it had ranked matches, a population database, and formal
verification signals. `tzu` does not yet have those inputs. The transferable
lesson is to combine:

- hard verifier gates;
- retained populations;
- relative or staged evidence;
- budget-aware sampling;
- final checked output.

## Design Principles

### Validation Before Selection

Hard validation gates should remain outside the search objective. Invalid
candidates are evidence and debugging material, not executable options.

The current validation rules are worth preserving:

- spec hash checks prevent silent mutation
  ([crates/tzu-core/src/lib.rs:738](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:738));
- empty summaries, missing tasks, invalid DAGs, missing verification, and hidden
  `TBD` task descriptions are rejected
  ([crates/tzu-core/src/lib.rs:747](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:747));
- candidate blockers become explicit obligations
  ([crates/tzu-core/src/lib.rs:777](/data/nvme0/can/Projects/tzu/crates/tzu-core/src/lib.rs:777)).

### Frontier Before Champion

The planning harness should preserve multiple viable candidates during search.
A champion is needed only when `tzu` commits to an executable `current_plan`.

Percentile selection can help only as a retention filter. It is not sufficient
as the main algorithm, especially with small candidate counts where a 95th
percentile threshold can degenerate to one candidate. Any percentile rule needs
`min_elite`, `max_elite`, deterministic tie-breaking, and diversity constraints.

### Structured Evidence Before Scalar Score

The selection model should keep dimensions visible. At minimum, candidate
metadata should distinguish:

- verifier strength;
- obligation burden;
- risk profile;
- cost tier;
- verifier dependency;
- task graph quality or execution readiness as tie-breakers.

Validity is not a dimension. It is an admissibility gate.

### Determinism Before ACP Generation

The first implementation should not depend on stochastic ACP/LLM output.
Deterministic fixtures and deterministic adapter-generated candidates are needed
to test Pareto filtering, beam caps, descriptor cells, and boundary tie-breaks.

ACP-backed candidate generation should come after the selector is proven with
controlled inputs.

## Recommended Architecture

### Candidate Metadata

Replace `CandidateRating::elo` as the main decision primitive with explicit
candidate selection metadata.

Recommended v1 shape:

```text
CandidateScore {
  verifier_strength: Weak | Moderate | Strong,
  obligation_burden: None | One | Many,
  risk_profile: Low | Medium | High,
  cost_tier: Low | Medium | High,
  task_graph_quality: u8,
  execution_readiness: u8,
}

CandidateDescriptor {
  cost_tier: Low | Medium | High,
  risk_profile: Low | Medium | High,
  verifier_dependency: Static | Repository | Agent,
}
```

The exact Rust names can change, but these concepts should stay separate:
scores rank evidence; descriptors preserve diversity.

### Generic Candidate Diversity

The tightest blocker is current one-candidate generation. The v1
`GenericDomainAdapter` should emit a deterministic palette of candidates with
distinct descriptor tuples.

Recommended fixed archetypes:

| Archetype            | Plan Shape                                                                                   | Descriptor Bias                                                            |
| -------------------- | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| Conservative serial  | Ground inputs, validate blockers, choose the smallest executable plan, verify.               | Low risk, low cost, static verifier dependency.                            |
| Evidence-first broad | Spend more work collecting and cross-checking evidence before narrowing options.             | Low risk, medium cost, static or repository verifier dependency.           |
| Parallel exploration | Explore multiple approaches before convergence and preserve rejected alternatives.           | Medium risk, medium or high cost, repository or agent verifier dependency. |
| Blocker-reduction    | Treat missing artifacts as the main work product and sequence around regeneration workflows. | Low risk, medium cost, static verifier dependency.                         |

The diversity guarantee should be mechanical:

- every generated candidate has a descriptor tuple;
- unit tests assert at least three distinct descriptor tuples;
- duplicate task IDs across candidates are allowed only when the surrounding DAG
  or descriptor tuple differs;
- the selector is tested against fixtures with conflicting score vectors.

Do not use semantic clustering, embeddings, graph-edit distance, or LLM
classification in v1 candidate generation.

### Pareto Filtering

Pareto filtering is useful only if the objective set is small. A naive 6-8
dimensional frontier will usually fail to prune because nearly every candidate
can be marginally better on one minor dimension.

V1 Pareto axes should be limited to:

- verifier strength, higher is better;
- obligation burden, lower is better;
- risk profile, lower is better;
- cost tier, lower is better.

Use coarse buckets or epsilon dominance:

```text
A dominates B if:
  A is no worse than B in every primary bucket, and
  A is strictly better than B in at least one primary bucket.
```

For continuous future scores, "strictly better" should mean better by a defined
epsilon, not a floating point hairline. V1 should prefer ordinal buckets because
they are easier to explain, persist, and test.

If the non-dominated set exceeds `max_elite`, truncation should be deterministic:

1. preserve descriptor-cell coverage;
2. rank by verifier-first boundary score;
3. tie-break by stable candidate ID.

This keeps Pareto filtering as a pruning layer, not the sole frontier cap.

### Quality-Diversity Cells

MAP-Elites requires discrete niches. V1 cells should be enumerated, not inferred
by runtime clustering.

Recommended cell:

```text
(cost_tier, risk_profile, verifier_dependency)
```

Within each cell, keep the best candidate by verifier-first score. Across cells,
retain a capped frontier. This prevents the frontier from filling with many
near-duplicates of the same plan style.

If later work needs finer novelty, use cheap structural fingerprints before
semantic methods:

- sorted task IDs;
- sorted dependency edge list;
- obligation ID set;
- normalized task-title token set;
- Jaccard distance over these sets.

LLM embeddings should not be on the v1 path. They add latency,
nondeterminism, and an external dependency to a local-first planner.

### Boundary Champion Selection

The existing runner expects one executable `current_plan`, so `tzu` must choose
one champion at the execution boundary. This should be deterministic and should
not prompt the user by default.

Use a verifier-first scalar only at this boundary:

```text
champion = argmax frontier by (
  verifier_strength,
  -obligation_burden,
  -risk_profile,
  -cost_tier,
  task_graph_quality,
  execution_readiness,
  stable_candidate_id
)
```

This does not contradict the frontier architecture. The scalar is not used to
collapse exploration early. It is used only after admissibility, Pareto
filtering, diversity retention, and beam caps have done their work.

Manual selection can be added later as an explicit mode or command. It should
not be the default planning path.

## Resource Allocation

Successive halving and bandits belong later, after outcome telemetry exists.

### Successive Halving

Successive halving is the right next resource-allocation layer once validation
has cost tiers:

1. cheap static schema, DAG, evidence, and obligation checks;
2. repository inspection or local check planning;
3. ACP-backed plan refinement;
4. expensive verifier or execution run.

Many candidates should receive cheap checks. Only retained candidates should
receive expensive checks.

### Bandit Sampling

UCB or Thompson sampling should answer a narrow question:

```text
Which frontier candidate should receive the next unit of validation budget?
```

That question allows a normalized scalar reward without reverting to a scalar
plan-quality model. The reward is stage-specific verifier progress, not total
plan value.

Example reward:

```text
1.0 = passes next tier and discharges an obligation or adds concrete evidence
0.5 = passes next tier but adds no new evidence
0.0 = fails next tier or introduces a hard blocker
```

A "pull" is allocating the next validation tier to a candidate.

UCB1-style formulas require a measurable reward, visit count, and total pull
count, for example:

```text
Q_t(a) + c * sqrt(ln(t) / N_t(a))
```

Do not introduce UCB or Thompson sampling until candidate pulls and rewards are
persisted. Otherwise `tzu` would create another fake rating layer.

### TrueSkill And Pairwise Ranking

TrueSkill or Bradley-Terry models are appropriate only if `tzu` has real
pairwise or ranked matches. TrueSkill is more appropriate than basic Elo when
uncertainty matters because it tracks rating uncertainty and supports multiple
competing entities, but it still needs match evidence.

For v1, do not add a final pairwise LLM/rater step at the execution boundary.
It adds latency and another possible failure point just before execution.

## Persistence And Pruning

The current schema has candidate and match tables, but candidate history is not
yet written there. Frontier search should avoid growing `ProjectState` into a
large search transcript.

Retention policy:

- Keep full JSON for active candidates in the current run.
- Keep full JSON for the final frontier.
- Keep full JSON for the final champion.
- For candidates discarded in early tiers, keep only summary metadata:
  candidate hash, parent hashes, descriptor cell, score vector, validation tier,
  discard reason, hard failures, and obligation IDs.
- Keep full validator and agent outputs only for candidates that reach expensive
  tiers or are needed to explain the final decision.
- Keep long-lived candidate history in candidate, match, validator, and agent
  tables, not inside `ProjectState.current_plan`.

The first frontier implementation does not need structural sharing for task
DAGs. It should compute stable hashes instead:

```text
candidate_hash = hash(canonical PlanCandidate JSON)
task_graph_hash = hash(canonical task nodes and dependency edges)
```

Lineage should point to parent candidate hashes, not embed parent candidate
JSON.

If measured storage pressure later shows task DAGs dominate database size, add
content-addressed task storage:

```text
task_node(hash, task_json)
candidate_task(candidate_hash, task_hash)
candidate_edge(candidate_hash, from_task_hash, to_task_hash)
```

That schema should be driven by measurement, not added prematurely.

## CLI Observability

Keep `tzu status` concise. It should show:

- selected champion;
- total candidates evaluated;
- final frontier size;
- current plan tasks.

Example user-facing shape:

```text
Selected candidate: candidate-3
Candidates evaluated: 12
Frontier size: 4
```

Detailed inspection should move to a separate command such as
`tzu inspect --frontier` or `tzu tree`. That command can show descriptor cells,
score vectors, candidate lineage, discard reasons, and validator outcomes.

## Implementation Epochs

The work should land in reviewable epochs.

| Epoch                  | Scope                                                                                                                                              | Exit Criteria                                                                                                                                |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. Data and generation | Deterministic multi-candidate fixtures, generic adapter candidate palette, score vector, descriptors, candidate hashes, and candidate persistence. | Unit tests prove multiple distinct generic candidates, stable scoring, stable hashes, and persisted candidate summaries.                     |
| 2. Frontier            | Pareto filtering, beam caps, quality-diversity cells, final frontier metadata, concise status output, and detailed inspect output.                 | Tests prove non-dominated pruning, `min_elite` and `max_elite`, descriptor-cell coverage, deterministic champion tie-breaks, and CLI output. |
| 3. Resource allocation | Validation tiers, successive halving, persisted validator outcomes, then UCB or Thompson sampling.                                                 | Tests prove budget advancement, reward recording, no budget for invalid candidates, and deterministic replay of allocation decisions.        |

Epoch 1 is the highest-leverage first step. Without a real candidate population
and persisted metadata, every later selection algorithm is untestable.

## Blockers And Missing Artifacts

| Missing Artifact                      | Why It Matters                                                                            | Upstream Producer                                                        | Regeneration Workflow                                                                                                                                                                    | Validation Command                                                                                          |
| ------------------------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Historical candidate-selection traces | Needed to calibrate thresholds, beam widths, and score weights without inventing numbers. | Future planner and runner instrumentation.                               | Persist candidate populations, score components, frontier membership, match results, validator outcomes, execution outcomes, and lineage; then run a fixture matrix of `tzu plan` goals. | Query SQLite/Postgres for non-empty candidate, frontier, validator, and outcome rows for every fixture run. |
| Real multi-candidate generation       | Current adapters emit one candidate, so selection cannot be evaluated.                    | `DomainAdapter::seed_candidates` and later ACP-backed planning episodes. | Add deterministic generic archetypes and test fixtures.                                                                                                                                  | Unit tests assert at least three distinct valid candidates and descriptor tuples.                           |
| Stable diversity descriptors          | Quality-diversity retention needs discrete cells.                                         | Planner candidate generation and validation layer.                       | Add `cost_tier`, `risk_profile`, and `verifier_dependency`.                                                                                                                              | Tests show frontier selection keeps distinct descriptor cells when scores are close.                        |
| Validation budget tiers               | Successive halving and bandits need staged costs and outcomes.                            | Runner, validator integration, and ACP-backed execution layer.           | Define static, repository, ACP, and expensive verifier stages.                                                                                                                           | Tests show candidates advance only after passing prior tiers and failed candidates stop receiving budget.   |

These blockers do not block Epoch 1. They do block claims that a specific
percentile, UCB constant, or beam width is empirically correct.

## Evidence Inventory

Local evidence inspected:

- `git status --short`: the repository had many existing added/modified files;
  this dossier is documentation-only and does not revert unrelated work.
- `rg --files`: showed no pre-existing root `docs/` tree, so this dossier lives
  in `docs/planning/`.
- `rg -n "plan|planning|candidate|rating|elo|frontier|beam|pareto|validation|obligation|harness|selected|match|score|persist|schema" -S README.md Cargo.toml crates simit.toml .github docs`:
  located the harness, candidate, selection, persistence, and CLI status
  surfaces. It also confirmed `.github` and `docs` were absent in the root repo
  at the time of research.
- `nl -ba README.md`: established declared harness responsibilities.
- `nl -ba crates/tzu-core/src/lib.rs`: established current candidate models,
  validation, rating, and greedy selection.
- `nl -ba crates/tzu-runner/src/lib.rs`: established persisted planning summary
  metadata and unused candidate/match tables.
- `nl -ba crates/tzu-cli/src/main.rs`: established current CLI status surface.
- `nl -ba alphaproof-nexus-results/README.md`: established the APN checkout is
  a successful-output archive, not complete telemetry.

External primary references:

- AlphaProof Nexus paper:
  https://arxiv.org/html/2605.22763v1
- TrueSkill, Microsoft Research:
  https://www.microsoft.com/en-us/research/publication/trueskilltm-a-bayesian-skill-rating-system-2/
- Hyperband, JMLR 2018:
  https://www.jmlr.org/beta/papers/v18/16-558.html
- UCB1 finite-time bandit analysis:
  https://www.cs.utexas.edu/~shivaram/readings/b2hd-AuerCF2002.html
- Thompson Sampling, Agrawal and Goyal:
  https://proceedings.mlr.press/v23/agrawal12.html
- NSGA-II:
  https://doi.org/10.1109/4235.996017
- MAP-Elites:
  https://arxiv.org/abs/1504.04909
- Beam search resource/quality tradeoff reference:
  https://ojs.aaai.org/index.php/ICAPS/article/view/19805

## Resolved Decisions

| Decision                 | Resolution                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Status surface           | Keep `tzu status` concise. Add a separate frontier/tree inspection command for detailed search observability.                               |
| First candidate source   | Start with deterministic fixtures and deterministic `GenericDomainAdapter` archetypes. Defer ACP-backed generation.                         |
| V1 descriptors           | Use `cost_tier`, `risk_profile`, and `verifier_dependency`. Avoid semantic strategy-family classification in v1.                            |
| Final champion selection | Use a verifier-first deterministic scalar at the execution boundary. Do not add a final LLM pairwise comparison step.                       |
| Pareto dimensionality    | Keep Pareto axes small and ordinal. Use coarse bucket dominance in v1 and epsilon dominance only if continuous scores are later introduced. |
| Storage                  | Use retention and hashes first. Add structural task DAG sharing only after measured storage pressure justifies it.                          |

## Conclusion

The next real step is Epoch 1: create a deterministic multi-candidate population
and persist enough metadata to make selection decisions observable and
testable. Until that exists, neither percentiles, Pareto frontiers, bandits, nor
TrueSkill can be evaluated honestly.

Once Epoch 1 exists, `tzu` can add a capped, quality-diverse frontier that keeps
multiple promising basins alive without weakening validation or overloading the
CLI. Only after staged validation outcomes are flowing should `tzu` add
successive halving or bandit allocation.
