//! Dependency-free extraction of framework, API-contract, and data-lineage facts.
//!
//! These scanners intentionally recognize only explicit, source-backed forms. A
//! later resolution pass may connect reference facts to declarations; collisions
//! remain ambiguous instead of being guessed here.

use super::types::GraphTrust;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFact {
    pub key: String,
    pub line: usize,
    pub kind: String,
    pub label: String,
    pub edge_kind: String,
    pub detail: String,
    pub trust: GraphTrust,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractLink {
    pub from_key: String,
    pub to_key: String,
    pub edge_kind: String,
    pub detail: String,
    pub trust: GraphTrust,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractExtraction {
    pub facts: Vec<ContractFact>,
    pub links: Vec<ContractLink>,
}

impl ContractExtraction {
    fn fact(
        &mut self,
        line: usize,
        kind: &str,
        label: impl Into<String>,
        edge_kind: &str,
        detail: &str,
    ) -> String {
        let label = clean_label(&label.into());
        if label.is_empty() || label.len() > 240 {
            return String::new();
        }
        let key = format!("{kind}:{line}:{label}:{}", self.facts.len());
        self.facts.push(ContractFact {
            key: key.clone(),
            line,
            kind: kind.to_string(),
            label,
            edge_kind: edge_kind.to_string(),
            detail: detail.to_string(),
            trust: GraphTrust::Extracted,
        });
        key
    }

    fn reference(
        &mut self,
        line: usize,
        kind: &str,
        label: impl Into<String>,
        edge_kind: &str,
        detail: &str,
    ) -> String {
        self.fact(line, kind, label, edge_kind, detail)
    }

    fn ambiguous_reference(&mut self, line: usize, label: impl Into<String>, detail: &str) {
        let key = self.fact(line, "dynamic_reference", label, "may_reference", detail);
        if let Some(fact) = self.facts.iter_mut().find(|fact| fact.key == key) {
            fact.trust = GraphTrust::Ambiguous;
        }
    }

    fn link(&mut self, from_key: &str, to_key: &str, edge_kind: &str, detail: &str) {
        if from_key.is_empty() || to_key.is_empty() {
            return;
        }
        self.links.push(ContractLink {
            from_key: from_key.to_string(),
            to_key: to_key.to_string(),
            edge_kind: edge_kind.to_string(),
            detail: detail.to_string(),
            trust: GraphTrust::Extracted,
        });
    }
}

pub fn extract_contracts(path: &str, source: &str) -> ContractExtraction {
    let lower_path = path.to_ascii_lowercase();
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = ContractExtraction::default();
    let mut openapi_route: Option<(usize, String, String)> = None;
    let mut openapi_operation: Option<String> = None;
    let mut openapi_schema_indent: Option<usize> = None;
    let mut proto_service: Option<String> = None;

    for (index, raw_line) in lines.iter().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        let lower = line.to_ascii_lowercase();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }

        extract_sql(line_number, line, &lower, &mut output);
        extract_route_and_handler(line_number, line, &lower, &lines, index, &mut output);
        extract_jobs_events_and_bindings(line_number, line, &lower, &lines, index, &mut output);
        extract_config_reference(line_number, line, &lower, &mut output);
        extract_dynamic_reference(line_number, line, &lower, &mut output);

        if is_openapi_path(&lower_path, source) {
            extract_openapi(
                line_number,
                raw_line,
                &lower,
                &mut openapi_route,
                &mut openapi_operation,
                &mut openapi_schema_indent,
                &mut output,
            );
        }
        if lower_path.ends_with(".graphql") || lower_path.ends_with(".gql") {
            extract_graphql(line_number, line, &lower, &mut output);
        }
        if lower_path.ends_with(".proto") {
            extract_protobuf(line_number, line, &lower, &mut proto_service, &mut output);
        }
        if is_dbt_path(&lower_path) {
            extract_dbt(line_number, line, &lower, &mut output);
        }
    }
    output
}

