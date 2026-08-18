use std::collections::HashSet;

use super::super::{advanced_combat, RemoteAddress};
use super::memory::with_offset;
use super::{ApiError, ApiResult, GameApi, Memory, Object, Unit};

/// Entry point for additional combat state and casting data.
pub struct AdvancedCombat<'api, M> {
    api: &'api GameApi<M>,
}

impl<'api, M> AdvancedCombat<'api, M> {
    pub(crate) const fn new(api: &'api GameApi<M>) -> Self {
        Self { api }
    }
}

impl<'api, M: Memory> AdvancedCombat<'api, M> {
    pub fn combo_points(&self) -> ApiResult<u8, M::Error> {
        self.api
            .memory()
            .read_u8(advanced_combat::state::COMBO_POINTS)
            .map_err(ApiError::Memory)
    }

    pub fn mouseover_guid(&self) -> ApiResult<Option<u64>, M::Error> {
        self.api
            .memory()
            .read_u64(advanced_combat::state::MOUSEOVER_GUID)
            .map(|guid| (guid != 0).then_some(guid))
            .map_err(ApiError::Memory)
    }

    pub fn focus_guid(&self) -> ApiResult<Option<u64>, M::Error> {
        self.api
            .memory()
            .read_u64(advanced_combat::state::FOCUS_GUID)
            .map(|guid| (guid != 0).then_some(guid))
            .map_err(ApiError::Memory)
    }

    /// Resolves the mouseover object using the object manager's default bound.
    pub fn mouseover(&self) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        self.mouseover_with_limit(super::ObjectManager::<M>::DEFAULT_OBJECT_LIMIT)
    }

    /// Same as [`Self::mouseover`], with a caller-selected traversal bound.
    pub fn mouseover_with_limit(
        &self,
        max_objects: u32,
    ) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        let Some(guid) = self.mouseover_guid()? else {
            return Ok(None);
        };

        self.api
            .world()
            .object_manager()?
            .object_by_guid_with_limit(guid, max_objects)
    }

    /// Resolves the focus object using the object manager's default bound.
    pub fn focus(&self) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        self.focus_with_limit(super::ObjectManager::<M>::DEFAULT_OBJECT_LIMIT)
    }

    /// Same as [`Self::focus`], with a caller-selected traversal bound.
    pub fn focus_with_limit(
        &self,
        max_objects: u32,
    ) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        let Some(guid) = self.focus_guid()? else {
            return Ok(None);
        };

        self.api
            .world()
            .object_manager()?
            .object_by_guid_with_limit(guid, max_objects)
    }

    pub fn is_auto_attacking(&self) -> ApiResult<bool, M::Error> {
        self.api
            .memory()
            .read_u32(advanced_combat::state::IS_AUTO_ATTACKING)
            .map(|value| value != 0)
            .map_err(ApiError::Memory)
    }

    /// Reads the spell ID from the unit base, if a spell is currently active.
    pub fn current_spell_id(&self, unit: &Unit<'_, M>) -> ApiResult<Option<u32>, M::Error> {
        self.api
            .memory()
            .read_u32(
                self.unit_address(unit.address(), advanced_combat::casting::CURRENT_SPELL_ID)?,
            )
            .map(|spell_id| (spell_id != 0).then_some(spell_id))
            .map_err(ApiError::Memory)
    }

    /// Reads the active cast, returning `None` when the unit is not casting.
    pub fn current_cast(&self, unit: &Unit<'_, M>) -> ApiResult<Option<CastInfo>, M::Error> {
        let pointer_slot = self.unit_address(
            unit.address(),
            advanced_combat::casting::SPELL_CAST_STRUCT_PTR,
        )?;
        let cast_address = self
            .api
            .memory()
            .read_u32(pointer_slot)
            .map_err(ApiError::Memory)?;

        if cast_address == 0 {
            return Ok(None);
        }

        Ok(Some(CastInfo {
            spell_id: self.read_u32(cast_address, advanced_combat::casting::SPELL_ID_OFFSET)?,
            start_time: self.read_u32(cast_address, advanced_combat::casting::START_TIME_OFFSET)?,
            end_time: self.read_u32(cast_address, advanced_combat::casting::END_TIME_OFFSET)?,
            is_channeling: self
                .api
                .memory()
                .read_u8(
                    self.cast_address(
                        cast_address,
                        advanced_combat::casting::IS_CHANNELING_OFFSET,
                    )?,
                )
                .map(|value| value != 0)
                .map_err(ApiError::Memory)?,
        }))
    }

    /// Iterates spell-cooldown entries with a protective default bound.
    pub fn cooldowns(&self) -> ApiResult<CooldownList<'api, M>, M::Error> {
        self.cooldowns_with_limit(CooldownList::<M>::DEFAULT_ENTRY_LIMIT)
    }

    /// Same as [`Self::cooldowns`], with a caller-selected traversal bound.
    pub fn cooldowns_with_limit(
        &self,
        max_entries: u32,
    ) -> ApiResult<CooldownList<'api, M>, M::Error> {
        Ok(CooldownList {
            api: self.api,
            next: self.cooldown_head()?,
            visited: HashSet::new(),
            remaining: max_entries,
        })
    }

    /// Finds a spell cooldown with the default traversal bound.
    pub fn spell_cooldown(&self, spell_id: u32) -> ApiResult<Option<SpellCooldown>, M::Error> {
        self.spell_cooldown_with_limit(spell_id, CooldownList::<M>::DEFAULT_ENTRY_LIMIT)
    }

    /// Same as [`Self::spell_cooldown`], with a caller-selected traversal bound.
    pub fn spell_cooldown_with_limit(
        &self,
        spell_id: u32,
        max_entries: u32,
    ) -> ApiResult<Option<SpellCooldown>, M::Error> {
        for cooldown in self.cooldowns_with_limit(max_entries)? {
            let cooldown = cooldown?;
            if cooldown.spell_id == spell_id {
                return Ok(Some(cooldown));
            }
        }

        Ok(None)
    }

    fn cooldown_head(&self) -> ApiResult<Option<RemoteAddress>, M::Error> {
        self.api
            .memory()
            .read_u32(advanced_combat::cooldown::SPELL_COOLDOWN_PTR)
            .map(|address| (address != 0).then_some(address))
            .map_err(ApiError::Memory)
    }

    fn read_u32(&self, base: RemoteAddress, offset: u32) -> ApiResult<u32, M::Error> {
        self.api
            .memory()
            .read_u32(self.cast_address(base, offset)?)
            .map_err(ApiError::Memory)
    }

    fn unit_address(&self, base: RemoteAddress, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(base, offset).ok_or(ApiError::AddressOverflow { base, offset })
    }

    fn cast_address(&self, base: RemoteAddress, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(base, offset).ok_or(ApiError::AddressOverflow { base, offset })
    }
}

