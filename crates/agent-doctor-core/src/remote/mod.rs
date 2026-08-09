//! Agentless remote project / runtime health over SSH.

mod doctor;
mod registry;
mod ssh;

pub use doctor::{
    run_remote_doctor, run_remote_doctor_with_backend, write_remote_doctor_report,
    RemoteDoctorOptions, RemoteDoctorReport, RemoteRuntimeDoctorResult,
};
pub use registry::{
    add_host, add_project, list_hosts, list_projects, load_remote_hosts, remote_hosts_path,
    remote_root_dir, remove_host, remove_project, save_remote_hosts, RemoteHostEntry,
    RemoteHostsDocument, RemoteProjectEntry,
};
pub use ssh::SshBackend;
