## ADDED Requirements
### Requirement: Trusted graph context includes evidence-traced business rules
The canonical repository graph SHALL represent archaeology rules, atomic clauses, domains, data, transactions, programs, calls, and source units as versioned nodes and typed relationships whose origin is `extracted`, `deterministic`, `model_synthesized`, or `human_confirmed`, with exact evidence IDs and coverage. Rule graph edges MUST NOT erase contradiction, ambiguity, lifecycle, or parser limitations and MUST NOT independently create a finding or verified behavioral claim.

#### Scenario: Rule connects code to a payment field
- **WHEN** normalized facts support a rule condition and mutation involving a payment data field
- **THEN** the graph links rule, clause, predicate, mutation, data field, and source spans with direction, trust, evidence, and coverage for every hop

#### Scenario: Model-synthesized dependency is uncertain
- **WHEN** rule prose suggests a dependency that normalized call/data/control facts do not prove
- **THEN** the graph marks or omits the relationship as unsupported/ambiguous and does not upgrade it through graph centrality or neighboring trusted edges

### Requirement: Rule graph navigation remains bounded at catalog scale
CodeVetter SHALL provide bounded rule-to-code, code-to-rule, rule-to-rule dependency/conflict, domain, and impact navigation without materializing the full rule catalog or source graph in the UI, prompt, or MCP response. Traversal SHALL use deterministic limits, opaque continuation identities, and honest no-path/partial-coverage outcomes.

#### Scenario: One source field participates in thousands of rules
- **WHEN** a source or data node has more rule relationships than the configured traversal bound
- **THEN** CodeVetter returns a stable ranked subset plus total/truncation/continuation metadata instead of rendering or serializing every relationship

#### Scenario: No qualified path is available
- **WHEN** the indexed graph has no bounded evidence-supported path between two rule concepts
- **THEN** CodeVetter reports no qualified path within current coverage and does not claim the concepts are unrelated
