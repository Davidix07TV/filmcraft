//! AI assistant configuration (bring-your-own-key), persisted *outside* projects:
//! `<data-dir>/ai.json`. The API key never enters a project file, the journal or a log.
//! File permissions are tightened to `0600` on Unix. Environment variables win over the
//! stored key: `FILMCRAFT_AI_API_KEY`, plus `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Anthropic,
    OpenAi,
    /// Any OpenAI-compatible endpoint (OpenRouter, Ollama, LM Studio, …) via `base_url`.
    Compatible,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic (Claude)",
            Provider::OpenAi => "OpenAI",
            Provider::Compatible => "OpenAI-compatible (custom URL)",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => "claude-sonnet-4-5",
            Provider::OpenAi => "gpt-5",
            Provider::Compatible => "llama3.1",
        }
    }
    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com",
            Provider::Compatible => "http://localhost:11434/v1",
        }
    }
    /// Whether a key is expected (local compatible endpoints usually need none).
    pub fn key_required(self) -> bool {
        !matches!(self, Provider::Compatible)
    }
}

/// How much the agent may do without asking. Chosen by the user in the AI panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalMode {
    /// Show the whole plan (the mutating commands) and wait for Approve/Reject.
    #[default]
    PlanApprove,
    /// Run read-only queries immediately; ask before any mutating command.
    AutoSafe,
    /// Run everything; only log what happened.
    FullAuto,
}

impl ApprovalMode {
    pub fn label(self) -> &'static str {
        match self {
            ApprovalMode::PlanApprove => "Chiedi prima di modificare (piano)",
            ApprovalMode::AutoSafe => "Auto per le letture, chiedi per le modifiche",
            ApprovalMode::FullAuto => "Autonomia totale",
        }
    }
    /// True when a mutating command needs explicit user approval in this mode.
    pub fn requires_approval(self, mutating: bool) -> bool {
        mutating && !matches!(self, ApprovalMode::FullAuto)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    /// Stored key (never serialised into projects/logs). Empty = fall back to env vars.
    pub api_key: String,
    pub approval_mode: ApprovalMode,
    pub web_search: bool,
    pub max_steps: usize,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Anthropic,
            model: Provider::Anthropic.default_model().to_string(),
            base_url: Provider::Anthropic.default_base_url().to_string(),
            api_key: String::new(),
            approval_mode: ApprovalMode::PlanApprove,
            web_search: true,
            max_steps: 24,
        }
    }
}

impl AiConfig {
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join("ai.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        let path = Self::path_in(data_dir);
        std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = Self::path_in(data_dir);
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Effective key: stored value, else environment. Never logged anywhere.
    pub fn effective_key(&self) -> String {
        if !self.api_key.trim().is_empty() {
            return self.api_key.trim().to_string();
        }
        for var in ["FILMCRAFT_AI_API_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
            if let Ok(v) = std::env::var(var)
                && !v.trim().is_empty()
            {
                return v.trim().to_string();
            }
        }
        String::new()
    }

    pub fn is_ready(&self) -> bool {
        !self.provider.key_required() || !self.effective_key().is_empty()
    }

    /// `sk-ant-…abcd` style mask for UI display.
    pub fn masked_key(&self) -> String {
        let k = self.effective_key();
        if k.is_empty() {
            return String::new();
        }
        let tail: String = k.chars().rev().take(4).collect::<String>().chars().rev().collect();
        format!("••••••••{tail}")
    }
}
