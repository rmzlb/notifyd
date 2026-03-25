use serde_json::Value;

/// Render a template string by substituting {{variable}} with values from vars.
pub fn render(template: &str, vars: &Value) -> String {
    let mut result = template.to_string();

    if let Some(map) = vars.as_object() {
        for (key, value) in map {
            let placeholder = format!("{{{{{}}}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                _ => value.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render() {
        let template = "Hello {{name}}, your appointment is on {{date}} at {{hour}}.";
        let vars = json!({"name": "Jean", "date": "25 mars", "hour": "14h"});
        let result = render(template, &vars);
        assert_eq!(result, "Hello Jean, your appointment is on 25 mars at 14h.");
    }

    #[test]
    fn test_render_missing_var() {
        let template = "Hello {{name}}, status: {{missing}}";
        let vars = json!({"name": "Jean"});
        let result = render(template, &vars);
        assert_eq!(result, "Hello Jean, status: {{missing}}");
    }
}