fn extract_sql(line_number: usize, line: &str, lower: &str, output: &mut ContractExtraction) {
    for (marker, kind, detail) in [
        ("table", "db_table", "SQL table declaration"),
        ("view", "db_view", "SQL view declaration"),
        ("index", "db_index", "SQL index declaration"),
    ] {
        if lower.contains(&format!("create {marker}"))
            || lower.contains(&format!("create or replace {marker}"))
        {
            if let Some(label) = sql_object_after(line, marker) {
                output.fact(line_number, kind, label, "declares", detail);
            }
        }
    }
    for (marker, edge_kind) in [
        ("from", "reads_from"),
        ("join", "reads_from"),
        ("update", "writes_to"),
        ("into", "writes_to"),
        ("delete from", "writes_to"),
    ] {
        for label in sql_references_after(line, marker) {
            output.reference(
                line_number,
                "db_object_reference",
                label,
                edge_kind,
                "explicit SQL object reference",
            );
        }
    }
}

fn extract_route_and_handler(
    line_number: usize,
    line: &str,
    lower: &str,
    lines: &[&str],
    index: usize,
    output: &mut ContractExtraction,
) {
    let route_marker = [
        "router.get(",
        "router.post(",
        "router.put(",
        "router.patch(",
        "router.delete(",
        "app.get(",
        "app.post(",
        "app.put(",
        "app.patch(",
        "app.delete(",
        "@get(",
        "@post(",
        "@put(",
        "@patch(",
        "@delete(",
        "#[get(",
        "#[post(",
        "#[put(",
        "#[patch(",
        "#[delete(",
        "@getmapping(",
        "@postmapping(",
        "@putmapping(",
        "@deletemapping(",
        "@requestmapping(",
        "handlefunc(",
        "route::get(",
        "route::post(",
        "get \"/",
        "get '/",
        "post \"/",
        "post '/",
        "<route",
        ".route(",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    if !route_marker {
        return;
    }
    let Some(route) = quoted_values(line)
        .into_iter()
        .find(|value| value.starts_with('/'))
    else {
        return;
    };
    let route_key = output.fact(
        line_number,
        "route",
        route,
        "exposes",
        "explicit framework route",
    );
    let handler = handler_identifier(line).or_else(|| {
        lines
            .iter()
            .skip(index + 1)
            .take(4)
            .find_map(|next| function_identifier(next))
    });
    if let Some(handler) = handler {
        let reference_key = output.reference(
            line_number,
            "handler_reference",
            handler,
            "references",
            "explicit route handler reference",
        );
        output.link(
            &route_key,
            &reference_key,
            "routes_to",
            "route explicitly names this handler",
        );
    }
}

fn extract_jobs_events_and_bindings(
    line_number: usize,
    line: &str,
    lower: &str,
    lines: &[&str],
    index: usize,
    output: &mut ContractExtraction,
) {
    if ["@job", "#[job", "@task", "#[task", "@celery.task"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        let label = first_quoted(line).or_else(|| {
            lines
                .iter()
                .skip(index + 1)
                .take(4)
                .find_map(|next| function_identifier(next))
        });
        if let Some(label) = label {
            output.fact(
                line_number,
                "job",
                label,
                "declares",
                "explicit framework background job",
            );
        }
    }
    if ["queue.add(", "enqueue(", "schedule(", ".cron("]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        if let Some(label) = first_quoted(line) {
            output.reference(
                line_number,
                "job_reference",
                label,
                "schedules",
                "explicit job scheduling call",
            );
        }
    }
    if [".emit(", "publish(", "dispatch("]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        if let Some(label) = first_quoted(line) {
            output.reference(
                line_number,
                "event_reference",
                label,
                "emits",
                "explicit event emission",
            );
        }
    }
    if [".on(", "subscribe(", "addlistener("]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        let values = quoted_values(line);
        if let Some(event) = values.first() {
            let event_key = output.fact(
                line_number,
                "event_subscription",
                event,
                "subscribes",
                "explicit event subscription",
            );
            if let Some(handler) = handler_identifier(line) {
                let handler_key = output.reference(
                    line_number,
                    "handler_reference",
                    handler,
                    "references",
                    "explicit event handler reference",
                );
                output.link(
                    &event_key,
                    &handler_key,
                    "handles",
                    "subscription explicitly names this handler",
                );
            }
        }
    }
    if lower.contains(".bind(") && lower.contains(".to(") {
        let identifiers = parenthesized_identifiers(line);
        if identifiers.len() >= 2 {
            let binding = output.fact(
                line_number,
                "dependency_binding",
                format!("{} -> {}", identifiers[0], identifiers[1]),
                "binds",
                "explicit dependency-injection binding",
            );
            let contract = output.reference(
                line_number,
                "type_reference",
                &identifiers[0],
                "references",
                "dependency contract",
            );
            let implementation = output.reference(
                line_number,
                "type_reference",
                &identifiers[1],
                "references",
                "dependency implementation",
            );
            output.link(&binding, &contract, "binds_contract", "binding contract");
            output.link(
                &binding,
                &implementation,
                "binds_to",
                "binding implementation",
            );
        }
    }
    if lower.contains("addscoped<")
        || lower.contains("addsingleton<")
        || lower.contains("addtransient<")
    {
        let identifiers = generic_identifiers(line);
        if identifiers.len() >= 2 {
            let binding = output.fact(
                line_number,
                "dependency_binding",
                format!("{} -> {}", identifiers[0], identifiers[1]),
                "binds",
                "explicit dependency-injection registration",
            );
            let implementation = output.reference(
                line_number,
                "type_reference",
                &identifiers[1],
                "references",
                "dependency implementation",
            );
            output.link(
                &binding,
                &implementation,
                "binds_to",
                "binding implementation",
            );
        }
    }
}

