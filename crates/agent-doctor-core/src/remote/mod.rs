//! Agentless remote project / runtime health over SSH.

mod bootstrap;
mod doctor;
mod registry;
mod ssh;

pub use bootstrap::{
    bootstrap_and_add_host, build_install_pubkey_remote_script, generate_ed25519_key,
    install_pubkey_with_password, verify_key_login, BootstrapHostOptions,
};
pub use doctor::{
    probe_remote_host, run_remote_doctor, run_remote_doctor_with_backend,
    write_remote_doctor_report, RemoteDoctorOptions, RemoteDoctorReport, RemoteHostProbeReport,
    RemoteRuntimeDoctorResult,
};
pub use registry::{
    add_host, add_project, list_hosts, list_projects, load_remote_hosts, remote_hosts_path,
    remote_key_path_for, remote_keys_dir, remote_root_dir, remove_host, remove_project,
    save_remote_hosts, upsert_managed_host, RemoteHostEntry, RemoteHostsDocument,
    RemoteProjectEntry,
};
pub use ssh::{SshBackend, SshTarget};
