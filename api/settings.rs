use std::collections::HashMap;

use super::PluginId;

/// Stable, plugin-local name of a persisted setting.
///
/// Labels are intentionally not used as keys: a label may change or be
/// localized without losing the user's stored value.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SettingKey(&'static str);

impl SettingKey {
    #[must_use]
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// A persistent boolean setting declared by a plugin.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BoolSetting {
    key: SettingKey,
    default: bool,
}

impl BoolSetting {
    #[must_use]
    pub const fn new(key: &'static str, default: bool) -> Self {
        Self {
            key: SettingKey::new(key),
            default,
        }
    }
}

/// A persistent floating-point setting declared by a plugin.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FloatSetting {
    key: SettingKey,
    default: f32,
}

impl FloatSetting {
    #[must_use]
    pub const fn new(key: &'static str, default: f32) -> Self {
        Self {
            key: SettingKey::new(key),
            default,
        }
    }
}

/// Typed setting declaration usable with [`PluginSettings::get`].
pub trait Setting: Copy {
    type Value: Copy;

    fn key(self) -> SettingKey;
    fn get(self, store: &SettingsStore, plugin: PluginId) -> Self::Value;
    fn ensure(self, store: &mut SettingsStore, plugin: PluginId);
    fn set(self, store: &mut SettingsStore, plugin: PluginId, value: Self::Value);
}

impl Setting for BoolSetting {
    type Value = bool;

    fn key(self) -> SettingKey {
        self.key
    }

    fn get(self, store: &SettingsStore, plugin: PluginId) -> Self::Value {
        store.bool_value(plugin, self.key).unwrap_or(self.default)
    }

    fn ensure(self, store: &mut SettingsStore, plugin: PluginId) {
        store
            .bool_values
            .entry(ScopedSettingKey::new(plugin, self.key))
            .or_insert(self.default);
    }

    fn set(self, store: &mut SettingsStore, plugin: PluginId, value: Self::Value) {
        store.set_bool(plugin, self, value);
    }
}

impl Setting for FloatSetting {
    type Value = f32;

    fn key(self) -> SettingKey {
        self.key
    }

    fn get(self, store: &SettingsStore, plugin: PluginId) -> Self::Value {
        store.float_value(plugin, self.key).unwrap_or(self.default)
    }

    fn ensure(self, store: &mut SettingsStore, plugin: PluginId) {
        store
            .float_values
            .entry(ScopedSettingKey::new(plugin, self.key))
            .or_insert(self.default);
    }

    fn set(self, store: &mut SettingsStore, plugin: PluginId, value: Self::Value) {
        store.set_float(plugin, self, value);
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct ScopedSettingKey {
    plugin: PluginId,
    key: SettingKey,
}

impl ScopedSettingKey {
    const fn new(plugin: PluginId, key: SettingKey) -> Self {
        Self { plugin, key }
    }
}

/// Host-owned storage for declared plugin settings.
///
/// The current store is in-memory. Persistence will be implemented by a later
/// host adapter; plugin keys are already stable for that transition.
#[derive(Default)]
pub struct SettingsStore {
    bool_values: HashMap<ScopedSettingKey, bool>,
    float_values: HashMap<ScopedSettingKey, f32>,
}

impl SettingsStore {
    pub(crate) fn set_bool(&mut self, plugin: PluginId, setting: BoolSetting, value: bool) {
        self.bool_values
            .insert(ScopedSettingKey::new(plugin, setting.key), value);
    }

    pub(crate) fn set_float(&mut self, plugin: PluginId, setting: FloatSetting, value: f32) {
        self.float_values
            .insert(ScopedSettingKey::new(plugin, setting.key), value);
    }

    fn bool_value(&self, plugin: PluginId, key: SettingKey) -> Option<bool> {
        self.bool_values
            .get(&ScopedSettingKey::new(plugin, key))
            .copied()
    }

    fn float_value(&self, plugin: PluginId, key: SettingKey) -> Option<f32> {
        self.float_values
            .get(&ScopedSettingKey::new(plugin, key))
            .copied()
    }
}

/// Read-only settings capability scoped to one plugin identity.
pub struct PluginSettings<'store> {
    plugin: PluginId,
    store: &'store SettingsStore,
}

impl<'store> PluginSettings<'store> {
    pub(crate) const fn new(plugin: PluginId, store: &'store SettingsStore) -> Self {
        Self { plugin, store }
    }

    /// Reads a typed setting, using its declared default until the host has a
    /// stored value for this plugin/key pair.
    #[must_use]
    pub fn get<S: Setting>(&self, setting: S) -> S::Value {
        setting.get(self.store, self.plugin)
    }
}
