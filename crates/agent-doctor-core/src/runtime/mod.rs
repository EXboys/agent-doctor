mod bash_allowlist;
mod registry;

pub use bash_allowlist::{bash_command_allowed_for_runtime, runtime_allowed_bash_commands};

pub(crate) use registry::{
    ask_backend, ask_runtime_ids, bind_workspace_runtimes, dispatch_job, dispatch_runtime_ids,
    open_session, skill_mount_runtime_ids as descriptor_skill_mount_runtime_ids, ConfigFormat,
    WorkspaceBindInput,
};

pub use registry::{
    adapter_by_id, all_adapters, all_runtime_ids, apply_runtime_playbook,
    apply_runtime_playbook_filtered, descriptor_by_id, run_runtime_lifecycle, runtime_catalog,
    runtime_supports_lifecycle, runtime_supports_playbook, suggest_runtime_repairs,
    RuntimeCatalogEntry, RuntimeDescriptor, RuntimeLifecycleAction, RuntimeProbeSpec,
};
