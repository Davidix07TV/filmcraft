//! Optional web access for the agent (user-toggleable, off-switch in AiConfig):
//! `web_search` (DuckDuckGo HTML endpoint, no key required) and `web_fetch`
//! (fetch a page and return readable text, hard size/time caps).

const MAX_CHARS: usize = 12_000;

pub fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_ws = true;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_ws {
                        out.push(' ');
                    }
                    last_ws = true;
                } else {
                    out.push(ch);
                    last_ws = false;
                }
            }
            _ => {}
        }
        if out.len() > MAX_CHARS {
            break;
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").trim().to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn get(url: &str) -> Result<String, String> {
    let provider = std::sync::Arc::new(rustls_rustcrypto::provider());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(provider)
                .build(),
        )
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent("FilmCraft-AI/1.0")
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(body.chars().take(MAX_CHARS * 4).collect())
}

#[cfg(target_arch = "wasm32")]
fn get(_url: &str) -> Result<String, String> {
    Err("web non disponibile nella build browser".into())
}

pub fn web_search(query: &str) -> Result<String, String> {
    let q = query.trim();
    if q.is_empty() || q.len() > 300 {
        return Err("query vuota o troppo lunga".into());
    }
    let encoded: String = q
        .chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect();
    let html = get(&format!("https://html.duckduckgo.com/html/?q={encoded}"))?;
    // Result links/snippets are plain anchors in the lite HTML; readable text is enough.
    let text = strip_html(&html);
    let snippet: String = text.chars().take(MAX_CHARS).collect();
    if snippet.is_empty() { Err("nessun risultato leggibile".into()) } else { Ok(snippet) }
}

pub fn web_fetch(url: &str) -> Result<String, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 {
        return Err("URL non valido".into());
    }
    Ok(strip_html(&get(url)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags() {
        assert_eq!(strip_html("<p>Ciao <b>mondo</b></p>"), "Ciao mondo");
    }
}
