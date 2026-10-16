//! Shared mutex serializing SQL imports and local database backup restores.

use std::sync::OnceLock;

pub(crate) fn sync_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}
