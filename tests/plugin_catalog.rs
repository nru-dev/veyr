use veyr::engine::{DeveloperApi, DeveloperPlugin, Engine, PluginId, PluginResult};
use veyr::offsets::{api::Memory, RemoteAddress};
use veyr::plugins::{CatalogInstallError, CatalogRegistrationError, DeveloperPluginCatalog};

struct NoMemory;

impl Memory for NoMemory {
    type Error = ();

    fn read_u8(&self, _address: RemoteAddress) -> Result<u8, Self::Error> {
        Err(())
    }

    fn read_u32(&self, _address: RemoteAddress) -> Result<u32, Self::Error> {
        Err(())
    }

    fn read_u64(&self, _address: RemoteAddress) -> Result<u64, Self::Error> {
        Err(())
    }

    fn read_f32(&self, _address: RemoteAddress) -> Result<f32, Self::Error> {
        Err(())
    }
}

struct NamedPlugin(PluginId);

impl DeveloperPlugin<NoMemory> for NamedPlugin {
    fn id(&self) -> PluginId {
        self.0
    }

    fn on_load(&mut self, _api: &DeveloperApi<'_, '_, NoMemory>) -> PluginResult<()> {
        Ok(())
    }
}

#[test]
fn catalog_transfers_plugins_without_making_engine_choose_them() {
    let mut catalog = DeveloperPluginCatalog::new();
    catalog
        .add(NamedPlugin(PluginId::new("dev.warrior")))
        .expect("first catalog entry");
    catalog
        .add(NamedPlugin(PluginId::new("dev.priest")))
        .expect("second catalog entry");

    let mut engine = Engine::new(NoMemory);
    catalog
        .install_into(&mut engine)
        .expect("catalog installation");

    assert!(engine.has_plugin(PluginId::new("dev.warrior")));
    assert!(engine.has_plugin(PluginId::new("dev.priest")));
    assert!(engine.initialize().is_clean());
    assert_eq!(engine.active_plugin_count(), 2);
}

#[test]
fn catalog_rejects_duplicates_and_never_partially_installs_on_conflict() {
    let mut catalog = DeveloperPluginCatalog::new();
    catalog
        .add(NamedPlugin(PluginId::new("dev.duplicate")))
        .expect("first catalog entry");
    assert_eq!(
        catalog.add(NamedPlugin(PluginId::new("dev.duplicate"))),
        Err(CatalogRegistrationError::DuplicateInCatalog(PluginId::new(
            "dev.duplicate"
        )))
    );

    let mut conflicting_catalog = DeveloperPluginCatalog::new();
    conflicting_catalog
        .add(NamedPlugin(PluginId::new("dev.existing")))
        .expect("conflicting catalog entry");
    conflicting_catalog
        .add(NamedPlugin(PluginId::new("dev.new")))
        .expect("new catalog entry");

    let mut engine = Engine::new(NoMemory);
    engine
        .register_plugin(NamedPlugin(PluginId::new("dev.existing")))
        .expect("existing plugin");
    assert_eq!(
        conflicting_catalog.install_into(&mut engine),
        Err(CatalogInstallError::EngineConflict(PluginId::new(
            "dev.existing"
        )))
    );
    assert!(!engine.has_plugin(PluginId::new("dev.new")));
}
