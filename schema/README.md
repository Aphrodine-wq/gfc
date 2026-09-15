# Published health schema

[`repository-health.v1.json`](repository-health.v1.json) is the normalized
repository-health inventory schema (`schema_version` `1.0.0`).

Every signal includes structured evidence and `updated_at`. Stale cache
entries must be presented as `cached`, never `current`.
