//! Loom CRUD and discovery use cases.
//!
//! Covers loom registration, unregistration, discovery, and config reload.
//! Shares the `ensure_strand_dir_and_watch` helper for file watcher setup.

mod discover;
mod mod_watchers;
mod register;
mod reload;
mod unregister;

pub use discover::DiscoverLooms;
pub use register::RegisterLoom;
pub use reload::ReloadConfig;
pub use unregister::UnregisterLoom;
pub(crate) use mod_watchers::ensure_strand_dir_and_watch;
