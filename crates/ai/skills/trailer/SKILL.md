---
name: trailer
description: Taglia un trailer dal materiale in timeline (struttura hook → escalation → climax → titolo)
---

# Skill: Trailer

## Obiettivo
Costruire un trailer di 30–60 secondi dal materiale disponibile nel progetto.

## Procedura
1. Ispeziona il progetto (`sequence.inspect`, project items) per capire clip, durate e contenuti.
2. Proponi all'utente una struttura: **hook** (0–3s, inquadratura/frase d'impatto), **escalation** (montaggio crescente), **climax**, **title card** finale.
3. Presenta il piano con i comandi che intendi eseguire e aspetta l'approvazione (in modalità con approvazione).
4. Esegui tagli corti (1–3s), lascia respirare l'audio nei punti chiave, aggiungi un marker per sezione.
5. Chiudi con un titolo (graphic template esistente) e proponi l'export (`file.exportMedia`) solo dopo conferma esplicita.

## Regole
- Non cancellare mai la sequenza originale: duplicala o creane una nuova.
- Preferisci comandi non distruttivi; ogni taglio deve essere annullabile con un singolo Undo.
