## ADDED Requirements
### Requirement: Agents can query the canonical business-rule catalog safely
The local MCP server SHALL expose strict bounded tools and versioned resources for rule listing/search, exact rule explanation, domain summaries, source-span reverse lookup, dependencies/conflicts, release/history comparison, and explicit evidence hydration through the same canonical archaeology read service as the desktop. Requests MUST use opaque repository, rule, source, and evidence identities and MUST fail closed on unknown fields, invalid cursors, stale scope, cross-repository identity, or unavailable coverage.

#### Scenario: Agent searches a large rule catalog
- **WHEN** an enabled client searches rules by domain, concept, data field, source identity, release, trust, or lifecycle state
- **THEN** the server returns deterministic paginated compact rows with stable opaque IDs, freshness, coverage, applied limits, continuation, and no model call or repository rescan

#### Scenario: Agent requests exact rule evidence
- **WHEN** an enabled client requests one valid rule and a bounded set of its evidence IDs
- **THEN** the server returns atomic clauses, provenance, dependencies, contradictions, redacted source spans, and stable links within the response-byte and evidence-count limits

#### Scenario: Agent crosses repository scope
- **WHEN** a rule, source, evidence, cursor, or history identity belongs to another repository or cannot be resolved exactly
- **THEN** the server rejects the request with a redacted error without disclosing whether the foreign identity exists

### Requirement: Rule MCP reads preserve privacy, lifecycle, and qualification limits
Every rule MCP response SHALL exclude raw prompts, raw email, credentials, absolute paths, protected source, unrestricted excerpts, and unsupported intent/quality claims; SHALL report rule origin trust, lifecycle, parser/synthesis identities, index freshness, temporal and language coverage, and response bounds; and SHALL remain subject to live revocation and metadata-only audit.

#### Scenario: Accepted rule has stale source evidence
- **WHEN** the enabled repository HEAD or relevant parser/config identity differs from the rule generation
- **THEN** the response labels the rule and review state stale and does not describe prior acceptance as validation of current code

#### Scenario: Rule response would exceed the byte ceiling
- **WHEN** dependencies, conflicts, aliases, history, or evidence exceed one MCP response
- **THEN** the server returns a deterministic bounded subset and opaque continuations rather than truncating JSON or leaking unrestricted content
