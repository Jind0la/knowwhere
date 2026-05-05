# KnowWhere Hermes Memory Provider

This directory mirrors the active Hermes memory provider plugin.

Runtime location:

```bash
~/.hermes/plugins/knowwhere/__init__.py
```

Install or update from the repository:

```bash
mkdir -p ~/.hermes/plugins/knowwhere
cp hermes-plugin/knowwhere/__init__.py ~/.hermes/plugins/knowwhere/__init__.py
```

The plugin is additive: if KnowWhere is unavailable, Hermes continues with its built-in memory.

Current default behavior:

- calls `POST /retrieve_fractal` without `reflect` for normal prefetch
- filters Meta and XML memory artifacts before prompt injection
- performs a separate decision-filtered retrieval for `[KW-DECISION]`
- stores each turn with `session_id`, `turn_index`, `observed_at`, `claim_scope`, and Hermes source metadata
