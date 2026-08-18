use std::collections::HashSet;

use super::{BoolSetting, FloatSetting, PluginId, Setting, SettingKey, SettingsStore};

/// Stable identifier of a menu page owned by a plugin.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct PageId(&'static str);

impl PageId {
    #[must_use]
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Stable identifier of a momentary menu button.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ButtonId(&'static str);

impl ButtonId {
    #[must_use]
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Display range used by a floating-point slider.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FloatRange {
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

impl FloatRange {
    #[must_use]
    pub const fn new(min: f32, max: f32, step: f32) -> Self {
        Self { min, max, step }
    }
}

/// Placement of a page in the host menu hierarchy.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum MenuPlacement {
    /// The future public-SDK surface: a page inside the owning plugin's tab.
    PluginTab,
    /// A privileged top-level page available only to trusted DEV code.
    Root,
}

/// Backend-neutral declaration of one menu element.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuCommand {
    Page {
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        label: &'static str,
    },
    Toggle {
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        setting: SettingKey,
        label: &'static str,
    },
    Slider {
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        setting: SettingKey,
        label: &'static str,
        range: FloatRange,
    },
    Button {
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        button: ButtonId,
        label: &'static str,
    },
}

/// Host-owned collection of declarations for the current plugin-menu frame.
#[derive(Default)]
pub(crate) struct MenuQueue {
    commands: Vec<MenuCommand>,
}

impl MenuQueue {
    pub(crate) fn plugin_frame_for<'settings, 'input>(
        &mut self,
        plugin: PluginId,
        settings: &'settings mut SettingsStore,
        input: &'input mut MenuInput,
    ) -> PluginMenu<'settings, 'input, '_> {
        PluginMenu {
            plugin,
            settings,
            input,
            commands: &mut self.commands,
        }
    }

    pub(crate) fn developer_frame_for<'settings, 'input>(
        &mut self,
        plugin: PluginId,
        settings: &'settings mut SettingsStore,
        input: &'input mut MenuInput,
    ) -> DeveloperMenu<'settings, 'input, '_> {
        DeveloperMenu {
            inner: self.plugin_frame_for(plugin, settings, input),
        }
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.commands.append(&mut other.commands);
    }

    pub(crate) fn clear(&mut self) {
        self.commands.clear();
    }

    pub(crate) fn take(&mut self) -> Vec<MenuCommand> {
        core::mem::take(&mut self.commands)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct ScopedButtonId {
    plugin: PluginId,
    placement: MenuPlacement,
    page: PageId,
    button: ButtonId,
}

/// One-shot interactions supplied by the future menu backend.
#[derive(Default)]
pub(crate) struct MenuInput {
    pressed_buttons: HashSet<ScopedButtonId>,
}

impl MenuInput {
    pub(crate) fn press(
        &mut self,
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        button: ButtonId,
    ) {
        self.pressed_buttons.insert(ScopedButtonId {
            plugin,
            placement,
            page,
            button,
        });
    }

    fn take_button(
        &mut self,
        plugin: PluginId,
        placement: MenuPlacement,
        page: PageId,
        button: ButtonId,
    ) -> bool {
        self.pressed_buttons.remove(&ScopedButtonId {
            plugin,
            placement,
            page,
            button,
        })
    }
}

/// Restricted menu capability scoped to one plugin's tab and one callback.
///
/// This is the future public SDK menu surface. It deliberately has no method
/// for declaring root pages.
pub struct PluginMenu<'settings, 'input, 'queue> {
    plugin: PluginId,
    settings: &'settings mut SettingsStore,
    input: &'input mut MenuInput,
    commands: &'queue mut Vec<MenuCommand>,
}

impl<'settings, 'input, 'queue> PluginMenu<'settings, 'input, 'queue> {
    /// Declares a page inside this plugin's tab.
    pub fn page(
        &mut self,
        page: PageId,
        label: &'static str,
        build: impl FnOnce(&mut MenuPage<'_, 'settings, 'input, 'queue>),
    ) {
        self.page_at(MenuPlacement::PluginTab, page, label, build);
    }

    fn page_at(
        &mut self,
        placement: MenuPlacement,
        page: PageId,
        label: &'static str,
        build: impl FnOnce(&mut MenuPage<'_, 'settings, 'input, 'queue>),
    ) {
        self.commands.push(MenuCommand::Page {
            plugin: self.plugin,
            placement,
            page,
            label,
        });

        let mut page_context = MenuPage {
            menu: self,
            placement,
            page,
        };
        build(&mut page_context);
    }
}

/// Full menu capability passed only to trusted developer plugins.
///
/// A developer plugin can choose between its ordinary isolated tab and a root
/// page. The public SDK adapter will pass [`PluginMenu`] instead, so SDK code
/// cannot opt into the root surface.
pub struct DeveloperMenu<'settings, 'input, 'queue> {
    inner: PluginMenu<'settings, 'input, 'queue>,
}

impl<'settings, 'input, 'queue> DeveloperMenu<'settings, 'input, 'queue> {
    /// Declares a page within this plugin's ordinary tab.
    pub fn plugin_page(
        &mut self,
        page: PageId,
        label: &'static str,
        build: impl FnOnce(&mut MenuPage<'_, 'settings, 'input, 'queue>),
    ) {
        self.inner.page(page, label, build);
    }

    /// Declares a privileged root-level page owned by this DEV plugin.
    pub fn root_page(
        &mut self,
        page: PageId,
        label: &'static str,
        build: impl FnOnce(&mut MenuPage<'_, 'settings, 'input, 'queue>),
    ) {
        self.inner.page_at(MenuPlacement::Root, page, label, build);
    }
}

/// Controls available within one declared plugin-menu page.
pub struct MenuPage<'page, 'settings, 'input, 'queue> {
    menu: &'page mut PluginMenu<'settings, 'input, 'queue>,
    placement: MenuPlacement,
    page: PageId,
}

impl<'page, 'settings, 'input, 'queue> MenuPage<'page, 'settings, 'input, 'queue> {
    /// Updates a value in this plugin's own settings namespace.
    pub fn set<S: Setting>(&mut self, setting: S, value: S::Value) {
        setting.set(self.menu.settings, self.menu.plugin, value);
    }

    /// Declares a persistent boolean toggle.
    pub fn toggle(&mut self, setting: BoolSetting, label: &'static str) {
        setting.ensure(self.menu.settings, self.menu.plugin);
        self.menu.commands.push(MenuCommand::Toggle {
            plugin: self.menu.plugin,
            placement: self.placement,
            page: self.page,
            setting: setting.key(),
            label,
        });
    }

    /// Declares a persistent float slider.
    pub fn slider(&mut self, setting: FloatSetting, label: &'static str, range: FloatRange) {
        setting.ensure(self.menu.settings, self.menu.plugin);
        self.menu.commands.push(MenuCommand::Slider {
            plugin: self.menu.plugin,
            placement: self.placement,
            page: self.page,
            setting: setting.key(),
            label,
            range,
        });
    }

    /// Declares a momentary button and returns whether the host clicked it in
    /// this menu frame.
    pub fn button(&mut self, button: ButtonId, label: &'static str) -> bool {
        self.menu.commands.push(MenuCommand::Button {
            plugin: self.menu.plugin,
            placement: self.placement,
            page: self.page,
            button,
            label,
        });
        self.menu
            .input
            .take_button(self.menu.plugin, self.placement, self.page, button)
    }
}
