use super::super::{
    advanced_combat, memory, GameObjectFields, ObjectFields, ObjectType, PlayerFields,
    RemoteAddress, UnitFields,
};
use super::memory::with_offset;
use super::{ApiError, ApiResult, Descriptor, GameApi, Memory};

/// A handle to any world object in the game process.
pub struct Object<'api, M> {
    pub(crate) api: &'api GameApi<M>,
    address: RemoteAddress,
}

impl<'api, M> Object<'api, M> {
    pub(crate) const fn new(api: &'api GameApi<M>, address: RemoteAddress) -> Self {
        Self { api, address }
    }

    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.address
    }
}

impl<'api, M: Memory> Object<'api, M> {
    pub fn object_type(&self) -> ApiResult<ObjectType, M::Error> {
        let value = self
            .api
            .memory()
            .read_u32(self.offset_address(memory::object::TYPE)?)
            .map_err(ApiError::Memory)?;

        match value {
            0 => Ok(ObjectType::Object),
            1 => Ok(ObjectType::Item),
            2 => Ok(ObjectType::Container),
            3 => Ok(ObjectType::Unit),
            4 => Ok(ObjectType::Player),
            5 => Ok(ObjectType::GameObject),
            6 => Ok(ObjectType::DynamicObject),
            7 => Ok(ObjectType::Corpse),
            8 => Ok(ObjectType::AreaTrigger),
            9 => Ok(ObjectType::SceneObject),
            _ => Err(ApiError::UnknownObjectType {
                address: self.address,
                value,
            }),
        }
    }

    pub fn guid(&self) -> ApiResult<u64, M::Error> {
        self.descriptor()?.read_u64(ObjectFields::Guid)
    }

    /// Returns the client entry ID for this object.
    pub fn entry_id(&self) -> ApiResult<u32, M::Error> {
        self.descriptor()?.read_u32(ObjectFields::Entry)
    }

    /// Returns the object's display scale multiplier.
    pub fn scale(&self) -> ApiResult<f32, M::Error> {
        self.descriptor()?.read_f32(ObjectFields::ScaleX)
    }

    pub fn descriptor(&self) -> ApiResult<Descriptor<'_, M>, M::Error> {
        let pointer_address = self.offset_address(memory::object::DESCRIPTOR_ARRAY)?;
        let descriptor_address = self
            .api
            .memory()
            .read_u32(pointer_address)
            .map_err(ApiError::Memory)?;

        if descriptor_address == 0 {
            return Err(ApiError::NullPointer {
                context: "object descriptor array",
            });
        }

        Ok(Descriptor::new(self.api.memory(), descriptor_address))
    }

    pub fn next(&self) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        let next_address = self
            .api
            .memory()
            .read_u32(self.offset_address(memory::object::NEXT_OBJECT)?)
            .map_err(ApiError::Memory)?;

        Ok((next_address != 0).then(|| Object::new(self.api, next_address)))
    }

    pub fn into_unit(self) -> ApiResult<Unit<'api, M>, M::Error> {
        match self.object_type()? {
            ObjectType::Unit | ObjectType::Player => Ok(Unit { object: self }),
            _ => Err(ApiError::UnexpectedObjectType {
                address: self.address,
                operation: "convert object to unit",
            }),
        }
    }

    pub fn into_player(self) -> ApiResult<Player<'api, M>, M::Error> {
        match self.object_type()? {
            ObjectType::Player => Ok(Player {
                unit: Unit { object: self },
            }),
            _ => Err(ApiError::UnexpectedObjectType {
                address: self.address,
                operation: "convert object to player",
            }),
        }
    }

    /// Converts this object into a game-object handle after validating its type.
    pub fn into_game_object(self) -> ApiResult<GameObject<'api, M>, M::Error> {
        match self.object_type()? {
            ObjectType::GameObject => Ok(GameObject { object: self }),
            _ => Err(ApiError::UnexpectedObjectType {
                address: self.address,
                operation: "convert object to game object",
            }),
        }
    }

    fn offset_address(&self, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(self.address, offset).ok_or(ApiError::AddressOverflow {
            base: self.address,
            offset,
        })
    }
}

/// A world-object handle known to expose unit fields and unit-base offsets.
pub struct Unit<'api, M> {
    object: Object<'api, M>,
}

impl<'api, M> Unit<'api, M> {
    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.object.address()
    }
}

impl<'api, M: Memory> Unit<'api, M> {
    pub fn health(&self) -> ApiResult<u32, M::Error> {
        self.object.descriptor()?.read_u32(UnitFields::Health)
    }

    pub fn max_health(&self) -> ApiResult<u32, M::Error> {
        self.object.descriptor()?.read_u32(UnitFields::MaxHealth)
    }

    /// Returns the fraction of health remaining, or `None` when maximum health
    /// is zero (for example while a unit is not fully initialized).
    pub fn health_ratio(&self) -> ApiResult<Option<f32>, M::Error> {
        let maximum = self.max_health()?;
        (maximum != 0)
            .then(|| self.health().map(|health| health as f32 / maximum as f32))
            .transpose()
    }

