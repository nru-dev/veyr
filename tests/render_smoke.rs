use veyr::engine::Engine;
use veyr::offsets::{
    api::{Color, Memory, RenderCommand, ScreenPoint, Stroke},
    RemoteAddress,
};
use veyr::plugins::RenderSmokePlugin;

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

#[test]
fn visual_smoke_is_a_plugin_scoped_hud_marker_without_game_reads() {
    let mut engine = Engine::new(NoMemory);
    engine
        .register_plugin(RenderSmokePlugin)
        .expect("unique visual smoke plugin");

    assert!(engine.render().is_clean());
    let commands = engine.take_render_commands();

    assert_eq!(commands.len(), 3);
    assert!(commands
        .iter()
        .all(|command| command.plugin == RenderSmokePlugin::ID));
    assert_eq!(
        commands[0].command,
        RenderCommand::HudCircle {
            center: ScreenPoint::new(120.0, 120.0),
            radius: 48.0,
            stroke: Stroke::new(Color::CYAN, 3.0),
        }
    );
}
