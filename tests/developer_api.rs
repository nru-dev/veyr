use std::collections::HashMap;

use veyr::offsets::{
    api::{GameApi, Memory},
    memory, update_fields, ObjectFields, RemoteAddress, UnitFields,
};

#[derive(Default)]
struct FakeMemory {
    values: HashMap<RemoteAddress, u64>,
}

impl FakeMemory {
    fn insert_u8(&mut self, address: RemoteAddress, value: u8) {
        self.values.insert(address, u64::from(value));
    }

    fn insert_u32(&mut self, address: RemoteAddress, value: u32) {
        self.values.insert(address, u64::from(value));
    }

    fn insert_u64(&mut self, address: RemoteAddress, value: u64) {
        self.values.insert(address, value);
    }

    fn insert_f32(&mut self, address: RemoteAddress, value: f32) {
        self.insert_u32(address, value.to_bits());
    }
}

impl Memory for FakeMemory {
    type Error = RemoteAddress;

    fn read_u8(&self, address: RemoteAddress) -> Result<u8, Self::Error> {
        self.values
            .get(&address)
            .copied()
            .map(|value| value as u8)
            .ok_or(address)
    }

    fn read_u32(&self, address: RemoteAddress) -> Result<u32, Self::Error> {
        self.values
            .get(&address)
            .copied()
            .map(|value| value as u32)
            .ok_or(address)
    }

    fn read_u64(&self, address: RemoteAddress) -> Result<u64, Self::Error> {
        self.values.get(&address).copied().ok_or(address)
    }

    fn read_f32(&self, address: RemoteAddress) -> Result<f32, Self::Error> {
        self.values
            .get(&address)
            .copied()
            .map(|value| f32::from_bits(value as u32))
            .ok_or(address)
    }
}