/// Decoded, stable portion of the current spell-cast state.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CastInfo {
    pub spell_id: u32,
    pub start_time: u32,
    pub end_time: u32,
    pub is_channeling: bool,
}

/// One spell-cooldown entry decoded from the client's cooldown list.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SpellCooldown {
    pub spell_id: u32,
    pub item_id: u32,
    /// Client millisecond-clock value at which the cooldown started.
    pub start_time_ms: u32,
    pub duration_ms: u32,
}

/// Bounded, fallible traversal of the client's spell-cooldown linked list.
pub struct CooldownList<'api, M> {
    api: &'api GameApi<M>,
    next: Option<RemoteAddress>,
    visited: HashSet<RemoteAddress>,
    remaining: u32,
}

impl<'api, M> CooldownList<'api, M> {
    /// Protective default for a normal cooldown list.
    pub const DEFAULT_ENTRY_LIMIT: u32 = 1_024;
}

impl<'api, M: Memory> Iterator for CooldownList<'api, M> {
    type Item = ApiResult<SpellCooldown, M::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let address = self.next.take()?;
        self.remaining -= 1;

        if !self.visited.insert(address) {
            return Some(Err(ApiError::CooldownListCycle { address }));
        }

        let read_u32 = |offset| {
            let field_address = with_offset(address, offset).ok_or(ApiError::AddressOverflow {
                base: address,
                offset,
            })?;
            self.api
                .memory()
                .read_u32(field_address)
                .map_err(ApiError::Memory)
        };

        let next = match read_u32(advanced_combat::cooldown::NEXT_OFFSET) {
            Ok(next) => next,
            Err(error) => return Some(Err(error)),
        };
        let spell_id = match read_u32(advanced_combat::cooldown::SPELL_ID_OFFSET) {
            Ok(spell_id) => spell_id,
            Err(error) => return Some(Err(error)),
        };
        let item_id = match read_u32(advanced_combat::cooldown::ITEM_ID_OFFSET) {
            Ok(item_id) => item_id,
            Err(error) => return Some(Err(error)),
        };
        let start_time_ms = match read_u32(advanced_combat::cooldown::START_TIME_OFFSET) {
            Ok(start_time_ms) => start_time_ms,
            Err(error) => return Some(Err(error)),
        };
        let duration_ms = match read_u32(advanced_combat::cooldown::DURATION_OFFSET) {
            Ok(duration_ms) => duration_ms,
            Err(error) => return Some(Err(error)),
        };

        self.next = (next != 0).then_some(next);
        Some(Ok(SpellCooldown {
            spell_id,
            item_id,
            start_time_ms,
            duration_ms,
        }))
    }
}
