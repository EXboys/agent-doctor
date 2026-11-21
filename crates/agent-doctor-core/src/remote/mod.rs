//! Remote computers over SSH.
//!
//! Two modes share the same host registry:
//! - **Agentless doctor**: read-only checks when the other computer has no client.
//! - **Linked client** (Paseo-style): SSH tunnel to the other computer's loopback
//!   client service for projects, files, doctor, and fix.

mod bootstrap;
mod doctor;
mod lan_link;
mod link_client;
mod link_daemon;
mod link_files;
mod link_protocol;
mod link_tunnel;
mod openssh_config;
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
pub use lan_link::{
    connect_nearby, decide_ask, list_nearby, nearby_sharing, pending_asks, set_nearby_sharing,
    spawn_lan_link, this_computer_name, NearbyAsk, NearbyComputer,
};
pub use link_client::LinkClient;
pub use link_daemon::{
    is_link_daemon_running, probe_local_daemon, register_link_method, spawn_link_daemon,
    LinkDaemonGuard, LinkMethodHandler, LinkMethodOutcome, LinkRpcContext, LinkSocket,
};
pub use link_protocol::{
    LinkDirEntry, LinkDoctorSummary, LinkFilePayload, LinkFixSummary, LinkHello, LinkImageBlob,
    LinkInitResult, LinkPush, LinkResponse, LinkWorkspaceRow, LINK_DAEMON_PORT,
};
pub use link_tunnel::{open_link_tunnel, LinkConnectFailure, LinkTunnel};
pub use registry::{
    add_host, add_project, list_hosts, list_projects, load_remote_hosts, mark_host_linked,
    publish_managed_logins, remote_hosts_path, remote_key_path_for, remote_keys_dir,
    remote_root_dir, remove_host, remove_project, save_remote_hosts, upsert_managed_host,
    RemoteHostEntry, RemoteHostsDocument, RemoteProjectEntry,
};
pub use ssh::{SshBackend, SshTarget};
