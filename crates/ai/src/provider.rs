//! Provider clients (blocking; call from a worker thread, never the UI thread).
//! Two wire formats: Anthropic Messages and OpenAI Chat Completions (also used for
//! every OpenAI-compatible endpoint). Pure request/response (de)serialisation is kept
//! separate from HTTP so it is unit-testable offline.

use serde_json::{Value, json};

use crate::config::{AiConfig, Provider};

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: the id of the call this result answers.
    pub tool_call_id: String,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, text: text.into(), tool_calls: vec![], tool_call_id: String::new() }
    }
    pub fn assistant(text: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: Role::Assistant, text: text.into(), tool_calls, tool_call_id: String::new() }
    }
    pub fn tool_result(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, text: text.into(), tool_calls: vec![], tool_call_id: id.into() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("configura prima una API key per {0} (pannello AI ▸ Impostazioni)")]
    MissingKey(&'static str),
    #[error("rete/HTTP: {0}")]
    Http(String),
    #[error("il provider ha risposto in modo inatteso: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, AiError>;

// ---------- wire format: building requests ----------

pub fn build_request(cfg: &AiConfig, system: &str, messages: &[Message], tools: &[ToolDef]) -> (String, Value) {
    match cfg.provider {
        Provider::Anthropic => {
            let msgs: Vec<Value> = messages
                .iter()
                .filter(|m| !matches!(m.role, Role::System))
                .map(|m| match m.role {
                    Role::Assistant => {
                        let mut content = Vec::new();
                        if !m.text.is_empty() {
                            content.push(json!({"type":"text","text":m.text}));
                        }
                        for c in &m.tool_calls {
                            content.push(json!({"type":"tool_use","id":c.id,"name":c.name,"input":c.args}));
                        }
                        json!({"role":"assistant","content":content})
                    }
                    Role::Tool => json!({"role":"user","content":[{"type":"tool_result","tool_use_id":m.tool_call_id,"content":m.text}]}),
                    _ => json!({"role":"user","content":m.text}),
                })
                .collect();
            let tool_defs: Vec<Value> = tools.iter().map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.input_schema})).collect();
            (
                format!("{}/v1/messages", cfg.base_url.trim_end_matches('/')),
                json!({"model":cfg.model,"max_tokens":4096,"system":system,"messages":msgs,"tools":tool_defs}),
            )
        }
        Provider::OpenAi | Provider::Compatible => {
            let mut msgs = vec![json!({"role":"system","content":system})];
            for m in messages {
                match m.role {
                    Role::System => {}
                    Role::User => msgs.push(json!({"role":"user","content":m.text})),
                    Role::Tool => msgs.push(json!({"role":"tool","tool_call_id":m.tool_call_id,"content":m.text})),
                    Role::Assistant => {
                        let calls: Vec<Value> = m
                            .tool_calls
                            .iter()
                            .map(|c| json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.args.to_string()}}))
                            .collect();
                        let mut v = json!({"role":"assistant","content":m.text});
                        if !calls.is_empty() {
                            v["tool_calls"] = json!(calls);
                        }
                        msgs.push(v);
                    }
                }
            }
            let tool_defs: Vec<Value> =
                tools.iter().map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}})).collect();
            (format!("{}/chat/completions", cfg.base_url.trim_end_matches('/')), json!({"model":cfg.model,"messages":msgs,"tools":tool_defs}))
        }
    }
}

