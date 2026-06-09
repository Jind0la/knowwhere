# Breaking Changes

## v0.4.0 — Docker Compose: Ollama Service entfernt

**Datum:** 2026-05-01

**Änderung:** Der `ollama` Service wurde aus `docker-compose.yml` entfernt.

**Grund:** macOS Docker-Desktop kann llama3.2 (~4 GB) auf M1 (8 GB RAM) nicht laden.
Der Ollama-Service crasht mit "llama runner no longer running".

**Migration:** KnowWhere nutzt jetzt natives macOS-Ollama via `host.docker.internal`.

**Schritte:**
1. Ollama nativ installieren: `brew install ollama`
2. Model pullen: `ollama pull snowflake-arctic-embed2 && ollama pull llama3.2`
3. `OLLAMA_URL=http://host.docker.internal:11434` in `.env` setzen (oder Docker-Compose env)

**Betroffene Nutzer:** Nur macOS Docker-Nutzer. Native-Linux-Deployments sind nicht betroffen
(Ollama kann dort weiterhin als Container laufen, einfach wieder zur docker-compose.yml hinzufügen).

## v0.4.0 — Google Drive Connector jetzt Feature-gated

**Änderung:** `google-drive3`, `yup-oauth2`, `hyper` (~20 transitive Deps) sind jetzt hinter
`--features google-drive` Feature-Flag.

**Neuer Build-Befehl:**
```bash
cargo build --features "postgres-storage,summarizer,google-drive"
```

Ohne das Flag wird `GoogleDriveConnector` nicht kompiliert.
