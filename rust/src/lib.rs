extern crate core;

pub mod base;
pub mod block;
pub mod canonicity;
pub mod chain;
pub mod cli;
pub mod client;
pub mod command;
pub mod constants;
pub mod event;
pub mod ledger;
pub mod logging;
pub mod metrics;
pub mod mina_blocks;
pub mod proof_systems;
pub mod protocol;
pub mod server;
pub mod snark_work;
pub mod state;
pub mod store;
pub mod unix_socket_server;
pub mod utility;
pub mod web;

#[cfg(target_family = "unix")]
pub mod platform {
    use libc::{kill, pid_t};

    pub fn is_process_running(pid: pid_t) -> bool {
        // kill(pid, 0) sends signal 0 to the process, which is a no-op check
        // If the process exists, kill() returns 0, otherwise it returns -1
        unsafe { kill(pid, 0) == 0 }
    }

    /// Whether the process holding `pid` is actually a `mina-indexer`, not just
    /// any live process. Liveness alone is not enough to call a PID lock
    /// "held": PIDs get reused — most acutely PID 1 inside a container,
    /// which always belongs to the (re)started init, and any recycled PID
    /// can be inherited by an unrelated process. We confirm identity via
    /// Linux `/proc/<pid>/comm` (the executable name). When `/proc` is
    /// unavailable (e.g. macOS) we can't introspect, so fall back to
    /// "assume it is" and let the liveness check decide — preserving the
    /// prior behaviour there.
    pub fn is_indexer_process(pid: pid_t) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            Ok(comm) => comm.trim().contains("mina-indexer"),
            Err(_) if std::path::Path::new("/proc").exists() => false,
            Err(_) => true,
        }
    }
}
