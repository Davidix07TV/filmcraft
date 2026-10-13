//! The agent loop, split so the UI never blocks: the egui thread owns the
//! [`CommandHost`] (the engine `Session`), a worker thread runs the blocking
//! provider call, and this module decides — deterministically — what happens to
//! each tool call the model asks for, including the user's approval gate.

use serde_json::{Value, json};

use crate::config::{AiConfig, ApprovalMode};
use crate::provider::{Message, ToolCall, ToolDef};
use crate::skills::Skill;
use crate::web;

/// Everything the agent may do to the editor. Implemented for `Session` below;
/// tests can substitute a recording fake.
pub trait CommandHost {
    fn search_commands(&self, query: &str, limit: usize) -> Value;
    fn describe_command(&self, id: &str) -> Result<Value, String>;
    /// `None` = unknown command id.
    fn is_mutating(&self, id: &str) -> Option<bool>;
    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String>;
}

impl CommandHost for filmcraft_engine::Session {
    fn search_commands(&self, query: &str, limit: usize) -> Value {
        let q = query.to_ascii_lowercase();
        let hits: Vec<Value> = filmcraft_engine::command_specs()
            .iter()
            .filter(|c| q.is_empty() || c.id.to_ascii_lowercase().contains(&q) || c.label.to_ascii_lowercase().contains(&q))
            .take(limit.max(1))
            .map(|c| json!({"id": c.id, "label": c.label, "mutating": c.journal, "params": c.params}))
            .collect();
        json!({"commands": hits, "totalCatalogue": filmcraft_engine::command_specs().len()})
    }

    fn describe_command(&self, id: &str) -> Result<Value, String> {
        filmcraft_engine::find_command(id)
            .map(|c| json!({"id": c.id, "label": c.label, "mutating": c.journal, "params": c.params, "menu": c.menu}))
            .ok_or_else(|| format!("comando sconosciuto `{id}`: usa list_commands"))
    }

    fn is_mutating(&self, id: &str) -> Option<bool> {
        filmcraft_engine::find_command(id).map(|c| c.journal)
    }

    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        filmcraft_engine::Session::execute(self, id, params.clone()).map_err(|e| e.to_string())
    }
}

