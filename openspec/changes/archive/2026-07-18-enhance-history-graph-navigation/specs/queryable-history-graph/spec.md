## ADDED Requirements

### Requirement: Releases are first-class history landmarks
The system SHALL expose a complete bounded catalog of indexed release tags independently of the currently loaded revision window, SHALL resolve each selectable tag to its exact revision, and SHALL render releases as aligned slider landmarks plus a searchable release selector.

#### Scenario: User selects a release outside the recent window
- **WHEN** the user selects an indexed release whose revision is not in the current timeline window
- **THEN** the system loads a bounded window around the exact tagged revision, moves the shared cursor to that SHA, and reconstructs that release graph without checkout

#### Scenario: Multiple release tags point to one revision
- **WHEN** two or more release tags resolve to the same revision
- **THEN** the rail renders one grouped position while the selector preserves every tag and each selection resolves to the same exact graph state

#### Scenario: Release coverage is incomplete
- **WHEN** shallow history, missing ancestry, truncation, or an unindexed tag prevents a complete release interval
- **THEN** the selector and interval summary label the limitation and do not present a bounded topo-order count as a complete release range

### Requirement: History exposes explainable candidate inflection points
The system SHALL derive deterministic repository-relative candidate inflection landmarks from comparable indexed change facts, SHALL retain the versioned score inputs and human-readable reasons, and MUST NOT present a statistical landmark as verified intent, causation, quality, or impact.

#### Scenario: A revision is unusually large for the repository
- **WHEN** a revision exceeds the versioned robust baseline and minimum magnitude using churn, file-count, and available structural-topology facts
- **THEN** the timeline marks its exact revision and explains the observed components, baseline, algorithm identity, coverage, and any generated/vendor/release-noise caveat

#### Scenario: Inflection baseline is insufficient
- **WHEN** the indexed history lacks enough comparable revisions or required facts are bounded or missing
- **THEN** the system reports candidate-inflection coverage as unavailable or partial and does not invent a landmark

#### Scenario: Index identity is unchanged
- **WHEN** the same HEAD, tags, ignore rules, mailmap, facts, and algorithm version are indexed again
- **THEN** candidate-inflection IDs, scores, reasons, ordering, and selected revisions remain deterministic

### Requirement: All history navigation shares one exact temporal cursor
The system SHALL keep slider scrubbing, playback, search results, release selection, landmark navigation, contributor scope, and graph reconstruction synchronized through one revision-SHA cursor with latest-request-wins behavior.

#### Scenario: Slow historical state returns after a newer selection
- **WHEN** an earlier revision request finishes after the user has selected a newer release, landmark, search result, or slider position
- **THEN** the stale result is ignored and the graph, labels, contributor scope, and accessibility state remain on the newer revision

#### Scenario: User plays graph history
- **WHEN** the user starts playback or steps to the next or previous landmark
- **THEN** graph additions, removals, and changes animate in revision order, the active release and landmark update, and the user can pause or scrub immediately

#### Scenario: User navigates landmarks with a keyboard
- **WHEN** focus is on the history controls and the user invokes previous/next landmark or chooses a release
- **THEN** the exact cursor moves with an announced release or candidate-inflection label, revision, position, and coverage state

### Requirement: Contributor analytics follow the selected history interval
The system SHALL provide bounded contributor summaries for an exact ancestry-aware interval or release cycle using canonical repository-local identities, SHALL separate primary authorship, co-author participation, automation, and unknown identities, and MUST NOT equate contribution volume with ownership, causation, or quality.

#### Scenario: User selects a release
- **WHEN** the shared cursor is inside or exactly at a release cycle
- **THEN** contributor analytics state the resolved `from_exclusive` and `to_inclusive` revisions and show deterministic commit, active-day, churn, participation, automation, top-area, and concentration summaries for covered history

#### Scenario: Mailmap aliases and co-authors are present
- **WHEN** multiple local identities resolve through `.mailmap` or a commit contains canonicalizable `Co-authored-by` trailers
- **THEN** aliases resolve to one repository-scoped contributor, primary commit and churn totals remain single-counted, and co-author participation is reported separately

#### Scenario: Contributor results exceed the page bound
- **WHEN** more contributors participated than the configured result limit
- **THEN** the response returns a stable page and an explicit `other` aggregate so interval totals, automation share, and concentration remain reconcilable

#### Scenario: Identity or topology coverage is ambiguous
- **WHEN** mailmap resolution, merge policy, shallow history, binary churn, generated paths, or ancestry prevents exact attribution
- **THEN** the summary exposes the limitation and does not silently merge identities or claim complete release participation

### Requirement: Historical landmarks and contributor queries remain local and fast
The system SHALL build landmark and contributor facts incrementally during history indexing, SHALL serve warm navigation from indexed SQLite state and revision caches, and MUST NOT invoke a model, network service, repository checkout, or unbounded all-history scan while scrubbing or querying a selected interval.

#### Scenario: User scrubs across cached adjacent revisions
- **WHEN** adjacent structural states and temporal summaries are cached
- **THEN** the frontend schedules the selection within one animation frame without a Git process or duplicate backend request

#### Scenario: Repository history changes incrementally
- **WHEN** HEAD or tags advance while prior index identities remain valid
- **THEN** the system indexes only changed revision, landmark, and contributor facts unless an ignore, mailmap, schema, or algorithm fingerprint requires a bounded rebuild

#### Scenario: Qualification benchmark runs
- **WHEN** the realistic local history fixture is qualified on the named machine
- **THEN** the report publishes cached and uncached scrub latency, release/contributor query latency, incremental indexing cost, database growth, memory, and cleanup and enforces the established scrub regression gate
