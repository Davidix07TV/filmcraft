# AI assistant (pannello AI)

FilmCraft ha un assistente AI integrato, **bring-your-own-key**: l'utente collega la
propria API key e chatta con un agente che monta video al posto suo (o insieme a lui).

## Dove si usa

- **App desktop/web**: pannello **AI** (menu Window ▸ AI). Chat, impostazioni provider,
  piano di modifica con pulsanti **Approva / Rifiuta**, trascrizione delle azioni.
- **Headless**: `filmcraft-cli ai "fammi un trailer di 30 secondi" --demo --save-as out.fcproj`
  (approvazione del piano da stdin).

## Provider supportati

| Provider | Formato | Key |
|---|---|---|
| Anthropic (Claude) | Messages API | `x-api-key`, `ANTHROPIC_API_KEY` |
| OpenAI | Chat Completions | Bearer, `OPENAI_API_KEY` |
| OpenAI-compatible (OpenRouter, Ollama, LM Studio…) | Chat Completions su `base_url` | opzionale |

La configurazione vive in `<data-dir>/ai.json` (permessi `0600` su Unix), **mai** nel
file di progetto, nel journal o nei log. In alternativa: `FILMCRAFT_AI_API_KEY`.

## Come edita l'agente

L'agente non tocca mai lo stato interno: usa gli strumenti `list_commands`,
`describe_command` ed `execute_command`, cioè lo stesso `Session::execute` di UI, CLI,
control channel e MCP (650+ comandi). Prima ispeziona, poi agisce, poi riassume.

## Modalità di approvazione (scelta dall'utente)

1. **Piano** (`PlanApprove`, default): ogni esecuzione comando attende l'OK sul piano mostrato.
2. **Auto sicura** (`AutoSafe`): le query di sola lettura partono subito; le modifiche attendono l'OK.
3. **Autonomia totale** (`FullAuto`): tutto automatico, resta solo il log nella chat.

Il rifiuto viene rimandato al modello, che propone un'alternativa.

## Skill

Le skill sono file `SKILL.md` (frontmatter `name`/`description` + istruzioni markdown).
Incorporate: `trailer`, `captions`, `social-short`, `color-grade` (`crates/ai/skills/`).
L'utente può aggiungerne/sovrascriverne in `<data-dir>/skills/<nome>/SKILL.md` o
`<progetto>/.filmcraft/skills/<nome>/SKILL.md`. L'agente vede il catalogo nel prompt
di sistema e carica il corpo completo con lo strumento `load_skill`.

## Ricerca web

Con l'opzione attiva l'agente può usare `web_search` (DuckDuckGo HTML, nessuna key)
e `web_fetch` per imparare *come* fare una cosa, poi proporla all'utente prima di
eseguirla. Limiti rigidi su dimensione/tempo; disattivabile nelle impostazioni.

## Architettura

`crates/ai` (`filmcraft-ai`, L5): `config` (BYOK + approvazioni), `provider`
(traduzione Anthropic/OpenAI, HTTP ureq+rustls solo nativo), `agent` (loop a stati,
gate di approvazione, host astratto — testabile con un fake), `skills`, `web`.
La UI (`panels/ai.rs`) esegue i comandi sul thread UI con la `Session` vera; la
chiamata LLM bloccante gira su un worker thread.