fn extract_config_reference(
    line_number: usize,
    line: &str,
    lower: &str,
    output: &mut ContractExtraction,
) {
    let markers = ["process.env.", "import.meta.env."];
    for marker in markers {
        if let Some(position) = lower.find(marker) {
            let start = position + marker.len();
            let label = line[start..]
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>();
            output.reference(
                line_number,
                "configuration_reference",
                label,
                "reads_config",
                "explicit environment configuration reference",
            );
        }
    }
    for marker in ["std::env::var(", "env::var(", "os.getenv("] {
        if lower.contains(marker) {
            if let Some(label) = first_quoted(line) {
                output.reference(
                    line_number,
                    "configuration_reference",
                    label,
                    "reads_config",
                    "explicit environment configuration reference",
                );
            }
        }
    }
}

fn extract_dynamic_reference(
    line_number: usize,
    line: &str,
    lower: &str,
    output: &mut ContractExtraction,
) {
    let marker = [
        "getattr(",
        "setattr(",
        "import_module(",
        "class.forname(",
        "type.gettype(",
        "activator.createinstance(",
        "method.invoke(",
        "container.resolve(",
        "dlsym(",
        "libloading",
        "send(",
        "const_get(",
    ]
    .iter()
    .find(|marker| lower.contains(**marker));
    let Some(marker) = marker else {
        return;
    };
    let label = quoted_values(line)
        .into_iter()
        .next_back()
        .unwrap_or_else(|| format!("{marker} at line {line_number}"));
    output.ambiguous_reference(
        line_number,
        label,
        "reflection or runtime lookup may reference a symbol dynamically",
    );
}

