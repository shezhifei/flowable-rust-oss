// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

pub fn create_process_engine() -> Arc<ProcessEngine> {
    Arc::new(
        ProcessEngine::new_with_config(
            "default".to_string(),
            ProcessEngineConfiguration::default(),
        )
        .unwrap(),
    )
}