#[test]
fn developer_api_reads_the_local_players_health_through_the_descriptor() {
    const CONNECTION: RemoteAddress = 0x1000;
    const OBJECT_MANAGER: RemoteAddress = 0x2000;
    const PLAYER: RemoteAddress = 0x3000;
    const DESCRIPTOR: RemoteAddress = 0x4000;
    const GAME_OBJECT: RemoteAddress = 0x5000;
    const GAME_OBJECT_DESCRIPTOR: RemoteAddress = 0x6000;
    const CAST: RemoteAddress = 0x7000;
    const COOLDOWN: RemoteAddress = 0x8000;
    const CAMERA_BASE: RemoteAddress = 0x9000;
    const CAMERA_WORLD_FRAME: RemoteAddress = 0xA000;
    const LOCAL_GUID: u64 = 0xAABB_CCDD_EEFF_0011;
    const GAME_OBJECT_GUID: u64 = 0x1122_3344_5566_7788;

    let mut memory_backend = FakeMemory::default();
    memory_backend.insert_u32(memory::object_manager::CLIENT_CONNECTION, CONNECTION);
    memory_backend.insert_u32(memory::game_state::IS_INGAME, 1);
    memory_backend.insert_u32(
        CONNECTION + memory::object_manager::OBJECT_MANAGER,
        OBJECT_MANAGER,
    );
    memory_backend.insert_u64(
        OBJECT_MANAGER + memory::object_manager::LOCAL_GUID,
        LOCAL_GUID,
    );
    memory_backend.insert_u32(
        OBJECT_MANAGER + memory::object_manager::FIRST_OBJECT,
        PLAYER,
    );
    memory_backend.insert_u32(PLAYER + memory::object::TYPE, 4);
    memory_backend.insert_u32(PLAYER + memory::object::DESCRIPTOR_ARRAY, DESCRIPTOR);
    memory_backend.insert_u32(PLAYER + memory::object::NEXT_OBJECT, GAME_OBJECT);
    memory_backend.insert_u64(
        update_fields::address_of(DESCRIPTOR, ObjectFields::Guid),
        LOCAL_GUID,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, UnitFields::Health),
        1_234,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, UnitFields::MaxHealth),
        4_321,
    );
    memory_backend.insert_u64(
        update_fields::address_of(DESCRIPTOR, UnitFields::Target),
        GAME_OBJECT_GUID,
    );
    memory_backend.insert_u32(update_fields::address_of(DESCRIPTOR, UnitFields::Level), 80);
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, UnitFields::FactionTemplate),
        35,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, UnitFields::DisplayId),
        123,
    );
    memory_backend.insert_f32(
        update_fields::address_of(DESCRIPTOR, UnitFields::CombatReach),
        1.5,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, ObjectFields::Entry),
        42,
    );
    memory_backend.insert_f32(
        update_fields::address_of(DESCRIPTOR, ObjectFields::ScaleX),
        1.25,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, veyr::offsets::PlayerFields::GuildId),
        999,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, veyr::offsets::PlayerFields::GuildRank),
        2,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, veyr::offsets::PlayerFields::ArenaCurrency),
        15,
    );
    memory_backend.insert_u32(
        update_fields::address_of(DESCRIPTOR, veyr::offsets::PlayerFields::HonorCurrency),
        25,
    );
    memory_backend.insert_f32(PLAYER + memory::unit::POSITION_X, 10.0);
    memory_backend.insert_f32(PLAYER + memory::unit::POSITION_Y, 20.0);
    memory_backend.insert_f32(PLAYER + memory::unit::POSITION_Z, 30.0);
    memory_backend.insert_f32(PLAYER + memory::unit::ROTATION, 2.0);

    memory_backend.insert_u32(
        PLAYER + veyr::offsets::advanced_combat::auras::BASE_AURA_COUNT,
        1,
    );
    let aura = PLAYER + veyr::offsets::advanced_combat::auras::BASE_AURA_ARRAY;
    memory_backend.insert_u64(aura, LOCAL_GUID);
    memory_backend.insert_u32(aura + 0x08, 12345);
    memory_backend.insert_u8(aura + 0x0C, 3);
    memory_backend.insert_u8(aura + 0x0D, 80);
    memory_backend.insert_u8(aura + 0x0E, 2);
    memory_backend.insert_u32(aura + 0x10, 6_000);
    memory_backend.insert_u32(aura + 0x14, 9_000);

    memory_backend.insert_u32(
        PLAYER + veyr::offsets::advanced_combat::casting::CURRENT_SPELL_ID,
        1337,
    );
    memory_backend.insert_u32(
        PLAYER + veyr::offsets::advanced_combat::casting::SPELL_CAST_STRUCT_PTR,
        CAST,
    );
    memory_backend.insert_u32(CAST + 0x04, 1337);
    memory_backend.insert_u32(CAST + 0x08, 1_000);
    memory_backend.insert_u32(CAST + 0x0C, 2_000);
    memory_backend.insert_u8(CAST + 0x10, 1);

    memory_backend.insert_u32(GAME_OBJECT + memory::object::TYPE, 5);
    memory_backend.insert_u32(
        GAME_OBJECT + memory::object::DESCRIPTOR_ARRAY,
        GAME_OBJECT_DESCRIPTOR,
    );
    memory_backend.insert_u32(GAME_OBJECT + memory::object::NEXT_OBJECT, 0);
    memory_backend.insert_u64(
        update_fields::address_of(GAME_OBJECT_DESCRIPTOR, ObjectFields::Guid),
        GAME_OBJECT_GUID,
    );
    memory_backend.insert_u32(
        update_fields::address_of(GAME_OBJECT_DESCRIPTOR, ObjectFields::Entry),
        777,
    );
    memory_backend.insert_u32(
        update_fields::address_of(
            GAME_OBJECT_DESCRIPTOR,
            veyr::offsets::GameObjectFields::DisplayId,
        ),
        888,
    );
    memory_backend.insert_u32(
        update_fields::address_of(
            GAME_OBJECT_DESCRIPTOR,
            veyr::offsets::GameObjectFields::Faction,
        ),
        999,
    );
    memory_backend.insert_u8(
        GAME_OBJECT + veyr::offsets::advanced_combat::game_objects::GAMEOBJECT_STATE,
        1,
    );

    memory_backend.insert_u8(veyr::offsets::advanced_combat::state::COMBO_POINTS, 4);
    memory_backend.insert_u64(
        veyr::offsets::advanced_combat::state::MOUSEOVER_GUID,
        GAME_OBJECT_GUID,
    );
    memory_backend.insert_u64(
        veyr::offsets::advanced_combat::state::FOCUS_GUID,
        LOCAL_GUID,
    );
    memory_backend.insert_u32(veyr::offsets::advanced_combat::state::IS_AUTO_ATTACKING, 1);
    memory_backend.insert_u32(
        veyr::offsets::advanced_combat::cooldown::SPELL_COOLDOWN_PTR,
        COOLDOWN,
    );
    memory_backend.insert_u32(COOLDOWN, 0);
    memory_backend.insert_u32(COOLDOWN + 0x08, 1337);
    memory_backend.insert_u32(COOLDOWN + 0x0C, 0);
    memory_backend.insert_u32(COOLDOWN + 0x10, 2_000);
    memory_backend.insert_u32(COOLDOWN + 0x14, 1_500);

    let camera_address = CAMERA_BASE;
    memory_backend.insert_u32(
        veyr::offsets::advanced_combat::camera::CURRENT_WORLD_FRAME,
        CAMERA_WORLD_FRAME,
    );
    memory_backend.insert_u32(
        CAMERA_WORLD_FRAME + veyr::offsets::advanced_combat::camera::CAMERA_OFFSET,
        CAMERA_BASE,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::EYE_POSITION_OFFSET,
        1.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::EYE_POSITION_OFFSET + 4,
        2.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::EYE_POSITION_OFFSET + 8,
        3.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::FORWARD_BASIS_OFFSET,
        1.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::FORWARD_BASIS_OFFSET + 4,
        0.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::FORWARD_BASIS_OFFSET + 8,
        0.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::LEFT_BASIS_OFFSET,
        0.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::LEFT_BASIS_OFFSET + 4,
        1.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::LEFT_BASIS_OFFSET + 8,
        0.0,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::ROLL_OFFSET,
        0.1,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::YAW_OFFSET,
        0.2,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::PITCH_OFFSET,
        0.3,
    );
    memory_backend.insert_f32(
        camera_address + veyr::offsets::advanced_combat::camera::FOV_OFFSET,
        1.2,
    );

    let api = GameApi::new(memory_backend);
    assert!(api.world().is_in_game().expect("in-game state"));
    assert_eq!(
        api.camera().state().expect("camera state"),
        veyr::offsets::api::CameraState {
            eye: veyr::offsets::api::Vector3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            forward: veyr::offsets::api::Vector3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            left: veyr::offsets::api::Vector3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            roll: 0.1,
            yaw: 0.2,
            pitch: 0.3,
            field_of_view: 1.2,
        }
    );

    let manager = api.world().object_manager().expect("object manager");
    let player = manager
        .local_player_with_limit(32)
        .expect("object traversal")
        .expect("local player");

    assert_eq!(player.guid().expect("player GUID"), LOCAL_GUID);
    assert_eq!(player.health().expect("health"), 1_234);
    assert_eq!(player.max_health().expect("maximum health"), 4_321);
    assert_eq!(
        player.health_ratio().expect("health ratio"),
        Some(1_234.0 / 4_321.0)
    );
    assert!(player.is_alive().expect("alive state"));
    assert_eq!(
        player.target_guid().expect("target GUID"),
        Some(GAME_OBJECT_GUID)
    );
    assert_eq!(player.level().expect("level"), 80);
    assert_eq!(player.guild_id().expect("guild ID"), 999);
    assert_eq!(player.guild_rank().expect("guild rank"), 2);
    assert_eq!(player.arena_currency().expect("arena currency"), 15);
    assert_eq!(player.honor_currency().expect("honor currency"), 25);
    assert_eq!(player.unit().faction_template_id().expect("faction"), 35);
    assert_eq!(player.unit().display_id().expect("display ID"), 123);
    assert_eq!(player.unit().combat_reach().expect("combat reach"), 1.5);
    assert_eq!(player.unit().position().expect("position").x, 10.0);
    assert_eq!(player.unit().aura_count().expect("aura count"), 1);

    let aura = player.unit().aura(0).expect("aura read").expect("aura");
    assert_eq!(aura.creator_guid, LOCAL_GUID);
    assert_eq!(aura.spell_id, 12345);
    assert_eq!(aura.stack_count, 2);
    assert_eq!(aura.duration_ms, 6_000);
    assert_eq!(player.unit().aura(1).expect("aura range"), None);

    let addresses = manager
        .objects()
        .expect("object list")
        .map(|object| object.expect("valid object").address())
        .collect::<Vec<_>>();
    assert_eq!(addresses, vec![PLAYER, GAME_OBJECT]);

    let game_object = manager
        .object_by_guid(GAME_OBJECT_GUID)
        .expect("game object lookup")
        .expect("game object")
        .into_game_object()
        .expect("game-object conversion");
    assert_eq!(game_object.entry_id().expect("entry ID"), 777);
    assert_eq!(game_object.display_id().expect("display ID"), 888);
    assert_eq!(game_object.faction_id().expect("faction ID"), 999);
    assert_eq!(game_object.state().expect("state"), 1);

    let combat = api.combat();
    assert_eq!(combat.combo_points().expect("combo points"), 4);
    assert_eq!(
        combat.mouseover_guid().expect("mouseover GUID"),
        Some(GAME_OBJECT_GUID)
    );
    assert_eq!(combat.focus_guid().expect("focus GUID"), Some(LOCAL_GUID));
    assert!(combat.is_auto_attacking().expect("auto attack"));
    assert_eq!(
        combat.current_spell_id(player.unit()).expect("spell ID"),
        Some(1337)
    );
    assert_eq!(
        combat.current_cast(player.unit()).expect("cast"),
        Some(veyr::offsets::api::CastInfo {
            spell_id: 1337,
            start_time: 1_000,
            end_time: 2_000,
            is_channeling: true,
        })
    );
    assert_eq!(
        combat
            .mouseover()
            .expect("mouseover object")
            .expect("mouseover object")
            .guid()
            .expect("mouseover GUID"),
        GAME_OBJECT_GUID
    );
    assert_eq!(
        combat
            .cooldowns()
            .expect("cooldown list")
            .collect::<Result<Vec<_>, _>>()
            .expect("cooldown entry"),
        vec![veyr::offsets::api::SpellCooldown {
            spell_id: 1337,
            item_id: 0,
            start_time_ms: 2_000,
            duration_ms: 1_500,
        }]
    );
    assert_eq!(
        combat.spell_cooldown(1337).expect("spell cooldown"),
        Some(veyr::offsets::api::SpellCooldown {
            spell_id: 1337,
            item_id: 0,
            start_time_ms: 2_000,
            duration_ms: 1_500,
        })
    );
}
