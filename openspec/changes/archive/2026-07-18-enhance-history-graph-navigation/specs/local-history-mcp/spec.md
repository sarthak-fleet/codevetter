## ADDED Requirements

### Requirement: Agents can query bounded history landmarks
The MCP server SHALL expose compact paginated release and candidate-inflection landmark records over the canonical history read model with stable opaque IDs, exact revisions, kinds, labels, observed reasons, trust, coverage, freshness, and continuation metadata.

#### Scenario: Agent lists release and inflection landmarks
- **WHEN** an enabled repository scope requests valid landmark kinds and a bounded temporal range
- **THEN** the server returns deterministic non-duplicated landmarks and opaque continuation links without reconstructing historical graphs or invoking Git per result

#### Scenario: Agent requests an inferred landmark as fact
- **WHEN** a candidate-inflection record is returned or read as a resource
- **THEN** the envelope identifies its algorithm, score components, qualified trust, evidence gaps, and non-causal status rather than presenting inferred importance as verified intent

### Requirement: Agents can query privacy-safe temporal contributor summaries
The MCP server SHALL expose bounded contributor summaries for an exact release, revision, or explicit ancestry-aware interval using opaque repository-scoped contributor IDs and MUST exclude raw email, absolute paths, cross-repository identity, and unsupported quality or ownership claims.

#### Scenario: Agent asks who participated in a release
- **WHEN** an enabled client requests contributors for a valid release tag
- **THEN** the server resolves exact interval revisions and returns deterministic primary-author, co-author, automation, activity, churn, concentration, coverage, and bounded evidence-link summaries

#### Scenario: Agent requests contributor detail
- **WHEN** the request includes a valid contributor ID within the same repository scope
- **THEN** the server returns only bounded temporal participation and opaque evidence links for that contributor without disclosing canonical email or another repository's identity mapping

#### Scenario: Contributor page is truncated
- **WHEN** the contributor result exceeds row or response-byte limits
- **THEN** the server returns a stable page, deterministic cursor, applied limits, and an `other` aggregate that preserves reconcilable interval totals

### Requirement: Landmark and contributor MCP reads preserve index identity
Every landmark and contributor response SHALL include indexed/current revision identity, tag and mailmap freshness, algorithm/schema identity, ancestry and coverage limitations, and applied bounds, and MUST fail closed when the requested temporal reference cannot be resolved safely.

#### Scenario: Tags or mailmap changed after indexing
- **WHEN** the current tag or `.mailmap` fingerprint differs from the facts used by a landmark or contributor query
- **THEN** the response is marked stale and does not imply coverage of the changed release or identity mapping

#### Scenario: Temporal reference is ambiguous or out of scope
- **WHEN** a release, revision, interval, contributor, cursor, or resource belongs outside the enabled repository scope or cannot be resolved exactly
- **THEN** the server returns a stable redacted error without leaking repository existence, paths, identities, SQL, or query content
