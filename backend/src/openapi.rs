use serde_json::{Map, Value, json};

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub location: String,
    pub required: bool,
    pub schema: Value,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub method: String,
    pub path: String,
    pub summary: String,
    pub description: String,
    pub operation_id: String,
    pub tags: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub request_body: Option<Value>,
    pub responses: Value,
}

impl Operation {
    /// `GET /api/client/servers — List servers (query: page, per_page, search)`
    fn summary_line(&self) -> String {
        let mut line = format!("{} {}", self.method, self.path);

        if !self.summary.is_empty() {
            line.push_str(" — ");
            line.push_str(&self.summary);
        }

        let mut notes = Vec::new();

        let query: Vec<&str> = self
            .parameters
            .iter()
            .filter(|parameter| parameter.location == "query")
            .map(|parameter| parameter.name.as_str())
            .collect();
        if !query.is_empty() {
            notes.push(format!("query: {}", query.join(", ")));
        } else if self.request_body.is_some() {
            notes.push("body".to_string());
        }

        if !notes.is_empty() {
            line.push_str(&format!(" ({})", notes.join("; ")));
        }

        line
    }
}

pub struct Index {
    pub operations: Vec<Operation>,
    pub schemas: Value,
}

impl Index {
    pub fn parse(document: &Value) -> Result<Self, String> {
        let paths = document
            .get("paths")
            .and_then(Value::as_object)
            .ok_or_else(|| "the openapi document has no `paths` object".to_string())?;

        let schemas = document
            .get("components")
            .and_then(|components| components.get("schemas"))
            .cloned()
            .unwrap_or_else(|| json!({}));

        let mut operations = Vec::new();

        for (path, item) in paths {
            let Some(item) = item.as_object() else {
                continue;
            };

            for method in ["get", "post", "put", "patch", "delete"] {
                let Some(operation) = item.get(method).and_then(Value::as_object) else {
                    continue;
                };

                let parameters = operation
                    .get("parameters")
                    .and_then(Value::as_array)
                    .map(|parameters| {
                        parameters
                            .iter()
                            .filter_map(|parameter| {
                                let parameter = parameter.as_object()?;

                                Some(Parameter {
                                    name: parameter.get("name")?.as_str()?.to_string(),
                                    location: parameter
                                        .get("in")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                    required: parameter
                                        .get("required")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                    schema: parameter.get("schema").cloned().unwrap_or(Value::Null),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                operations.push(Operation {
                    method: method.to_ascii_uppercase(),
                    path: path.clone(),
                    summary: operation
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    description: operation
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    operation_id: operation
                        .get("operationId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    tags: operation
                        .get("tags")
                        .and_then(Value::as_array)
                        .map(|tags| {
                            tags.iter()
                                .filter_map(|tag| tag.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    parameters,
                    request_body: operation.get("requestBody").cloned(),
                    responses: operation
                        .get("responses")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                });
            }
        }

        Ok(Self {
            operations,
            schemas,
        })
    }

    pub fn search(&self, query: &str, scope: &str, limit: usize) -> Vec<String> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|token| token.to_ascii_lowercase())
            .collect();

        let mut scored: Vec<(u32, &Operation)> = self
            .operations
            .iter()
            .filter(|operation| in_scope(&operation.path, scope))
            .filter_map(|operation| {
                let mut score = 0;
                let mut matched = tokens.is_empty();

                for token in &tokens {
                    if operation.path.to_ascii_lowercase().contains(token) {
                        score += 10;
                        matched = true;
                    }
                    if operation.summary.to_ascii_lowercase().contains(token) {
                        score += 5;
                        matched = true;
                    }
                    if operation.description.to_ascii_lowercase().contains(token) {
                        score += 3;
                        matched = true;
                    }
                    if operation.operation_id.to_ascii_lowercase().contains(token) {
                        score += 2;
                        matched = true;
                    }
                    if operation
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(token))
                    {
                        score += 1;
                        matched = true;
                    }
                }

                matched.then_some((score, operation))
            })
            .collect();

        scored.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.method.cmp(&right.method))
        });

        scored
            .into_iter()
            .take(limit)
            .map(|(_, operation)| operation.summary_line())
            .collect()
    }

    pub fn find(&self, method: &str, path: &str) -> Option<&Operation> {
        self.operations.iter().find(|operation| {
            operation.path == path && operation.method.eq_ignore_ascii_case(method)
        })
    }

    pub fn describe(&self, operation: &Operation) -> String {
        let parameters: Vec<Value> = operation
            .parameters
            .iter()
            .map(|parameter| {
                json!({
                    "name": parameter.name,
                    "in": parameter.location,
                    "required": parameter.required,
                    "schema": parameter.schema,
                })
            })
            .collect();

        let mut document = Map::new();
        document.insert("method".to_string(), json!(operation.method));
        document.insert("path".to_string(), json!(operation.path));
        document.insert("summary".to_string(), json!(operation.summary));
        document.insert("description".to_string(), json!(operation.description));
        document.insert("operation_id".to_string(), json!(operation.operation_id));
        document.insert("tags".to_string(), json!(operation.tags));
        document.insert("parameters".to_string(), Value::Array(parameters));
        document.insert(
            "request_body".to_string(),
            operation.request_body.clone().unwrap_or(Value::Null),
        );
        document.insert("responses".to_string(), operation.responses.clone());
        document.insert(
            "components".to_string(),
            json!({ "schemas": self.reachable_schemas(operation) }),
        );

        serde_json::to_string_pretty(&Value::Object(document)).unwrap_or_default()
    }

    /// The schemas this operation's `$ref`s resolve to, followed transitively. Shipping the whole
    /// `components.schemas` would bury the operation under the panel's entire schema set and push
    /// exactly those definitions past the truncation limit.
    fn reachable_schemas(&self, operation: &Operation) -> Value {
        let mut wanted = Vec::new();
        collect_refs(operation.request_body.as_ref(), &mut wanted);
        collect_refs(Some(&operation.responses), &mut wanted);

        let mut schemas = Map::new();
        let mut index = 0;

        while index < wanted.len() {
            let name = wanted[index].clone();
            index += 1;

            if schemas.contains_key(&name) {
                continue;
            }

            let Some(schema) = self.schemas.get(&name).cloned() else {
                continue;
            };

            collect_refs(Some(&schema), &mut wanted);
            schemas.insert(name, schema);
        }

        Value::Object(schemas)
    }
}

fn collect_refs(value: Option<&Value>, wanted: &mut Vec<String>) {
    match value {
        Some(Value::Object(map)) => {
            if let Some(name) = map
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
            {
                wanted.push(name.to_string());
            }

            for value in map.values() {
                collect_refs(Some(value), wanted);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_refs(Some(item), wanted);
            }
        }
        _ => {}
    }
}

fn in_scope(path: &str, scope: &str) -> bool {
    match scope {
        "client" => path.starts_with("/api/client"),
        "admin" => path.starts_with("/api/admin"),
        _ => !path.starts_with("/api/auth") && !path.starts_with("/api/remote"),
    }
}
