pub mod apply;
pub mod configure;
pub mod deploy;
pub mod diag;
pub mod enroll;
pub mod init;
pub mod install;
pub mod invite;
pub mod job;
pub mod p2p;
pub mod package;
pub mod policy;
pub mod push;
pub mod status;
pub mod storage;
pub mod upgrade;
pub mod util;
pub mod whoami;

pub use apply::apply;
pub use configure::configure;
pub use deploy::deploy_component;
pub use diag::diag_quic;
pub use enroll::enroll;
pub use init::init;
pub use init::key_show;
pub use install::install;
#[cfg(unix)]
pub use install::install_cli;
pub use invite::invite;
pub use job::{
    cancel_job, job_artifacts, job_artifacts_json, job_download, job_logs, job_status,
    job_status_json, list_jobs, list_jobs_json, net_list_jobs_json, net_status_job_json,
    submit_job, submit_job_from_spec,
};
pub use p2p::watch;
pub use package::package_create;
pub use policy::{policy_set, policy_show};
pub use push::push;
pub use push::push_package;
pub use status::status;
pub use storage::{storage_gc, storage_ls, storage_pin};
pub use upgrade::{upgrade, upgrade_multi};
pub use whoami::whoami;