fn extract_openapi(
    line_number: usize,
    line: &str,
    lower: &str,
    current_route: &mut Option<(usize, String, String)>,
    current_operation: &mut Option<String>,
    schema_indent: &mut Option<usize>,
    output: &mut ContractExtraction,
) {
    let indent = leading_indent(line);
    let key = mapping_key(line);
    if key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case("schemas"))
    {
        *schema_indent = Some(indent);
        return;
    }
    if let Some(root_indent) = *schema_indent {
        if indent <= root_indent {
            *schema_indent = None;
        } else if indent == root_indent + 2 && key.is_some() {
            output.fact(
                line_number,
                "openapi_schema",
                key.as_deref().unwrap_or_default(),
                "declares",
                "OpenAPI component schema declaration",
            );
        }
    }
    if key.as_deref().is_some_and(|key| key.starts_with('/')) {
        let label = key.as_deref().unwrap_or_default();
        let key = output.fact(
            line_number,
            "openapi_path",
            label,
            "declares",
            "OpenAPI path declaration",
        );
        *current_route = Some((leading_indent(line), label.to_string(), key));
        *current_operation = None;
        return;
    }
    if current_route
        .as_ref()
        .is_some_and(|(route_indent, _, _)| indent <= *route_indent)
    {
        *current_route = None;
        *current_operation = None;
    }
    let operation = ["get", "post", "put", "patch", "delete", "options", "head"]
        .iter()
        .find(|method| {
            key.as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case(method))
        });
    if let (Some(method), Some((_, path, path_key))) = (operation, current_route.as_ref()) {
        let operation_key = output.fact(
            line_number,
            "openapi_operation",
            format!("{} {path}", method.to_ascii_uppercase()),
            "declares",
            "OpenAPI operation declaration",
        );
        output.link(
            path_key,
            &operation_key,
            "exposes",
            "OpenAPI path exposes this operation",
        );
        *current_operation = Some(operation_key);
    }
    if key.as_deref() == Some("$ref") || lower.contains("$ref") {
        if let Some(reference) = mapping_value(line) {
            let reference_key = output.reference(
                line_number,
                "schema_reference",
                reference,
                "references_schema",
                "OpenAPI schema reference",
            );
            if let Some(operation) = current_operation.as_ref() {
                output.link(
                    operation,
                    &reference_key,
                    "uses_schema",
                    "OpenAPI operation references this schema",
                );
            }
        }
    }
    if key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case("operationId"))
    {
        if let Some(label) = line.split(':').nth(1) {
            let handler_key = output.reference(
                line_number,
                "handler_reference",
                clean_label(label),
                "implemented_by",
                "OpenAPI operationId handler reference",
            );
            if let Some(operation) = current_operation.as_ref() {
                output.link(
                    operation,
                    &handler_key,
                    "implemented_by",
                    "OpenAPI operationId names this handler",
                );
            }
        }
    }
}

fn extract_graphql(line_number: usize, line: &str, lower: &str, output: &mut ContractExtraction) {
    for (keyword, kind) in [
        ("type ", "graphql_type"),
        ("input ", "graphql_input"),
        ("interface ", "graphql_interface"),
        ("enum ", "graphql_enum"),
        ("scalar ", "graphql_scalar"),
        ("directive ", "graphql_directive"),
    ] {
        if lower.starts_with(keyword) {
            if let Some(label) = identifier_after(line, keyword.len()) {
                output.fact(
                    line_number,
                    kind,
                    label,
                    "declares",
                    "GraphQL schema declaration",
                );
            }
        }
    }
    if lower.starts_with("query ")
        || lower.starts_with("mutation ")
        || lower.starts_with("subscription ")
    {
        if let Some(label) = line.split_whitespace().nth(1) {
            output.fact(
                line_number,
                "graphql_operation",
                clean_label(label),
                "declares",
                "GraphQL operation declaration",
            );
        }
    }
}

