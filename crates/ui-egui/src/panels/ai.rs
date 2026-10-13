//! Pannello **AI**: chat con un agente (BYOK — Anthropic, OpenAI o endpoint
//! OpenAI-compatibile) che monta video eseguendo comandi FilmCraft tramite
//! `filmcraft-ai`, con skill locali, ricerca web opzionale e piano di modifica
//! approvato dall'utente. La chiamata LLM gira su un thread; i comandi sono
//! eseguiti qui, sul thread UI, dalla `Session` vera.
//!
//! Automation ids: `ai.input`, `ai.send`, `ai.approve`, `ai.reject`, `ai.settings`, `ai.newChat`.

use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Rect, vec2};
use filmcraft_ai::config::{AiConfig, ApprovalMode, Provider};
use filmcraft_ai::provider::ToolCall;
use filmcraft_ai::{Agent, Message, Role, Skill, Step};
use serde_json::Value;

use crate::FilmcraftApp;

#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub who: String,
    pub text: String,
}

type LlmResult = Result<Message, String>;

#[derive(Default)]
pub struct AiState {
    pub config: AiConfig,
    loaded: bool,
    pub settings_open: bool,
    pub transcript: Vec<ChatLine>,
    pub input: String,
    agent: Option<Agent>,
    skills: Vec<Skill>,
    rx: Option<Receiver<LlmResult>>,
    pending: Option<Vec<ToolCall>>,
}

fn data_dir(app: &FilmcraftApp) -> Option<std::path::PathBuf> {
    app.session.prefs_path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).or_else(filmcraft_engine::autosave::default_data_dir)
}

fn ensure_loaded(app: &mut FilmcraftApp) {
    if app.ai.loaded {
        return;
    }
    app.ai.loaded = true;
    if let Some(dir) = data_dir(app) {
        app.ai.config = AiConfig::load(&dir);
    }
    let project_dir = app.session.path.as_ref().and_then(|p| std::path::Path::new(p).parent().map(|d| d.to_path_buf()));
    let dirs = filmcraft_ai::skills::user_skill_dirs(data_dir(app).as_deref(), project_dir.as_deref());
    app.ai.skills = filmcraft_ai::skills::load_skills(&dirs);
    app.ai.agent = Some(Agent::new(app.ai.config.clone(), app.ai.skills.clone()));
    app.ai.transcript.push(ChatLine {
        who: "Sistema".into(),
        text: "Ciao! Sono l'assistente AI di FilmCraft. Dimmi cosa vuoi montare: posso cercare i comandi giusti, caricare una skill (trailer, sottotitoli, short 9:16, color grade), cercare sul web come si fa una cosa e proporti un piano prima di toccare il progetto.".into(),
    });
}

/// Spawn the blocking provider call on a worker thread.
fn start_call(app: &mut FilmcraftApp) {
    let Some(agent) = app.ai.agent.as_ref() else { return };
    let cfg = agent.cfg.clone();
    let system = filmcraft_ai::agent::system_prompt(&agent.skills, &agent.cfg);
    let messages = agent.messages.clone();
    let tools = filmcraft_ai::agent::tool_defs(&agent.cfg);
    let (tx, rx): (Sender<LlmResult>, Receiver<LlmResult>) = channel();
    std::thread::spawn(move || {
        let r = filmcraft_ai::provider::chat(&cfg, &system, &messages, &tools).map_err(|e| e.to_string());
        let _ = tx.send(r);
    });
    app.ai.rx = Some(rx);
}

fn describe_call(c: &ToolCall) -> String {
    if c.name == "execute_command" {
        let id = c.args.get("id").and_then(Value::as_str).unwrap_or("?");
        let params = c.args.get("params").map(|p| p.to_string()).unwrap_or_else(|| "{}".into());
        format!("{id}  {params}")
    } else {
        format!("{}  {}", c.name, c.args)
    }
}

