//! GraphQL schema introspection.

use serde_json::Value;

pub const INTROSPECTION_QUERY: &str = r#"query IntrospectionQuery {
  __schema {
    queryType { name }
    mutationType { name }
    subscriptionType { name }
    types {
      kind name description
      fields(includeDeprecated: true) { name description args { name type { ...TypeRef } } type { ...TypeRef } }
      inputFields { name type { ...TypeRef } }
      enumValues(includeDeprecated: true) { name }
    }
  }
}
fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }"#;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldInfo {
    pub name: String,
    pub type_name: String,
    pub args: Vec<(String, String)>,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TypeInfo {
    pub name: String,
    pub kind: String,
    pub fields: Vec<FieldInfo>,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub query_type: String,
    pub mutation_type: String,
    pub subscription_type: String,
    pub types: Vec<TypeInfo>,
}

impl Schema {
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.types.iter().find(|t| t.name == name)
    }
    pub fn roots(&self) -> Vec<(&'static str, &TypeInfo)> {
        let mut out = Vec::new();
        for (label, name) in [
            ("Query", &self.query_type),
            ("Mutation", &self.mutation_type),
            ("Subscription", &self.subscription_type),
        ] {
            if let Some(t) = self.get(name) {
                out.push((label, t));
            }
        }
        out
    }
}

fn type_ref(v: &Value) -> String {
    match v.get("kind").and_then(Value::as_str) {
        Some("NON_NULL") => format!("{}!", type_ref(&v["ofType"])),
        Some("LIST") => format!("[{}]", type_ref(&v["ofType"])),
        _ => v.get("name").and_then(Value::as_str).unwrap_or("?").to_string(),
    }
}

/// Strip list/non-null wrappers: `[User!]!` → `User`.
pub fn base_type(t: &str) -> &str {
    t.trim_matches(|c| c == '[' || c == ']' || c == '!')
}

pub fn parse_introspection(body: &str) -> Result<Schema, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("Introspection response is not JSON: {e}"))?;
    if let Some(errs) = v.get("errors") {
        if v.get("data").map(Value::is_null).unwrap_or(true) {
            return Err(format!("Introspection failed: {errs}"));
        }
    }
    let s = v.pointer("/data/__schema").ok_or("No __schema in response (is introspection disabled?)")?;
    let name_of = |k: &str| s.get(k).and_then(|t| t.get("name")).and_then(Value::as_str).unwrap_or("").to_string();
    let mut types: Vec<TypeInfo> = s["types"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|t| !t["name"].as_str().unwrap_or("").starts_with("__"))
        .map(|t| {
            let fields_src = t["fields"].as_array().or_else(|| t["inputFields"].as_array());
            let mut fields: Vec<FieldInfo> = fields_src
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|f| FieldInfo {
                    name: f["name"].as_str().unwrap_or("").into(),
                    type_name: type_ref(&f["type"]),
                    args: f["args"]
                        .as_array()
                        .map(|a| a.as_slice())
                        .unwrap_or_default()
                        .iter()
                        .map(|a| (a["name"].as_str().unwrap_or("").to_string(), type_ref(&a["type"])))
                        .collect(),
                    description: f["description"].as_str().unwrap_or("").into(),
                })
                .collect();
            if let Some(ev) = t["enumValues"].as_array() {
                fields.extend(ev.iter().map(|e| FieldInfo {
                    name: e["name"].as_str().unwrap_or("").into(),
                    type_name: String::new(),
                    ..Default::default()
                }));
            }
            TypeInfo {
                name: t["name"].as_str().unwrap_or("").into(),
                kind: t["kind"].as_str().unwrap_or("").into(),
                fields,
                description: t["description"].as_str().unwrap_or("").into(),
            }
        })
        .collect();
    types.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Schema {
        query_type: name_of("queryType"),
        mutation_type: name_of("mutationType"),
        subscription_type: name_of("subscriptionType"),
        types,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_schema() {
        let body = r#"{"data":{"__schema":{"queryType":{"name":"Query"},"mutationType":null,"subscriptionType":null,
          "types":[{"kind":"OBJECT","name":"Query","fields":[{"name":"user","args":[{"name":"id","type":{"kind":"NON_NULL","name":null,"ofType":{"kind":"SCALAR","name":"ID"}}}],
          "type":{"kind":"LIST","name":null,"ofType":{"kind":"OBJECT","name":"User"}}}]},{"kind":"OBJECT","name":"__Type","fields":[]}]}}}"#;
        let s = parse_introspection(body).unwrap();
        assert_eq!(s.types.len(), 1);
        let f = &s.roots()[0].1.fields[0];
        assert_eq!(f.type_name, "[User]");
        assert_eq!(f.args[0], ("id".to_string(), "ID!".to_string()));
        assert_eq!(base_type("[User!]!"), "User");
    }
}