fn extract_protobuf(
    line_number: usize,
    line: &str,
    lower: &str,
    current_service: &mut Option<String>,
    output: &mut ContractExtraction,
) {
    for (keyword, kind) in [
        ("message ", "protobuf_message"),
        ("enum ", "protobuf_enum"),
        ("service ", "protobuf_service"),
    ] {
        if lower.starts_with(keyword) {
            if let Some(label) = identifier_after(line, keyword.len()) {
                let key = output.fact(
                    line_number,
                    kind,
                    &label,
                    "declares",
                    "protobuf contract declaration",
                );
                if kind == "protobuf_service" {
                    *current_service = Some(key);
                }
            }
        }
    }
    if lower.starts_with("rpc ") {
        if let Some(label) = identifier_after(line, 4) {
            let rpc_key = output.fact(
                line_number,
                "protobuf_rpc",
                label,
                "declares",
                "protobuf RPC declaration",
            );
            if let Some(service) = current_service.as_ref() {
                output.link(service, &rpc_key, "exposes", "service exposes this RPC");
            }
            let identifiers = parenthesized_identifiers(line);
            for (position, contract) in identifiers.into_iter().take(2).enumerate() {
                let reference = output.reference(
                    line_number,
                    "protobuf_message_reference",
                    contract,
                    "references",
                    "protobuf RPC message contract",
                );
                output.link(
                    &rpc_key,
                    &reference,
                    if position == 0 { "accepts" } else { "returns" },
                    "RPC request/response contract",
                );
            }
        }
    }
}

fn extract_dbt(line_number: usize, line: &str, lower: &str, output: &mut ContractExtraction) {
    for marker in ["ref(", "source("] {
        if lower.contains(marker) {
            let values = quoted_values(line);
            if let Some(label) = values.last() {
                output.reference(
                    line_number,
                    "dbt_model_reference",
                    label,
                    "depends_on",
                    "explicit dbt model/source reference",
                );
            }
        }
    }
    if lower.starts_with("- name:") || lower.starts_with("name:") {
        if let Some(label) = line.split(':').nth(1) {
            output.fact(
                line_number,
                "dbt_model",
                clean_label(label),
                "declares",
                "dbt model/schema declaration",
            );
        }
    }
}

fn is_openapi_path(path: &str, source: &str) -> bool {
    path.contains("openapi")
        || path.contains("swagger")
        || source.lines().take(20).any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("openapi:") || lower.contains("\"openapi\"")
        })
}

fn is_dbt_path(path: &str) -> bool {
    path.starts_with("models/")
        || path.contains("/models/")
        || path.ends_with("schema.yml")
        || path.ends_with("schema.yaml")
}

fn mapping_key(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_end_matches(',');
    if let Some(quote) = trimmed
        .chars()
        .next()
        .filter(|value| matches!(value, '"' | '\''))
    {
        let remainder = &trimmed[quote.len_utf8()..];
        let end = remainder.find(quote)?;
        if remainder[end + quote.len_utf8()..]
            .trim_start()
            .starts_with(':')
        {
            return Some(remainder[..end].trim().to_string());
        }
    }
    let (key, _) = trimmed.split_once(':')?;
    let key = clean_label(key);
    (!key.is_empty()).then_some(key)
}

fn mapping_value(line: &str) -> Option<String> {
    let (_, value) = line.split_once(':')?;
    let value = clean_label(value.trim_end_matches(','));
    (!value.is_empty()).then_some(value)
}

fn sql_object_after(line: &str, object_kind: &str) -> Option<String> {
    let tokens = sql_tokens(line);
    let position = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(object_kind))?;
    tokens
        .iter()
        .skip(position + 1)
        .find(|token| {
            !matches!(
                token.to_ascii_lowercase().as_str(),
                "if" | "not" | "exists" | "unique" | "concurrently"
            )
        })
        .map(|value| clean_sql_identifier(value))
}

fn sql_references_after(line: &str, marker: &str) -> Vec<String> {
    let tokens = sql_tokens(line);
    let marker_tokens = marker.split_whitespace().collect::<Vec<_>>();
    let mut results = Vec::new();
    for window_start in 0..tokens.len() {
        if window_start + marker_tokens.len() >= tokens.len() {
            break;
        }
        let matches = marker_tokens
            .iter()
            .enumerate()
            .all(|(offset, expected)| tokens[window_start + offset].eq_ignore_ascii_case(expected));
        if matches {
            let candidate = clean_sql_identifier(tokens[window_start + marker_tokens.len()]);
            if !candidate.is_empty() && !candidate.starts_with('(') {
                results.push(candidate);
            }
        }
    }
    results
}