    /// Returns whether the unit has a non-zero health value.
    pub fn is_alive(&self) -> ApiResult<bool, M::Error> {
        self.health().map(|health| health != 0)
    }

    /// Returns the GUID of the unit's current target, if it has one.
    pub fn target_guid(&self) -> ApiResult<Option<u64>, M::Error> {
        self.object
            .descriptor()?
            .read_u64(UnitFields::Target)
            .map(|guid| (guid != 0).then_some(guid))
    }

    /// Returns the GUID of the object this unit is currently channeling, if any.
    pub fn channel_object_guid(&self) -> ApiResult<Option<u64>, M::Error> {
        self.object
            .descriptor()?
            .read_u64(UnitFields::ChannelObject)
            .map(|guid| (guid != 0).then_some(guid))
    }

    /// Returns the spell ID currently being channeled, if any.
    pub fn channel_spell_id(&self) -> ApiResult<Option<u32>, M::Error> {
        self.object
            .descriptor()?
            .read_u32(UnitFields::ChannelSpell)
            .map(|spell_id| (spell_id != 0).then_some(spell_id))
    }

    /// Returns the unit's level from its update fields.
    pub fn level(&self) -> ApiResult<u32, M::Error> {
        self.object.descriptor()?.read_u32(UnitFields::Level)
    }

    /// Returns the faction-template ID used by the client for the unit.
    pub fn faction_template_id(&self) -> ApiResult<u32, M::Error> {
        self.object
            .descriptor()?
            .read_u32(UnitFields::FactionTemplate)
    }

    /// Returns the creature/display model ID selected for this unit.
    pub fn display_id(&self) -> ApiResult<u32, M::Error> {
        self.object.descriptor()?.read_u32(UnitFields::DisplayId)
    }

    /// Returns the unit's combat reach in world units.
    pub fn combat_reach(&self) -> ApiResult<f32, M::Error> {
        self.object.descriptor()?.read_f32(UnitFields::CombatReach)
    }

    /// Number of active aura entries reported by the unit base.
    pub fn aura_count(&self) -> ApiResult<u32, M::Error> {
        self.object
            .api
            .memory()
            .read_u32(self.offset_address(advanced_combat::auras::BASE_AURA_COUNT)?)
            .map_err(ApiError::Memory)
    }

    /// Reads an aura at a zero-based index, returning `None` outside the
    /// client's active aura count or for an empty slot.
    pub fn aura(&self, index: u32) -> ApiResult<Option<Aura>, M::Error> {
        if index >= self.aura_count()? {
            return Ok(None);
        }

        let address = self.aura_address(index)?;
        let memory = self.object.api.memory();
        let spell_id = memory
            .read_u32(self.aura_field_address(address, advanced_combat::auras::SPELL_ID_OFFSET)?)
            .map_err(ApiError::Memory)?;

        if spell_id == 0 {
            return Ok(None);
        }

        Ok(Some(Aura {
            spell_id,
            creator_guid: memory
                .read_u64(
                    self.aura_field_address(address, advanced_combat::auras::CREATOR_GUID_OFFSET)?,
                )
                .map_err(ApiError::Memory)?,
            flags: memory
                .read_u8(self.aura_field_address(address, advanced_combat::auras::FLAGS_OFFSET)?)
                .map_err(ApiError::Memory)?,
            level: memory
                .read_u8(self.aura_field_address(address, advanced_combat::auras::LEVEL_OFFSET)?)
                .map_err(ApiError::Memory)?,
            stack_count: memory
                .read_u8(
                    self.aura_field_address(address, advanced_combat::auras::STACK_COUNT_OFFSET)?,
                )
                .map_err(ApiError::Memory)?,
            duration_ms: memory
                .read_u32(
                    self.aura_field_address(address, advanced_combat::auras::DURATION_OFFSET)?,
                )
                .map_err(ApiError::Memory)? as i32,
            end_time_ms: memory
                .read_u32(
                    self.aura_field_address(address, advanced_combat::auras::END_TIME_OFFSET)?,
                )
                .map_err(ApiError::Memory)? as i32,
        }))
    }

    pub fn position(&self) -> ApiResult<Position, M::Error> {
        let memory = self.object.api.memory();

        Ok(Position {
            x: memory
                .read_f32(self.offset_address(memory::unit::POSITION_X)?)
                .map_err(ApiError::Memory)?,
            y: memory
                .read_f32(self.offset_address(memory::unit::POSITION_Y)?)
                .map_err(ApiError::Memory)?,
            z: memory
                .read_f32(self.offset_address(memory::unit::POSITION_Z)?)
                .map_err(ApiError::Memory)?,
            rotation: memory
                .read_f32(self.offset_address(memory::unit::ROTATION)?)
                .map_err(ApiError::Memory)?,
        })
    }