pub fn tool_defs(cfg: &AiConfig) -> Vec<ToolDef> {
    let mut v = vec![
        ToolDef {
            name: "list_commands".into(),
            description: "Cerca nel catalogo comandi FilmCraft (id, label, parametri). Usalo prima di inventare un id.".into(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolDef {
            name: "describe_command".into(),
            description: "Dettagli e parametri di un comando per id esatto.".into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
        },
        ToolDef {
            name: "execute_command".into(),
            description: "Esegue un comando FilmCraft sull'editor (tagli, titoli, export…). I comandi che modificano il progetto possono richiedere l'approvazione dell'utente in base alla modalità scelta.".into(),
            input_schema: json!({"type":"object","properties":{"id":{"type":"string"},"params":{"type":"object"}},"required":["id"]}),
        },
        ToolDef {
            name: "load_skill".into(),
            description: "Carica le istruzioni complete di una skill dal catalogo (nome esatto).".into(),
            input_schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
        },
    ];
    if cfg.web_search {
        v.push(ToolDef {
            name: "web_search".into(),
            description: "Cerca sul web (come si fa una cosa, documentazione, tutorial). Riassumi le fonti all'utente invece di incollare testo.".into(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        });
        v.push(ToolDef {
            name: "web_fetch".into(),
            description: "Legge il testo di una pagina web per URL.".into(),
            input_schema: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        });
    }
    v
}

pub fn system_prompt(skills: &[Skill], cfg: &AiConfig) -> String {
    let catalogue: Vec<String> = skills.iter().map(|s| format!("- {}: {}", s.name, s.description)).collect();
    let mode_note = match cfg.approval_mode {
        ApprovalMode::PlanApprove => {
            "Modalità approvazione: PIANO — prima di modificare, spiega il piano all'utente; i comandi di modifica verranno eseguiti solo dopo il suo OK esplicito nell'interfaccia."
        }
        ApprovalMode::AutoSafe => "Modalità approvazione: le letture sono automatiche, le modifiche richiedono l'OK dell'utente nell'interfaccia.",
        ApprovalMode::FullAuto => "Modalità approvazione: autonomia totale concessa dall'utente; agisci e poi riassumi cosa hai fatto.",
    };
    format!(
        "Sei l'assistente AI integrato in FilmCraft, un editor video non lineare (equivalente open-source di Premiere Pro). \
Aiuti l'utente a montare video usando SOLO gli strumenti forniti: non inventare mai id di comando — cercali con list_commands e verificali con describe_command. \
I tempi nell'editor sono tick esatti; preferisci i comandi di ispezione (es. sequence.inspect) per capire lo stato prima di agire. \
Se una richiesta è ambigua, fai una domanda breve invece di indovinare. Quando proponi un montaggio, chiedi conferma che vada bene così prima di export irreversibili. \
Rispondi nella lingua dell'utente, in modo conciso, e riassumi sempre cosa hai cambiato.\n{mode_note}\n\
Skill disponibili (caricale con load_skill quando pertinenti):\n{}",
        catalogue.join("\n")
    )
}

pub enum Step {
    /// The turn is over; show this final text (may be empty if already in transcript).
    Done,
    /// Mutating/consequential calls are waiting for the user. UI shows Approve/Reject.
    NeedsApproval(Vec<ToolCall>),
    /// Tool results were appended; call the provider again.
    Continue,
}

pub struct Agent {
    pub cfg: AiConfig,
    pub skills: Vec<Skill>,
    pub messages: Vec<Message>,
    pub llm_calls: usize,
}

impl Agent {
    pub fn new(cfg: AiConfig, skills: Vec<Skill>) -> Self {
        Self { cfg, skills, messages: Vec::new(), llm_calls: 0 }
    }

    pub fn push_user(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    /// Run the provider call for the current state (blocking — worker thread only).
    pub fn complete(&self) -> Result<Message, crate::provider::AiError> {
        crate::provider::chat(&self.cfg, &system_prompt(&self.skills, &self.cfg), &self.messages, &tool_defs(&self.cfg))
    }

    fn needs_approval(&self, call: &ToolCall, host: &dyn CommandHost) -> bool {
        if call.name != "execute_command" {
            return false;
        }
        let id = call.args.get("id").and_then(Value::as_str).unwrap_or_default();
        match self.cfg.approval_mode {
            ApprovalMode::FullAuto => false,
            ApprovalMode::PlanApprove => true,
            ApprovalMode::AutoSafe => host.is_mutating(id).unwrap_or(true),
        }
    }

    fn run_tool(&self, call: &ToolCall, host: &mut dyn CommandHost) -> String {
        let arg = |k: &str| call.args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let result: Result<Value, String> = match call.name.as_str() {
            "list_commands" => Ok(host.search_commands(&arg("query"), 40)),
            "describe_command" => host.describe_command(&arg("id")),
            "load_skill" => self
                .skills
                .iter()
                .find(|s| s.name == arg("name"))
                .map(|s| json!({"name": s.name, "instructions": s.body}))
                .ok_or_else(|| format!("skill `{}` non trovata", arg("name"))),
            "web_search" if self.cfg.web_search => web::web_search(&arg("query")).map(Value::String),
            "web_fetch" if self.cfg.web_search => web::web_fetch(&arg("url")).map(Value::String),
            "execute_command" => {
                let params = call.args.get("params").cloned().unwrap_or_else(|| json!({}));
                host.execute(&arg("id"), &params)
            }
            other => Err(format!("strumento sconosciuto o non abilitato: {other}")),
        };
        match result {
            Ok(v) => {
                let s = v.to_string();
                s.chars().take(16_000).collect()
            }
            Err(e) => format!("ERRORE: {e}"),
        }
    }

    /// Feed one provider response: executes ungated tools, gates the rest.
    pub fn ingest(&mut self, resp: Message, host: &mut dyn CommandHost) -> Step {
        self.llm_calls += 1;
        let calls = resp.tool_calls.clone();
        self.messages.push(resp);
        if calls.is_empty() {
            return Step::Done;
        }
        if self.llm_calls > self.cfg.max_steps.max(1) {
            self.messages.push(Message::user("[sistema] limite di passi raggiunto: riassumi e fermati"));
            return Step::Done;
        }
        let mut gated = Vec::new();
        for call in &calls {
            if self.needs_approval(call, host) {
                gated.push(call.clone());
            } else {
                let out = self.run_tool(call, host);
                self.messages.push(Message::tool_result(call.id.clone(), out));
            }
        }
        if gated.is_empty() { Step::Continue } else { Step::NeedsApproval(gated) }
    }

    /// Execute the gated calls after the user pressed Approve.
    pub fn approve(&mut self, calls: &[ToolCall], host: &mut dyn CommandHost) -> Step {
        for call in calls {
            let out = self.run_tool(call, host);
            self.messages.push(Message::tool_result(call.id.clone(), out));
        }
        Step::Continue
    }

    /// Record a rejection so the model can offer an alternative.
    pub fn reject(&mut self, calls: &[ToolCall], reason: &str) -> Step {
        for call in calls {
            self.messages.push(Message::tool_result(
                call.id.clone(),
                format!(
                    "RIFIUTATO dall'utente{}. Proponi un'alternativa o chiedi indicazioni.",
                    if reason.is_empty() { String::new() } else { format!(": {reason}") }
                ),
            ));
        }
        Step::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeHost {
        executed: Vec<String>,
    }
    impl CommandHost for FakeHost {
        fn search_commands(&self, _q: &str, _l: usize) -> Value {
            json!({"commands": []})
        }
        fn describe_command(&self, id: &str) -> Result<Value, String> {
            Ok(json!({"id": id}))
        }
        fn is_mutating(&self, id: &str) -> Option<bool> {
            Some(id != "sequence.inspect")
        }
        fn execute(&mut self, id: &str, _p: &Value) -> Result<Value, String> {
            self.executed.push(id.to_string());
            Ok(json!({"ok": true}))
        }
    }

    fn call(id: &str, cmd: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "execute_command".into(), args: json!({"id": cmd, "params": {}}) }
    }

    #[test]
    fn plan_mode_gates_every_execute() {
        let mut a = Agent::new(AiConfig::default(), vec![]);
        let mut h = FakeHost::default();
        match a.ingest(Message::assistant("", vec![call("1", "sequence.inspect")]), &mut h) {
            Step::NeedsApproval(c) => assert_eq!(c.len(), 1),
            _ => panic!("expected approval gate"),
        }
        assert!(h.executed.is_empty());
    }

    #[test]
    fn autosafe_runs_queries_and_gates_mutations() {
        let cfg = AiConfig { approval_mode: ApprovalMode::AutoSafe, ..Default::default() };
        let mut a = Agent::new(cfg, vec![]);
        let mut h = FakeHost::default();
        match a.ingest(Message::assistant("", vec![call("1", "sequence.inspect"), call("2", "edit.undo")]), &mut h) {
            Step::NeedsApproval(c) => assert_eq!(c[0].id, "2"),
            _ => panic!("expected gate for mutation only"),
        }
        assert_eq!(h.executed, ["sequence.inspect"]);
        match a.approve(&[call("2", "edit.undo")], &mut h) {
            Step::Continue => {}
            _ => panic!("expected continue"),
        }
        assert_eq!(h.executed, ["sequence.inspect", "edit.undo"]);
    }

    #[test]
    fn rejection_is_fed_back() {
        let mut a = Agent::new(AiConfig::default(), vec![]);
        let mut h = FakeHost::default();
        let _ = a.ingest(Message::assistant("", vec![call("1", "edit.undo")]), &mut h);
        let _ = a.reject(&[call("1", "edit.undo")], "troppo aggressivo");
        assert!(a.messages.last().map(|m| m.text.contains("RIFIUTATO")).unwrap_or(false));
    }
}