fn sql_tokens(line: &str) -> Vec<&str> {
    line.split(|character: char| {
        character.is_whitespace() || matches!(character, '(' | ')' | ',' | ';' | '=')
    })
    .filter(|token| !token.is_empty())
    .collect()
}

fn clean_sql_identifier(value: &str) -> String {
    value
        .trim_matches(['`', '"', '\'', '[', ']'])
        .trim_end_matches(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '.'
        })
        .to_string()
}

fn quoted_values(line: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (index, character) in line.char_indices() {
        if let Some(active) = quote {
            if character == active {
                let value = line[start..index].trim();
                if !value.is_empty() {
                    output.push(value.to_string());
                }
                quote = None;
            }
        } else if matches!(character, '"' | '\'' | '`') {
            quote = Some(character);
            start = index + character.len_utf8();
        }
    }
    output
}

fn first_quoted(line: &str) -> Option<String> {
    quoted_values(line).into_iter().next()
}

fn handler_identifier(line: &str) -> Option<String> {
    let after_comma = line.rsplit_once(',')?.1;
    let candidate = after_comma
        .trim()
        .trim_matches([')', ']', '}', ';', ' ', '<', '>', '/'])
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':'))
        })
        .find(|token| !token.is_empty())?;
    let terminal = candidate.split(['.', ':']).rfind(|part| !part.is_empty())?;
    is_identifier(terminal).then(|| terminal.to_string())
}

fn function_identifier(line: &str) -> Option<String> {
    for marker in ["fn ", "function ", "def ", "func ", "fun "] {
        if let Some(position) = line.find(marker) {
            return identifier_after(line, position + marker.len());
        }
    }
    None
}

fn identifier_after(line: &str, start: usize) -> Option<String> {
    let value = line.get(start..)?.trim_start();
    let identifier = value
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    is_identifier(&identifier).then_some(identifier)
}

fn parenthesized_identifiers(line: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut remainder = line;
    while let Some(start) = remainder.find('(') {
        let after = &remainder[start + 1..];
        let Some(end) = after.find(')') else {
            break;
        };
        for candidate in after[..end].split(',') {
            let value = clean_label(candidate);
            if is_identifier(&value) {
                output.push(value);
            }
        }
        remainder = &after[end + 1..];
    }
    output
}

