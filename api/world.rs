use std::collections::HashSet;

use super::super::{memory, RemoteAddress};
use super::memory::with_offset;
use super::{ApiError, ApiResult, GameApi, Memory, Object, Player};

/// Root for object-manager and world-object operations.
pub struct World<'api, M> {
    api: &'api GameApi<M>,
}

impl<'api, M> World<'api, M> {
    pub(crate) const fn new(api: &'api GameApi<M>) -> Self {
        Self { api }
    }
}

impl<'api, M: Memory> World<'api, M> {
    /// Returns whether the client currently has a character in the world.
    pub fn is_in_game(&self) -> ApiResult<bool, M::Error> {
        self.api
            .memory()
            .read_u32(memory::game_state::IS_INGAME)
            .map(|value| value != 0)
            .map_err(ApiError::Memory)
    }

    pub fn object_manager(&self) -> ApiResult<ObjectManager<'api, M>, M::Error> {
        let connection = self
            .api
            .memory()
            .read_u32(memory::object_manager::CLIENT_CONNECTION)
            .map_err(ApiError::Memory)?;

        if connection == 0 {
            return Err(ApiError::NullPointer {
                context: "client connection",
            });
        }

        let manager_pointer_address =
            with_offset(connection, memory::object_manager::OBJECT_MANAGER).ok_or(
                ApiError::AddressOverflow {
                    base: connection,
                    offset: memory::object_manager::OBJECT_MANAGER,
                },
            )?;
        let address = self
            .api
            .memory()
            .read_u32(manager_pointer_address)
            .map_err(ApiError::Memory)?;

        if address == 0 {
            return Err(ApiError::NullPointer {
                context: "object manager",
            });
        }

        Ok(ObjectManager {
            api: self.api,
            address,
        })
    }

    /// Finds the current local player using the default traversal bound.
    pub fn local_player(&self) -> ApiResult<Option<Player<'api, M>>, M::Error> {
        self.object_manager()?.local_player()
    }
}

/// Handle to the current object manager.
pub struct ObjectManager<'api, M> {
    api: &'api GameApi<M>,
    address: RemoteAddress,
}

impl<'api, M> ObjectManager<'api, M> {
    /// Protective default for object-list traversal in normal game state.
    pub const DEFAULT_OBJECT_LIMIT: u32 = 10_000;

    #[must_use]
    pub const fn address(&self) -> RemoteAddress {
        self.address
    }
}

impl<'api, M: Memory> ObjectManager<'api, M> {
    pub fn local_guid(&self) -> ApiResult<u64, M::Error> {
        self.api
            .memory()
            .read_u64(self.offset_address(memory::object_manager::LOCAL_GUID)?)
            .map_err(ApiError::Memory)
    }

    pub fn first_object(&self) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        let address = self
            .api
            .memory()
            .read_u32(self.offset_address(memory::object_manager::FIRST_OBJECT)?)
            .map_err(ApiError::Memory)?;

        Ok((address != 0).then(|| Object::new(self.api, address)))
    }

    /// Iterates world objects with a bounded traversal and cycle detection.
    pub fn objects(&self) -> ApiResult<ObjectList<'api, M>, M::Error> {
        self.objects_with_limit(Self::DEFAULT_OBJECT_LIMIT)
    }

    /// Same as [`Self::objects`], with a caller-selected traversal bound.
    pub fn objects_with_limit(&self, max_objects: u32) -> ApiResult<ObjectList<'api, M>, M::Error> {
        Ok(ObjectList {
            next: self.first_object()?,
            visited: HashSet::new(),
            remaining: max_objects,
        })
    }

    /// Finds an object by GUID using the default traversal bound.
    pub fn object_by_guid(&self, guid: u64) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        self.object_by_guid_with_limit(guid, Self::DEFAULT_OBJECT_LIMIT)
    }

    /// Same as [`Self::object_by_guid`], with a caller-selected traversal bound.
    pub fn object_by_guid_with_limit(
        &self,
        guid: u64,
        max_objects: u32,
    ) -> ApiResult<Option<Object<'api, M>>, M::Error> {
        for object in self.objects_with_limit(max_objects)? {
            let object = object?;
            if object.guid()? == guid {
                return Ok(Some(object));
            }
        }

        Ok(None)
    }

    /// Finds the local player by walking the object list with the default bound.
    pub fn local_player(&self) -> ApiResult<Option<Player<'api, M>>, M::Error> {
        self.local_player_with_limit(Self::DEFAULT_OBJECT_LIMIT)
    }

    /// Same as [`Self::local_player`], with a caller-selected traversal bound.
    pub fn local_player_with_limit(
        &self,
        max_objects: u32,
    ) -> ApiResult<Option<Player<'api, M>>, M::Error> {
        let local_guid = self.local_guid()?;
        for object in self.objects_with_limit(max_objects)? {
            let object = object?;
            if object.guid()? == local_guid {
                return object.into_player().map(Some);
            }
        }

        Ok(None)
    }

    fn offset_address(&self, offset: u32) -> ApiResult<RemoteAddress, M::Error> {
        with_offset(self.address, offset).ok_or(ApiError::AddressOverflow {
            base: self.address,
            offset,
        })
    }
}

/// Bounded fallible traversal of the object-manager linked list.
pub struct ObjectList<'api, M> {
    next: Option<Object<'api, M>>,
    visited: HashSet<RemoteAddress>,
    remaining: u32,
}

impl<'api, M: Memory> Iterator for ObjectList<'api, M> {
    type Item = ApiResult<Object<'api, M>, M::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let object = self.next.take()?;
        self.remaining -= 1;

        if !self.visited.insert(object.address()) {
            return Some(Err(ApiError::ObjectListCycle {
                address: object.address(),
            }));
        }

        match object.next() {
            Ok(next) => {
                self.next = next;
                Some(Ok(object))
            }
            Err(error) => Some(Err(error)),
        }
    }
}
