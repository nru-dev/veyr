/// Stable namespace identity of a first-party developer plugin.
///
/// This will also prefix the plugin's future settings and menu pages, so it
/// must not change after a plugin has been published internally.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct PluginId(&'static str);

impl PluginId {
    #[must_use]
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}