    pub(crate) const fn object(&self) -> &Object<'api, M> {
        &self.object
    }

    fn offset_address(&self, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(self.address(), offset).ok_or(ApiError::AddressOverflow {
            base: self.address(),
            offset,
        })
    }

    fn aura_address(&self, index: u32) -> ApiResult<RemoteAddress, M::Error> {
        const INLINE_AURA_COUNT: u32 = 40;

        let base = if index < INLINE_AURA_COUNT {
            self.offset_address(advanced_combat::auras::BASE_AURA_ARRAY)?
        } else {
            let pointer_address =
                self.offset_address(advanced_combat::auras::DYNAMIC_AURA_POINTER)?;
            let address = self
                .object
                .api
                .memory()
                .read_u32(pointer_address)
                .map_err(ApiError::Memory)?;

            if address == 0 {
                return Err(ApiError::NullPointer {
                    context: "dynamic aura array",
                });
            }

            return self.aura_entry_address(address, index - INLINE_AURA_COUNT);
        };

        self.aura_entry_address(base, index)
    }

    fn aura_entry_address(
        &self,
        base: RemoteAddress,
        index: u32,
    ) -> ApiResult<RemoteAddress, M::Error> {
        let offset = index
            .checked_mul(advanced_combat::auras::ENTRY_STRIDE)
            .ok_or(ApiError::AddressOverflow {
                base,
                offset: u32::MAX,
            })?;

        with_offset(base, offset).ok_or(ApiError::AddressOverflow { base, offset })
    }

    fn aura_field_address(
        &self,
        aura_address: RemoteAddress,
        offset: u32,
    ) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(aura_address, offset).ok_or(ApiError::AddressOverflow {
            base: aura_address,
            offset,
        })
    }
}

/// Player-specific handle.
pub struct Player<'api, M> {
    unit: Unit<'api, M>,
}

impl<'api, M> Player<'api, M> {
    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.unit.address()
    }
}

impl<'api, M: Memory> Player<'api, M> {
    pub fn health(&self) -> ApiResult<u32, M::Error> {
        self.unit.health()
    }

    pub fn max_health(&self) -> ApiResult<u32, M::Error> {
        self.unit.max_health()
    }

    pub fn health_ratio(&self) -> ApiResult<Option<f32>, M::Error> {
        self.unit.health_ratio()
    }

    pub fn is_alive(&self) -> ApiResult<bool, M::Error> {
        self.unit.is_alive()
    }

    pub fn position(&self) -> ApiResult<Position, M::Error> {
        self.unit.position()
    }

    pub fn guid(&self) -> ApiResult<u64, M::Error> {
        self.unit.object().guid()
    }

    pub fn target_guid(&self) -> ApiResult<Option<u64>, M::Error> {
        self.unit.target_guid()
    }

    pub fn level(&self) -> ApiResult<u32, M::Error> {
        self.unit.level()
    }

    pub fn guild_id(&self) -> ApiResult<u32, M::Error> {
        self.unit
            .object()
            .descriptor()?
            .read_u32(PlayerFields::GuildId)
    }

    pub fn guild_rank(&self) -> ApiResult<u32, M::Error> {
        self.unit
            .object()
            .descriptor()?
            .read_u32(PlayerFields::GuildRank)
    }

    pub fn arena_currency(&self) -> ApiResult<u32, M::Error> {
        self.unit
            .object()
            .descriptor()?
            .read_u32(PlayerFields::ArenaCurrency)
    }

    pub fn honor_currency(&self) -> ApiResult<u32, M::Error> {
        self.unit
            .object()
            .descriptor()?
            .read_u32(PlayerFields::HonorCurrency)
    }

    #[must_use]
    pub const fn unit(&self) -> &Unit<'api, M> {
        &self.unit
    }
}

/// Position and facing read from a unit base.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
}

impl Position {
    /// Euclidean distance to another world-space position, ignoring facing.
    #[must_use]
    pub fn distance_to(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;

        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// A partial, decoded unit aura. Durations use the client's millisecond clock.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Aura {
    pub creator_guid: u64,
    pub spell_id: u32,
    pub flags: u8,
    pub level: u8,
    pub stack_count: u8,
    pub duration_ms: i32,
    pub end_time_ms: i32,
}

/// Handle to a game object with game-object-specific client state.
pub struct GameObject<'api, M> {
    object: Object<'api, M>,
}

impl<'api, M> GameObject<'api, M> {
    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.object.address()
    }
}

impl<'api, M: Memory> GameObject<'api, M> {
    pub fn guid(&self) -> ApiResult<u64, M::Error> {
        self.object.guid()
    }

    pub fn entry_id(&self) -> ApiResult<u32, M::Error> {
        self.object.entry_id()
    }

    pub fn state(&self) -> ApiResult<u8, M::Error> {
        self.object
            .api
            .memory()
            .read_u8(self.offset_address(advanced_combat::game_objects::GAMEOBJECT_STATE)?)
            .map_err(ApiError::Memory)
    }

    pub fn display_id(&self) -> ApiResult<u32, M::Error> {
        self.object
            .descriptor()?
            .read_u32(GameObjectFields::DisplayId)
    }

    pub fn faction_id(&self) -> ApiResult<u32, M::Error> {
        self.object
            .descriptor()?
            .read_u32(GameObjectFields::Faction)
    }

    fn offset_address(&self, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(self.address(), offset).ok_or(ApiError::AddressOverflow {
            base: self.address(),
            offset,
        })
    }
}