fn poll(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    let Some(rx) = app.ai.rx.as_ref() else { return };
    match rx.try_recv() {
        Ok(Ok(msg)) => {
            app.ai.rx = None;
            if !msg.text.trim().is_empty() {
                app.ai.transcript.push(ChatLine { who: "AI".into(), text: msg.text.clone() });
            }
            let Some(mut agent) = app.ai.agent.take() else { return };
            let step = agent.ingest(msg, &mut app.session);
            app.ai.agent = Some(agent);
            match step {
                Step::Done => {}
                Step::Continue => start_call(app),
                Step::NeedsApproval(calls) => {
                    let plan = calls.iter().map(describe_call).collect::<Vec<_>>().join("\n");
                    app.ai.transcript.push(ChatLine { who: "Piano da approvare".into(), text: plan });
                    app.ai.pending = Some(calls);
                }
            }
        }
        Ok(Err(e)) => {
            app.ai.rx = None;
            app.ai.transcript.push(ChatLine { who: "Errore".into(), text: e });
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.ai.rx = None;
            app.ai.transcript.push(ChatLine { who: "Errore".into(), text: "il worker AI si è chiuso".into() });
        }
    }
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    ensure_loaded(app);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 6.0))));
    let ui = &mut child;
    poll(app, ui);

    // ---- header: stato connessione + impostazioni ----
    ui.horizontal(|ui| {
        let ready = app.ai.config.is_ready();
        ui.label(if ready { "🟢 Pronto" } else { "🔴 API key mancante" });
        ui.label(format!("{} · {}", app.ai.config.provider.label(), app.ai.config.model));
        if ui.button("⚙ Impostazioni").clicked() {
            app.ai.settings_open = !app.ai.settings_open;
        }
        if ui.button("Nuova chat").clicked() {
            app.ai.agent = Some(Agent::new(app.ai.config.clone(), app.ai.skills.clone()));
            app.ai.transcript.clear();
            app.ai.pending = None;
        }
    });

    if app.ai.settings_open {
        settings(app, ui);
    }

    ui.separator();

    // ---- transcript ----
    let busy = app.ai.rx.is_some();
    let avail = ui.available_height();
    let input_h = 64.0;
    let plan_h = if app.ai.pending.is_some() { 44.0 } else { 0.0 };
    egui::ScrollArea::vertical()
        .id_salt("ai-transcript")
        .max_height((avail - input_h - plan_h - 12.0).max(80.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in &app.ai.transcript {
                ui.label(format!("{}:", line.who));
                ui.label(&line.text);
                ui.add_space(6.0);
            }
            if busy {
                ui.label("AI: sto pensando…");
            }
        });

    // ---- piano in attesa ----
    if app.ai.pending.is_some() {
        ui.horizontal(|ui| {
            let approve = ui.button("✅ Approva ed esegui");
            app.auto.add_response("ai.approve", &approve, "Approva ed esegui il piano AI");
            if approve.clicked()
                && let Some(calls) = app.ai.pending.take()
            {
                app.ai.transcript.push(ChatLine { who: "Tu".into(), text: "✅ Piano approvato".into() });
                if let Some(mut agent) = app.ai.agent.take() {
                    let _ = agent.approve(&calls, &mut app.session);
                    app.ai.agent = Some(agent);
                }
                start_call(app);
            }
            let reject = ui.button("❌ Rifiuta");
            app.auto.add_response("ai.reject", &reject, "Rifiuta il piano AI");
            if reject.clicked()
                && let Some(calls) = app.ai.pending.take()
            {
                app.ai.transcript.push(ChatLine { who: "Tu".into(), text: "❌ Piano rifiutato".into() });
                if let Some(mut agent) = app.ai.agent.take() {
                    let _ = agent.reject(&calls, "");
                    app.ai.agent = Some(agent);
                }
                start_call(app);
            }
        });
    }

    // ---- input ----
    ui.horizontal(|ui| {
        let edit = egui::TextEdit::multiline(&mut app.ai.input)
            .id_salt("ai-input")
            .hint_text("Es. «Fammi un trailer di 30s», «aggiungi i sottotitoli», «come faccio un match cut?»")
            .desired_rows(3)
            .desired_width(ui.available_width() - 92.0);
        let r = ui.add(edit);
        app.auto.add_response("ai.input", &r, "Messaggio per l'assistente AI");
        let send = ui.add_enabled(!busy && app.ai.pending.is_none(), egui::Button::new("Invia ▶"));
        app.auto.add_response("ai.send", &send, "Invia all'assistente AI");
        let enter = r.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
        if (send.clicked() || enter) && !app.ai.input.trim().is_empty() {
            let text = std::mem::take(&mut app.ai.input);
            app.ai.transcript.push(ChatLine { who: "Tu".into(), text: text.clone() });
            if let Some(agent) = app.ai.agent.as_mut() {
                agent.push_user(&text);
            }
            start_call(app);
        }
    });
    let _ = Role::User; // (ruoli interni dell'agente; la trascrizione UI usa `who`)
}