fn generic_identifiers(line: &str) -> Vec<String> {
    let Some(start) = line.find('<') else {
        return Vec::new();
    };
    let Some(end) = line[start + 1..].find('>') else {
        return Vec::new();
    };
    line[start + 1..start + 1 + end]
        .split(',')
        .map(clean_label)
        .filter(|value| is_identifier(value))
        .collect()
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn clean_label(value: &str) -> String {
    value
        .trim()
        .trim_matches(['`', '"', '\'', ';', ',', '(', ')', '{', '}', '[', ']'])
        .trim()
        .to_string()
}

fn leading_indent(line: &str) -> usize {
    line.chars()
        .take_while(|character| character.is_whitespace())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_route_handler_jobs_events_bindings_and_config() {
        let extraction = extract_contracts(
            "src/app.ts",
            r#"
router.get('/users', listUsers);
queue.add('refresh-users', payload);
bus.emit('user.updated', user);
bus.on('user.created', handleCreated);
container.bind(UserStore).to(SqlUserStore);
const key = process.env.ANALYTICS_KEY;
const field = getattr(user, 'display_name');
"#,
        );
        for (kind, label) in [
            ("route", "/users"),
            ("handler_reference", "listUsers"),
            ("job_reference", "refresh-users"),
            ("event_reference", "user.updated"),
            ("event_subscription", "user.created"),
            ("dependency_binding", "UserStore -> SqlUserStore"),
            ("configuration_reference", "ANALYTICS_KEY"),
            ("dynamic_reference", "display_name"),
        ] {
            assert!(
                extraction
                    .facts
                    .iter()
                    .any(|fact| fact.kind == kind && fact.label == label),
                "missing {kind} {label}: {:?}",
                extraction.facts
            );
        }
        assert!(extraction
            .links
            .iter()
            .any(|link| link.edge_kind == "routes_to"));
        assert!(extraction
            .links
            .iter()
            .any(|link| link.edge_kind == "binds_to"));
        assert!(extraction.facts.iter().any(|fact| {
            fact.kind == "dynamic_reference" && fact.trust == GraphTrust::Ambiguous
        }));
    }

    #[test]
    fn extracts_sql_openapi_graphql_protobuf_and_dbt_contracts() {
        let sql = extract_contracts(
            "models/orders.sql",
            "CREATE VIEW order_summary AS SELECT * FROM orders JOIN users ON users.id = orders.user_id;",
        );
        assert!(sql.facts.iter().any(|fact| fact.kind == "db_view"));
        assert!(
            sql.facts
                .iter()
                .filter(|fact| fact.kind == "db_object_reference")
                .count()
                >= 2
        );

        let openapi = extract_contracts(
            "openapi.yaml",
            "openapi: 3.1.0\npaths:\n  /users:\n    get:\n      operationId: listUsers\n      $ref: '#/components/schemas/User'\ncomponents:\n  schemas:\n    User:\n      type: object\n",
        );
        assert!(openapi
            .facts
            .iter()
            .any(|fact| fact.kind == "openapi_operation" && fact.label == "GET /users"));
        assert!(openapi
            .facts
            .iter()
            .any(|fact| fact.kind == "schema_reference"));
        assert!(openapi
            .facts
            .iter()
            .any(|fact| fact.kind == "openapi_schema" && fact.label == "User"));
        assert!(openapi
            .links
            .iter()
            .any(|link| link.edge_kind == "implemented_by"));
        let openapi_json = extract_contracts(
            "swagger.json",
            "{\n  \"openapi\": \"3.1.0\",\n  \"paths\": {\n    \"/users\": {\n      \"post\": {\n        \"operationId\": \"createUser\",\n        \"$ref\": \"#/components/schemas/User\"\n      }\n    }\n  }\n}",
        );
        assert!(openapi_json
            .facts
            .iter()
            .any(|fact| { fact.kind == "openapi_operation" && fact.label == "POST /users" }));
        assert!(openapi_json
            .facts
            .iter()
            .any(|fact| fact.kind == "handler_reference" && fact.label == "createUser"));
        assert!(openapi_json.facts.iter().any(|fact| {
            fact.kind == "schema_reference" && fact.label == "#/components/schemas/User"
        }));

        let graphql = extract_contracts(
            "schema.graphql",
            "type User { id: ID! }\nquery UserById($id: ID!) { user(id: $id) { id } }\n",
        );
        assert!(graphql
            .facts
            .iter()
            .any(|fact| fact.kind == "graphql_type" && fact.label == "User"));

        let protobuf = extract_contracts(
            "user.proto",
            "service Users {\n rpc GetUser (GetUserRequest) returns (User);\n}\nmessage GetUserRequest {}\nmessage User {}\n",
        );
        assert!(protobuf
            .facts
            .iter()
            .any(|fact| fact.kind == "protobuf_rpc" && fact.label == "GetUser"));
        assert!(protobuf
            .links
            .iter()
            .any(|link| link.edge_kind == "accepts"));

        let dbt = extract_contracts("models/orders.sql", "select * from {{ ref('users') }}");
        assert!(dbt
            .facts
            .iter()
            .any(|fact| fact.kind == "dbt_model_reference" && fact.label == "users"));
    }
}
