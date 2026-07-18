-- Physical index-density migration. These indexes duplicate a retained key
-- prefix or have no production query consumer. Removing them keeps canonical
-- constraints and hot read paths while avoiding per-generation write/storage
-- amplification.
DROP INDEX IF EXISTS idx_archaeology_rule_clauses_rule;
DROP INDEX IF EXISTS idx_archaeology_facts_kind;
DROP INDEX IF EXISTS idx_archaeology_rules_repository_revision;
DROP INDEX IF EXISTS idx_archaeology_rules_generation_stable;
DROP INDEX IF EXISTS idx_archaeology_source_units_content;
DROP INDEX IF EXISTS idx_archaeology_source_units_language;
DROP INDEX IF EXISTS idx_archaeology_rules_parser_compatibility;
