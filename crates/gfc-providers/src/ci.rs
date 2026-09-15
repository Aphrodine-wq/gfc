//! CI status normalization shared by GitHub, GitLab, and Origin.

use gfc_schema::CiState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCheck {
    pub name: String,
    /// queued, in_progress, completed, pending, running, ...
    pub status: String,
    /// success, failure, cancelled, skipped, ...
    pub conclusion: Option<String>,
}

pub fn normalize_checks(checks: &[RawCheck]) -> (CiState, Option<RawCheck>) {
    if checks.is_empty() {
        return (CiState::Unknown, None);
    }
    let mut worst = CiState::Passing;
    let mut evidence: Option<RawCheck> = None;
    for check in checks {
        let mapped = map_one(check);
        if rank(mapped) > rank(worst) {
            worst = mapped;
            evidence = Some(check.clone());
        }
    }
    (worst, evidence)
}

fn map_one(check: &RawCheck) -> CiState {
    let conclusion = check
        .conclusion
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let status = check.status.to_ascii_lowercase();
    if matches!(
        conclusion.as_str(),
        "failure" | "failed" | "cancelled" | "canceled" | "timed_out" | "action_required" | "error"
    ) || matches!(
        status.as_str(),
        "failed" | "failure" | "canceled" | "cancelled"
    ) {
        return CiState::Failing;
    }
    if matches!(
        status.as_str(),
        "queued"
            | "in_progress"
            | "pending"
            | "running"
            | "waiting"
            | "requested"
            | "created"
            | "preparing"
            | "waiting_for_resource"
            | "manual"
            | "scheduled"
    ) {
        return CiState::Pending;
    }
    if matches!(
        conclusion.as_str(),
        "success" | "skipped" | "neutral" | "passed"
    ) || matches!(status.as_str(), "success" | "passed" | "skipped")
    {
        return CiState::Passing;
    }
    CiState::Unknown
}

fn rank(state: CiState) -> u8 {
    match state {
        CiState::Unknown => 0,
        CiState::Unsupported => 1,
        CiState::Passing => 2,
        CiState::Pending => 3,
        CiState::Failing => 4,
    }
}

pub fn map_gitlab_pipeline(status: &str) -> CiState {
    map_one(&RawCheck {
        name: "pipeline".into(),
        status: status.into(),
        conclusion: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_wins_over_pending() {
        let checks = vec![
            RawCheck {
                name: "lint".into(),
                status: "completed".into(),
                conclusion: Some("success".into()),
            },
            RawCheck {
                name: "test".into(),
                status: "in_progress".into(),
                conclusion: None,
            },
            RawCheck {
                name: "build".into(),
                status: "completed".into(),
                conclusion: Some("failure".into()),
            },
        ];
        let (state, ev) = normalize_checks(&checks);
        assert_eq!(state, CiState::Failing);
        assert_eq!(ev.unwrap().name, "build");
    }

    #[test]
    fn empty_is_unknown() {
        let (state, _) = normalize_checks(&[]);
        assert_eq!(state, CiState::Unknown);
    }

    #[test]
    fn gitlab_running_is_pending() {
        assert_eq!(map_gitlab_pipeline("running"), CiState::Pending);
        assert_eq!(map_gitlab_pipeline("failed"), CiState::Failing);
        assert_eq!(map_gitlab_pipeline("success"), CiState::Passing);
    }
}
