//! Versioned channel-service plugins. Management authentication belongs to admin.
pub(crate) mod api;
pub(crate) mod builtin;
mod interceptors;
mod lease;
mod managed;
mod manifest;
pub(crate) mod native;
mod native_stream;
pub(crate) mod registry;
mod runtime;
mod upload;

pub(crate) use lease::PluginLease;
pub(crate) use registry::PluginRegistry;
pub(crate) use runtime::channel_plugin_id;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod migration_tests;
