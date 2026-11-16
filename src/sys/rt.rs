use std::sync::LazyLock;

use tokio::runtime::{Builder, Runtime};

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("piro-rt")
        .build()
        .expect("failed to start the tokio runtime")
});

/// The process-wide runtime for subprocesses, HTTP and agent tasks; the UI thread never blocks on it.
pub fn runtime() -> &'static Runtime {
    &RUNTIME
}