pub fn parse_response(cfg: &AiConfig, v: &Value) -> Result<Message> {
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("errore provider");
        return Err(AiError::Protocol(msg.to_string()));
    }
    match cfg.provider {
        Provider::Anthropic => {
            let mut text = String::new();
            let mut calls = Vec::new();
            for block in v["content"].as_array().map(Vec::as_slice).unwrap_or_default() {
                match block["type"].as_str() {
                    Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
                    Some("tool_use") => calls.push(ToolCall {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        args: block.get("input").cloned().unwrap_or_else(|| json!({})),
                    }),
                    _ => {}
                }
            }
            Ok(Message::assistant(text, calls))
        }
        Provider::OpenAi | Provider::Compatible => {
            let msg = &v["choices"][0]["message"];
            if msg.is_null() {
                return Err(AiError::Protocol("choices[0].message mancante".into()));
            }
            let mut calls = Vec::new();
            for c in msg["tool_calls"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let args_raw = c["function"]["arguments"].as_str().unwrap_or("{}");
                calls.push(ToolCall {
                    id: c["id"].as_str().unwrap_or_default().to_string(),
                    name: c["function"]["name"].as_str().unwrap_or_default().to_string(),
                    args: serde_json::from_str(args_raw).unwrap_or_else(|_| json!({"_raw": args_raw})),
                });
            }
            Ok(Message::assistant(msg["content"].as_str().unwrap_or_default().to_string(), calls))
        }
    }
}

// ---------- HTTP (native only) ----------

#[cfg(not(target_arch = "wasm32"))]
fn agent() -> ureq::Agent {
    let provider = std::sync::Arc::new(rustls_rustcrypto::provider());
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(provider)
                .build(),
        )
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        .timeout_global(Some(std::time::Duration::from_secs(180)))
        .build()
        .into()
}

/// One blocking chat completion. Run on a worker thread.
pub fn chat(cfg: &AiConfig, system: &str, messages: &[Message], tools: &[ToolDef]) -> Result<Message> {
    let key = cfg.effective_key();
    if cfg.provider.key_required() && key.is_empty() {
        return Err(AiError::MissingKey(cfg.provider.label()));
    }
    let (url, body) = build_request(cfg, system, messages, tools);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (url, body, key);
        return Err(AiError::Http("le chiamate di rete non sono disponibili nella build web; usa la app desktop".into()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut req = agent().post(&url).header("content-type", "application/json");
        req = match cfg.provider {
            Provider::Anthropic => req.header("x-api-key", &key).header("anthropic-version", "2023-06-01"),
            Provider::OpenAi | Provider::Compatible => {
                if key.is_empty() {
                    req
                } else {
                    req.header("authorization", &format!("Bearer {key}"))
                }
            }
        };
        let mut resp = req.send(&body.to_string()).map_err(|e| AiError::Http(e.to_string()))?;
        let raw = resp.body_mut().read_to_string().map_err(|e| AiError::Http(e.to_string()))?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| AiError::Http(e.to_string()))?;
        parse_response(cfg, &v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(p: Provider) -> AiConfig {
        AiConfig { provider: p, ..Default::default() }
    }

    #[test]
    fn anthropic_roundtrip() {
        let c = cfg(Provider::Anthropic);
        let (_url, body) = build_request(
            &c,
            "sys",
            &[Message::user("ciao")],
            &[ToolDef { name: "execute_command".into(), description: "d".into(), input_schema: json!({"type":"object"}) }],
        );
        assert_eq!(body["system"], "sys");
        assert_eq!(body["tools"][0]["name"], "execute_command");
        let m = parse_response(
            &c,
            &json!({"content":[{"type":"text","text":"ok"},{"type":"tool_use","id":"1","name":"execute_command","input":{"id":"edit.undo"}}]}),
        )
        .expect("parse");
        assert_eq!(m.text, "ok");
        assert_eq!(m.tool_calls[0].args["id"], "edit.undo");
    }

    #[test]
    fn openai_roundtrip() {
        let c = cfg(Provider::OpenAi);
        let (_url, body) = build_request(&c, "sys", &[Message::user("ciao")], &[]);
        assert_eq!(body["messages"][0]["role"], "system");
        let m = parse_response(
            &c,
            &json!({"choices":[{"message":{"content":"fatto","tool_calls":[{"id":"9","function":{"name":"web_search","arguments":"{\"q\":\"x\"}"}}]}}]}),
        )
        .expect("parse");
        assert_eq!(m.tool_calls[0].name, "web_search");
    }
}
