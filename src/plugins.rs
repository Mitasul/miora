//! Versioned channel-service plugins. Management authentication belongs to admin.
pub(crate) mod api;
pub(crate) mod builtin;
mod interceptors;
mod lease;
mod manifest;
pub(crate) mod registry;
mod runtime;
mod upload;

pub(crate) use lease::PluginLease;
pub(crate) use registry::PluginRegistry;
pub(crate) use runtime::channel_plugin_id;

#[cfg(test)]
mod tests;
