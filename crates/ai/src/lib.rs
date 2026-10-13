//! FilmCraft's built-in AI assistant (layer L5, beside the UI).
//!
//! Bring-your-own-key chat with Anthropic, OpenAI or any OpenAI-compatible
//! endpoint. The model edits *only* through engine commands — the same
//! `Session::execute` path as the UI, CLI, control channel and MCP — guided by
//! local `SKILL.md` skills and (optionally, user-toggleable) web search.
//! Mutating commands pass through an approval gate whose strictness the user
//! picks (plan / safe-auto / full-auto). See `docs/ai.md`.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod agent;
pub mod config;
pub mod provider;
pub mod skills;
pub mod web;

pub use agent::{Agent, CommandHost, Step};
pub use config::{AiConfig, ApprovalMode, Provider};
pub use provider::{Message, Role, ToolCall};
pub use skills::Skill;
