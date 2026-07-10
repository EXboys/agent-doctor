mod client;
mod config;
mod policy;
mod sync;

pub use client::{check_evotown_connectivity, EvotownClient, EvotownHealthReport};
pub use config::{
    default_policy_cache_path, default_skills_dir, default_skills_lock_path, evotown_config_dir,
    evotown_status, load_evotown_config, validate_evotown_api_key, EvotownConfig, EvotownStatus,
    DEFAULT_BUNDLE_ID, DEFAULT_RUNTIME_TARGET,
};
pub use policy::{execute_policy_pull, PolicyPullReport};
pub use sync::{execute_sync, SkillSyncOutcome, SyncOptions, SyncReport};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::setup::{
    evotown_base_from_gateway, execute_setup, gateway_url_from_evotown_base,
    write_evotown_agent_env, SetupOptions, SetupReport, DEFAULT_EVOTOWN_RUNTIME,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingOptions {
    pub url: String,
    pub api_key: String,
    pub hermes_provider: String,
    pub sync_skills: bool,
    pub pull_policies: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingReport {
    pub setup: SetupReport,
    pub health: Option<EvotownHealthReport>,
    pub sync: Option<SyncReport>,
    pub policy: Option<PolicyPullReport>,
    pub evotown_agent_env_path: String,
}

/// Connect this machine to Evotown: setup profile + optional sync/policy + doctor-ready state.
pub fn execute_evotown_onboarding(options: &OnboardingOptions) -> Result<OnboardingReport> {
    let gateway_url = gateway_url_from_evotown_base(&options.url);
    let evotown_base = evotown_base_from_gateway(&gateway_url);
    validate_evotown_api_key(&options.api_key)?;

    let setup = execute_setup(&SetupOptions {
        gateway_url: gateway_url.clone(),
        api_key: options.api_key.clone(),
        hermes_provider: options.hermes_provider.clone(),
    })?;

    let agent_env_path =
        write_evotown_agent_env(&evotown_base, &options.api_key, DEFAULT_EVOTOWN_RUNTIME)?;

    let config = load_evotown_config()?;
    let client = EvotownClient::new(&config.base_url, &config.api_key)?;
    let health = check_evotown_connectivity(&client).ok();

    let sync = if options.sync_skills {
        Some(execute_sync(
            &config,
            &SyncOptions {
                dry_run: false,
                only_skills: Vec::new(),
                runtime_target: None,
                bundle_id: None,
            },
        )?)
    } else {
        None
    };

    let policy = if options.pull_policies {
        Some(execute_policy_pull(&config)?)
    } else {
        None
    };

    Ok(OnboardingReport {
        setup,
        health,
        sync,
        policy,
        evotown_agent_env_path: agent_env_path.display().to_string(),
    })
}