fn settings(app: &mut FilmcraftApp, ui: &mut egui::Ui) {
    ui.group(|ui| {
        ui.label("Provider e API key (BYOK — la chiave resta sul tuo computer, in ai.json accanto alle preferenze, mai nel progetto)");
        let cfg = &mut app.ai.config;
        egui::ComboBox::from_id_salt("ai-provider").selected_text(cfg.provider.label()).show_ui(ui, |ui| {
            for p in [Provider::Anthropic, Provider::OpenAi, Provider::Compatible] {
                if ui.selectable_value(&mut cfg.provider, p, p.label()).clicked() {
                    cfg.model = p.default_model().to_string();
                    cfg.base_url = p.default_base_url().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Modello");
            ui.add(egui::TextEdit::singleline(&mut cfg.model).desired_width(220.0));
        });
        ui.horizontal(|ui| {
            ui.label("Base URL");
            ui.add(egui::TextEdit::singleline(&mut cfg.base_url).desired_width(220.0));
        });
        ui.horizontal(|ui| {
            ui.label("API key");
            ui.add(egui::TextEdit::singleline(&mut cfg.api_key).password(true).desired_width(220.0).hint_text("incolla qui la tua chiave"));
        });
        if cfg.api_key.is_empty() && !cfg.effective_key().is_empty() {
            ui.label(format!("Chiave da variabile d'ambiente: {}", cfg.masked_key()));
        }
        ui.horizontal(|ui| {
            ui.label("Approvazioni");
            egui::ComboBox::from_id_salt("ai-approval").selected_text(cfg.approval_mode.label()).show_ui(ui, |ui| {
                for m in [ApprovalMode::PlanApprove, ApprovalMode::AutoSafe, ApprovalMode::FullAuto] {
                    ui.selectable_value(&mut cfg.approval_mode, m, m.label());
                }
            });
        });
        ui.checkbox(&mut cfg.web_search, "Consenti ricerca web e lettura pagine (web_search / web_fetch)");
        ui.label("Skill locali: metti file SKILL.md in <dati>/skills/<nome>/ o <progetto>/.filmcraft/skills/<nome>/");
        if ui.button("💾 Salva impostazioni").clicked() {
            match data_dir(app) {
                Some(dir) => match app.ai.config.save(&dir) {
                    Ok(()) => {
                        app.ai.agent = Some(Agent::new(app.ai.config.clone(), app.ai.skills.clone()));
                        app.ai.transcript.push(ChatLine { who: "Sistema".into(), text: "Impostazioni AI salvate.".into() });
                    }
                    Err(e) => app.ai.transcript.push(ChatLine { who: "Errore".into(), text: e }),
                },
                None => app.ai.transcript.push(ChatLine { who: "Errore".into(), text: "cartella dati non trovata".into() }),
            }
        }
    });
}
