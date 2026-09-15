//! Metadata-only local cache. Tokens must never be stored here.

pub mod metrics;

use std::path::Path;

use chrono::{DateTime, Utc};
use gfc_schema::{
    present_freshness, FreshnessPresentedAs, HealthThresholds, Inventory, RepositoryHealth,
    SCHEMA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Cache {
    conn: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self, CacheError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS repos (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                health_json TEXT NOT NULL,
                local_updated_at TEXT NOT NULL,
                remote_updated_at TEXT
            );
            CREATE TABLE IF NOT EXISTS metrics (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )?;
        Ok(Self { conn })
    }

    pub fn put(&self, repo: &RepositoryHealth) -> Result<(), CacheError> {
        let json = serde_json::to_string(repo)?;
        debug_assert!(
            !json.to_ascii_lowercase().contains("gho_")
                && !json.to_ascii_lowercase().contains("glpat-")
                && !json.contains("oit_"),
            "cache must never persist tokens"
        );
        self.conn.execute(
            "INSERT OR REPLACE INTO repos (id, path, health_json, local_updated_at, remote_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                repo.identity.id,
                repo.identity.path,
                json,
                repo.freshness.local_updated_at.to_rfc3339(),
                repo.freshness
                    .remote_updated_at
                    .map(|t| t.to_rfc3339())
            ],
        )?;
        Ok(())
    }

    pub fn put_inventory(&self, inventory: &Inventory) -> Result<(), CacheError> {
        for repo in &inventory.repositories {
            self.put(repo)?;
        }
        Ok(())
    }

    pub fn load_inventory(
        &self,
        thresholds: &HealthThresholds,
        now: DateTime<Utc>,
    ) -> Result<Inventory, CacheError> {
        let mut stmt = self.conn.prepare(
            "SELECT health_json, local_updated_at, remote_updated_at FROM repos ORDER BY path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut repositories = Vec::new();
        for row in rows {
            let (json, local_raw, remote_raw) = row?;
            let mut health: RepositoryHealth = serde_json::from_str(&json)?;
            let local = DateTime::parse_from_rfc3339(&local_raw)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(health.freshness.local_updated_at);
            let remote = remote_raw.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            });
            health.freshness = present_freshness(local, remote, thresholds, now);
            repositories.push(health);
        }
        Ok(Inventory::new(repositories, vec![], now))
    }

    pub fn set_metric(&self, key: &str, value: &str) -> Result<(), CacheError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metrics (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn metric(&self, key: &str) -> Result<Option<String>, CacheError> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM metrics WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn current_percentage(
        &self,
        thresholds: &HealthThresholds,
        now: DateTime<Utc>,
    ) -> Result<f64, CacheError> {
        let inv = self.load_inventory(thresholds, now)?;
        if inv.repositories.is_empty() {
            return Ok(0.0);
        }
        let current = inv
            .repositories
            .iter()
            .filter(|r| r.freshness.presented_as == FreshnessPresentedAs::Current)
            .count();
        Ok((current as f64 / inv.repositories.len() as f64) * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gfc_schema::*;
    use tempfile::TempDir;

    fn sample(now: DateTime<Utc>) -> RepositoryHealth {
        RepositoryHealth {
            schema_version: SCHEMA_VERSION.into(),
            identity: Identity {
                id: "id1".into(),
                name: "demo".into(),
                path: "/tmp/demo".into(),
                provider: ProviderKind::Git,
                remote_url: None,
                group: None,
                tags: vec![],
                workspaces: vec![],
            },
            local: LocalHealth {
                worktree: WorktreeSignal {
                    state: WorktreeState::Clean,
                    evidence: WorktreeEvidence {
                        modified: 0,
                        staged: 0,
                        untracked: 0,
                        conflicted_paths: vec![],
                    },
                    updated_at: now,
                },
                divergence: DivergenceSignal {
                    state: DivergenceState::Synced,
                    evidence: DivergenceEvidence {
                        branch: "main".into(),
                        upstream: None,
                        ahead: 0,
                        behind: 0,
                    },
                    updated_at: now,
                },
                fetch_age: FetchAgeSignal {
                    last_fetch_at: Some(now),
                    age_secs: Some(0),
                    updated_at: now,
                },
            },
            remote: unknown_remote(now),
            staleness: StalenessSignal {
                state: StalenessState::Fresh,
                evidence: StalenessEvidence {
                    fetch_age_secs: Some(0),
                    unfinished_work: false,
                    last_activity_at: Some(now),
                    reasons: vec![],
                },
                updated_at: now,
            },
            freshness: present_freshness(now, Some(now), &HealthThresholds::default(), now),
            error: None,
        }
    }

    #[test]
    fn stale_rows_are_never_current() {
        let tmp = TempDir::new().unwrap();
        let cache = Cache::open(&tmp.path().join("c.sqlite")).unwrap();
        let old = Utc::now() - chrono::Duration::hours(2);
        let mut repo = sample(old);
        repo.freshness = present_freshness(old, Some(old), &HealthThresholds::default(), old);
        cache.put(&repo).unwrap();
        let inv = cache
            .load_inventory(&HealthThresholds::default(), Utc::now())
            .unwrap();
        assert_eq!(
            inv.repositories[0].freshness.presented_as,
            FreshnessPresentedAs::Cached
        );
    }
}
