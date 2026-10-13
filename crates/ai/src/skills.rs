//! Local skills: `SKILL.md` files (frontmatter `name`/`description` + markdown body).
//! Built-ins ship in `crates/ai/skills/`; users may add their own in
//! `<data-dir>/skills/<name>/SKILL.md` or `<project>/.filmcraft/skills/…` — same-named
//! user skills override built-ins. The model sees the catalogue in its system prompt
//! and loads a full skill body on demand with the `load_skill` tool.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub origin: String,
}

const BUILTINS: &[(&str, &str)] = &[
    ("trailer", include_str!("../skills/trailer/SKILL.md")),
    ("captions", include_str!("../skills/captions/SKILL.md")),
    ("social-short", include_str!("../skills/social-short/SKILL.md")),
    ("color-grade", include_str!("../skills/color-grade/SKILL.md")),
];

fn parse(raw: &str, origin: &str) -> Option<Skill> {
    let mut name = String::new();
    let mut description = String::new();
    let mut body = raw;
    if let Some(rest) = raw.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        for line in rest[..end].lines() {
            if let Some(v) = line.strip_prefix("name:") {
                name = v.trim().trim_matches('"').to_string();
            }
            if let Some(v) = line.strip_prefix("description:") {
                description = v.trim().trim_matches('"').to_string();
            }
        }
        body = &rest[end + 4..];
    }
    if name.is_empty() {
        return None;
    }
    if description.is_empty() {
        description = body.lines().next().unwrap_or_default().trim_start_matches('#').trim().chars().take(160).collect();
    }
    Some(Skill { name, description, body: body.trim().to_string(), origin: origin.to_string() })
}

/// All skills: built-ins first, then user skills (which override by name).
pub fn load_skills(extra_dirs: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = BUILTINS.iter().filter_map(|(n, raw)| parse(raw, &format!("built-in:{n}"))).collect();
    for dir in extra_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let file = e.path().join("SKILL.md");
            let Ok(raw) = std::fs::read_to_string(&file) else { continue };
            if let Some(s) = parse(&raw, &format!("file:{}", file.display())) {
                out.retain(|x| x.name != s.name);
                out.push(s);
            }
        }
    }
    out
}

pub fn user_skill_dirs(data_dir: Option<&Path>, project_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = data_dir {
        v.push(d.join("skills"));
    }
    if let Some(d) = project_dir {
        v.push(d.join(".filmcraft").join("skills"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        let s = load_skills(&[]);
        assert!(s.len() >= 4, "{s:?}");
        assert!(s.iter().all(|x| !x.body.is_empty() && !x.description.is_empty()));
    }

    #[test]
    fn user_skill_overrides_builtin() {
        let dir = std::env::temp_dir().join(format!("fc-ai-skill-test-{}", std::process::id()));
        let sub = dir.join("trailer");
        std::fs::create_dir_all(&sub).ok();
        std::fs::write(sub.join("SKILL.md"), "---\nname: trailer\ndescription: custom\n---\n# Custom\n").ok();
        let s = load_skills(&[dir.clone()]);
        let t = s.iter().find(|x| x.name == "trailer").expect("trailer skill");
        assert_eq!(t.description, "custom");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
