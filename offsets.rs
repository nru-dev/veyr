//! Memory map for World of Warcraft 3.3.5a, build 12340 (x86).
//!
//! `Address` values are absolute virtual addresses in the game process, while
//! `Offset` values are byte offsets from an explicitly documented base.
//!
//! # Critical: update fields are not byte offsets
//!
//! `ObjectFields`, `UnitFields`, `PlayerFields`, and similar enums contain
//! indices of 32-bit descriptor values. They must be multiplied by four before
//! adding them to a descriptor-array address. Do not do that arithmetic at call
//! sites: use [`offsets::update_fields::address_of`] instead.
//!
//! ```ignore
//! let health_address = offsets::update_fields::address_of(
//!     descriptor_array,
//!     offsets::UnitFields::Health,
//! );
//! ```
//!
//! All values in this module are build-specific. Do not reuse them for another
//! client build without validating the corresponding memory layout.

/// Internal implementation namespace; its public contents are re-exported by
/// this `offsets` module below.
mod layout {
    /// A 32-bit address stored in a structure owned by the x86 game client.
    ///
    /// This is deliberately not `usize`: the remote structure is always x86
    /// even when the reader/injector itself is built for a 64-bit host.
    pub type RemoteAddress = u32;

    /// An absolute virtual address in the 32-bit WoW client process.
    pub type Address = RemoteAddress;

    /// A byte offset within a client-side structure.
    pub type Offset = u32;

    /// An index into an update-field descriptor array (one index is four bytes).
    pub type FieldIndex = u32;

    /// Client build whose addresses and layouts are described by this module.
    pub const BUILD: u32 = 12_340;

    /// Architecture of the target game process.
    pub const ARCHITECTURE: &str = "x86";

    /// Identity of the exact executable examined for image-specific offsets.
    ///
    /// Descriptor indices are part of the 3.3.5a update protocol and are
    /// shared by compatible 12340 clients. Absolute addresses and ordinary
    /// structure offsets are not: they must match this image or have their
    /// own explicit live validation record.
    pub mod profile {
        /// Preferred PE image base of the supported executable.
        pub const IMAGE_BASE: u32 = 0x0040_0000;
        /// PE `SizeOfImage` of the examined executable.
        pub const IMAGE_SIZE: u32 = 0x009F_D000;
        /// PE COFF timestamp of the examined executable.
        pub const PE_TIMESTAMP: u32 = 0x4C24_8B3E;
        /// SHA-256 of the exact `Wow.exe` used to recover camera offsets.
        pub const WOW_EXE_SHA256: &str =
            "07c51ead92b0d420247fb8100cd2fc1f0c33117ca4f4743a557cfb1cbdede0bc";

        /// How a group of values entered this map.
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub enum Evidence {
            /// Recovered from and cross-checked against the exact executable.
            ExactExecutable,
            /// Observed successfully in the supported client at runtime.
            LiveClient,
            /// Defined by the 3.3.5a update-field protocol.
            UpdateProtocol,
            /// Retained candidate only; do not add new behaviour on it until
            /// it receives an executable or live-client validation.
            Candidate,
        }

        /// One documented area of the offset map.
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct Group {
            pub path: &'static str,
            pub evidence: Evidence,
            pub note: &'static str,
        }

        /// Whether a requested developer capability is represented by a
        /// usable offset group today. `Candidate` deliberately does not mean
        /// safe to call or write: it only preserves a lead for recovery.
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub enum Coverage {
            Mapped,
            Candidate,
            RequiresRecovery,
            IntentionallyOmitted,
        }

        /// Domain-level inventory. This is the answer to “do we have an
        /// offset for it?” without turning an unknown address into `0` or a
        /// fictional constant.
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct Capability {
            pub domain: &'static str,
            pub coverage: Coverage,
            pub paths: &'static str,
            pub note: &'static str,
        }

        /// Complete provenance inventory for the hand-maintained map below.
        ///
        /// This avoids the old failure mode where a random 12340 address sat
        /// beside a verified one with no visible distinction.
        pub const GROUPS: &[Group] = &[
            Group {
                path: "memory::object_manager, memory::object, memory::unit, memory::game_state::IS_INGAME",
                evidence: Evidence::LiveClient,
                note: "Used successfully by local-player world-circle reads.",
            },
            Group {
                path: "advanced_combat::camera",
                evidence: Evidence::ExactExecutable,
                note: "Recovered from the executable identified above.",
            },
            Group {
                path: "memory::{world, ui}, functions::frame_script::{GET_CURRENT_KEYBOARD_FOCUS, loot}",
                evidence: Evidence::ExactExecutable,
                note: "Recovered from native FrameScript registration and direct xrefs in the exact executable; call ABI is intentionally not exposed.",
            },
            Group {
                path: "functions::world::CG_WORLD_FRAME_INTERSECT",
                evidence: Evidence::ExactExecutable,
                note: "Recovered from direct xrefs to the client's CWorld/CGWorldFrame collision diagnostics; ABI and hit record remain unvalidated.",
            },
            Group {
                path: "ObjectFields, ItemFields, ContainerFields, UnitFields, PlayerFields, GameObjectFields, DynamicObjectFields, CorpseFields",
                evidence: Evidence::UpdateProtocol,
                note: "3.3.5a update-field indices; each index addresses a u32.",
            },
            Group {
                path: "memory::click_to_move, functions (except exact FrameScript/collision entries), advanced_combat::{cooldown, casting, auras, game_objects, state}, spell_dbc",
                evidence: Evidence::Candidate,
                note: "Imported legacy candidates; no new caller may treat them as verified.",
            },
            Group {
                path: "advanced_combat::hooks::loader_discovery",
                evidence: Evidence::Candidate,
                note: "Optional loader fallback only; launch-time D3D9 capture is authoritative.",
            },
        ];

        /// Full developer-facing coverage ledger for this map.
        pub const CAPABILITIES: &[Capability] = &[
            Capability {
                domain: "Entities: GUID, kind, entry, scale, position, rotation",
                coverage: Coverage::Mapped,
                paths: "memory::{object_manager, object, unit}, ObjectFields",
                note: "World traversal and local-player position were read successfully in the live client.",
            },
            Capability {
                domain: "Units: health, all powers, level, faction, display, reach, flags, attack data",
                coverage: Coverage::Mapped,
                paths: "UnitFields",
                note: "Protocol descriptor indices; use update_fields::address_of.",
            },
            Capability {
                domain: "Player: attributes, armor/resistances, combat ratings, crit/expertise, XP, skills, inventory, currencies",
                coverage: Coverage::Mapped,
                paths: "PlayerFields, descriptor_layout",
                note: "Protocol descriptor indices; no write behaviour is implied.",
            },
            Capability {
                domain: "Items, bags, containers, visible equipment, corpses, game objects, dynamic area objects",
                coverage: Coverage::Mapped,
                paths: "ItemFields, ContainerFields, GameObjectFields, DynamicObjectFields, CorpseFields",
                note: "Protocol descriptor indices. Object templates and localized names are separate client data.",
            },
            Capability {
                domain: "Spells: cost, cast/recovery, range, effects, raw base damage/heal values, auras, reagents, visuals",
                coverage: Coverage::Candidate,
                paths: "spell_dbc::{SPELL_STORE, entry}",
                note: "12340 record layout is catalogued; the client DBC-store pointer must be validated against this executable before use.",
            },
            Capability {
                domain: "Active auras, cast state, cooldowns, combo points, attack state",
                coverage: Coverage::Candidate,
                paths: "advanced_combat::{auras, casting, cooldown, state}",
                note: "Legacy client-layout leads only; validate against a live character before exposing through API.",
            },
            Capability {
                domain: "Camera and world-to-screen inputs",
                coverage: Coverage::Mapped,
                paths: "advanced_combat::camera",
                note: "Recovered from the exact executable and observed in the live client.",
            },
            Capability {
                domain: "Terrain height, static collision, world raycasts, obstruction",
                coverage: Coverage::Candidate,
                paths: "memory::world, functions::world",
                note: "The CWorld pointer slot and CGWorldFrame intersection candidate are now exact-binary values; their ABI, hit record, and terrain-height route still need live validation.",
            },
            Capability {
                domain: "Static scene: terrain, buildings, rocks, trees, doodads, water",
                coverage: Coverage::Candidate,
                paths: "world_assets::{adt, game_object_display_info}",
                note: "The 3.3.5 asset record layouts are catalogued. Reading them needs an MPQ/ADT asset reader, not a client-memory offset; exact triangle collision still needs M2/WMO parsing or the validated CWorld query.",
            },
            Capability {
                domain: "Movement state and click-to-move command block",
                coverage: Coverage::Candidate,
                paths: "memory::unit, memory::click_to_move, functions::unit::CLICK_TO_MOVE",
                note: "Position is live-validated; command writes/call ABI remain intentionally unimplemented.",
            },
            Capability {
                domain: "Gathering/interactable objects",
                coverage: Coverage::Candidate,
                paths: "ObjectFields::Entry, GameObjectFields, advanced_combat::game_objects, functions::game_object",
                note: "Object identity is mapped; template lookup, interaction ABI, and loot result are separate validation work.",
            },
            Capability {
                domain: "Loot window, slots, item quantities, and loot ownership",
                coverage: Coverage::RequiresRecovery,
                paths: "functions::frame_script::loot",
                note: "Exact native FrameScript entry points are recorded, but the loot-session structure and a safe direct reader are not recovered yet.",
            },
            Capability {
                domain: "Menus/UI layout, visible frame tree, keyboard focus (typing detector)",
                coverage: Coverage::Candidate,
                paths: "memory::ui::KEYBOARD_FOCUS, functions::frame_script::GET_CURRENT_KEYBOARD_FOCUS",
                note: "The exact focus pointer and its native query entry point are mapped; the focused-frame layout still requires live validation. Chat text remains out of scope.",
            },
            Capability {
                domain: "Quest log, chat contents, cursor, target selection and target state",
                coverage: Coverage::IntentionallyOmitted,
                paths: "—",
                note: "Excluded from the current requested offset scope.",
            },
        ];
    }

    /// Safe conversion helpers for update-field descriptor indices.
    ///
    /// Every descriptor field is one `u32` wide. These helpers centralize the
    /// required `index * 4` conversion, so call sites never perform it by hand.
    pub mod update_fields {
        use super::{FieldIndex, Offset, RemoteAddress};

        /// Size of one descriptor field in bytes.
        pub const BYTES_PER_FIELD: Offset = 4;

        /// Descriptor type that supplies a 32-bit field index.
        pub trait Field: Copy {
            fn index(self) -> FieldIndex;
        }

        /// Converts a descriptor field index into its byte offset.
        #[must_use]
        pub const fn byte_offset(index: FieldIndex) -> Offset {
            index * BYTES_PER_FIELD
        }

        /// Returns the byte offset of a typed descriptor field.
        #[must_use]
        pub fn byte_offset_of(field: impl Field) -> Offset {
            byte_offset(field.index())
        }

        /// Returns the remote address of a typed descriptor field.
        #[must_use]
        pub fn address_of(descriptor_array: RemoteAddress, field: impl Field) -> RemoteAddress {
            descriptor_array + byte_offset_of(field)
        }

        /// Returns the remote address of a descriptor field, or `None` on overflow.
        #[must_use]
        pub fn checked_address_of(
            descriptor_array: RemoteAddress,
            field: impl Field,
        ) -> Option<RemoteAddress> {
            descriptor_array.checked_add(byte_offset_of(field))
        }
    }

    /// Static addresses and in-structure offsets.
    pub mod memory {
        use super::{Address, Offset};

        /// Object-manager discovery and object-list traversal.
        pub mod object_manager {
            use super::{Address, Offset};

            /// Pointer to `ClientConnection`.
            pub const CLIENT_CONNECTION: Address = 0x00C7_9CE0;
            /// Offset from `ClientConnection` to the object manager pointer.
            pub const OBJECT_MANAGER: Offset = 0x2ED0;
            /// Offset from the object manager to the first world object.
            pub const FIRST_OBJECT: Offset = 0xAC;
            /// Offset from the object manager to the local player's GUID.
            pub const LOCAL_GUID: Offset = 0xC0;
        }

        /// Fields common to every world-object instance.
        pub mod object {
            use super::Offset;

            /// Pointer to the update-field descriptor array.
            pub const DESCRIPTOR_ARRAY: Offset = 0x08;
            /// `ObjectType` value (`u32`).
            pub const TYPE: Offset = 0x14;
            /// Offset from an object to the next object in the object list.
            pub const NEXT_OBJECT: Offset = 0x3C;
        }

        /// Fields from the unit/player base object.
        pub mod unit {
            use super::Offset;

            /// Pointer to the movement structure.
            pub const POINTER_TO_MOVEMENT: Offset = 0x10;
            pub const POSITION_X: Offset = 0x798;
            pub const POSITION_Y: Offset = 0x79C;
            pub const POSITION_Z: Offset = 0x7A0;
            /// Facing angle in radians.
            pub const ROTATION: Offset = 0x7A4;
        }

        /// Click-to-move command block.
        pub mod click_to_move {
            use super::Address;

            pub const BASE: Address = 0x00CA_11D8;
            /// Command action (`CtmActionType`, `u32`).
            pub const ACTION_TYPE: Address = BASE;
            pub const X: Address = 0x00CA_11DC;
            pub const Y: Address = 0x00CA_11E0;
            pub const Z: Address = 0x00CA_11E4;
            pub const TARGET_GUID: Address = 0x00CA_11C8;
            /// Arrival precision / stopping distance (`f32`).
            pub const DISTANCE: Address = 0x00CA_11CC;
        }

        /// Game-world state exposed at static client addresses.
        pub mod game_state {
            use super::Address;

            /// Non-zero while a character is in the game world.
            pub const IS_INGAME: Address = 0x00B6_A9E0;
            pub const REALM_NAME: Address = 0x00C7_9D18;
            /// Pointer to the spell-cooldown linked list.
            pub const SPELL_COOLDOWN_PTR: Address = 0x00CE_CAEC;
        }

        /// World-render and collision roots.
        pub mod world {
            use super::Address;

            /// Pointer slot consumed by the client's world-intersection path.
            ///
            /// It is *not* a terrain-height value and its pointed-to layout is
            /// not exposed until the collision ABI is validated in a live map.
            pub const COLLISION_WORLD: Address = 0x00CD_754C;
        }

        /// FrameXML/FrameScript state. These are client internals, not the
        /// overlay's own UI system.
        pub mod ui {
            use super::Address;

            /// Pointer used by the native `GetCurrentKeyBoardFocus` handler.
            /// A null pointer means that handler reports no focused frame.
            /// The pointed-to frame layout has not yet been reverse-validated.
            pub const KEYBOARD_FOCUS: Address = 0x00DC_E474;
        }
    }

    /// Addresses of native client functions.
    ///
    /// Calling conventions and parameter layouts are intentionally not encoded
    /// here; callers must define those at the call site and keep them audited.
    pub mod functions {
        use super::Address;

        pub mod frame_script {
            use super::Address;

            pub const EXECUTE: Address = 0x0081_9210;
            pub const GET_TEXT: Address = 0x0070_40D0;

            /// Native handler registered as `GetCurrentKeyBoardFocus`.
            ///
            /// This is a FrameScript VM function, not a regular C ABI. Its
            /// direct registration-table xref reads
            /// [`memory::ui::KEYBOARD_FOCUS`].
            pub const GET_CURRENT_KEYBOARD_FOCUS: Address = 0x0081_B820;

            /// Native FrameScript queries for discovering the client UI tree.
            /// Like all FrameScript handlers, these require the VM ABI and are
            /// catalogued here only as exact client entry points.
            pub mod ui {
                use super::Address;

                pub const GET_FRAMES_REGISTERED_FOR_EVENT: Address = 0x0081_BE70;
                pub const ENUMERATE_FRAMES: Address = 0x0081_B9C0;
                pub const GET_NUM_FRAMES: Address = 0x0081_BAB0;
            }

            /// Native handlers registered for the built-in loot UI.
            ///
            /// They all expect the FrameScript VM calling convention. They
            /// document the exact client entry points only; they must not be
            /// invoked from injected Rust until that ABI is wrapped safely.
            pub mod loot {
                use super::Address;

                pub const GET_NUM_ITEMS: Address = 0x0058_8540;
                pub const GET_SLOT_INFO: Address = 0x0058_8570;
                pub const GET_SLOT_LINK: Address = 0x0058_86D0;
                pub const SLOT_IS_ITEM: Address = 0x0058_8750;
                pub const SLOT_IS_COIN: Address = 0x0058_8810;
                pub const LOOT_SLOT: Address = 0x0058_9520;
                pub const CLOSE_LOOT: Address = 0x0058_88B0;
            }
        }

        pub mod unit {
            use super::Address;

            pub const CLICK_TO_MOVE: Address = 0x0061_1130;
            pub const SET_TARGET: Address = 0x0052_3A40;
            pub const GET_THREAT_STATE: Address = 0x0052_5D50;
        }

        pub mod world {
            use super::Address;

            /// Legacy raycast lead. Its ABI is not part of the API until the
            /// exact calling convention is audited.
            pub const TRACE_LINE: Address = 0x007A_5640;

            /// Build-12340 `CGWorldFrame::Intersect` implementation.
            ///
            /// Static auditing recovers the x86 member-function ABI as
            /// `thiscall(CGWorldFrame*, Vector3 const*, Vector3 const*, u32,
            /// Hit*) -> u32`; successful terrain return codes `1` and `2`
            /// write a hit position at `Hit + 0x08` and distance at
            /// `Hit + 0x14`. This remains a
            /// trusted injected-runtime primitive until its live probe has
            /// validated every collision flag and model-hit result. It is not
            /// a public developer API and must run only on the game thread.
            pub const CG_WORLD_FRAME_INTERSECT: Address = 0x004F_9930;

            /// Two-vector `CGWorldFrame` query adjacent to
            /// [`CG_WORLD_FRAME_INTERSECT`]. Its x86 member-function calling
            /// convention and output semantics still need a live probe.
            pub const CG_WORLD_FRAME_SEGMENT_QUERY: Address = 0x004F_9410;
        }

        pub mod game_object {
            use super::Address;

            pub const INTERACT: Address = 0x005E_2B90;
            pub const GET_NAME: Address = 0x0070_2850;
        }

        pub mod player {
            use super::Address;

            pub const GET_NAME_BY_GUID: Address = 0x0072_3D40;
        }

        pub mod spell {
            use super::Address;

            pub const GET_COOLDOWN: Address = 0x006E_13E0;
        }
    }

    /// Runtime object kinds stored at [`memory::object::TYPE`].
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum ObjectType {
        Object = 0,
        Item = 1,
        Container = 2,
        Unit = 3,
        Player = 4,
        GameObject = 5,
        DynamicObject = 6,
        Corpse = 7,
        AreaTrigger = 8,
        SceneObject = 9,
    }

    /// Actions accepted by the click-to-move command block.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum CtmActionType {
        None = 0,
        FaceTarget = 1,
        FaceLocation = 2,
        Move = 4,
        NpcInteract = 5,
        Loot = 6,
        ObjInteract = 7,
        AttackGuided = 8,
        AttackPosition = 9,
        AttackUnk = 10,
        Follow = 11,
    }

    /// Type namespace for discoverability in new code.
    pub mod types {
        pub use super::{CtmActionType, ObjectType};
    }

    /// Sizes and strides for descriptor-field arrays.
    ///
    /// These are counts of *descriptor indices* unless their documentation says
    /// otherwise. Use them with [`update_fields::byte_offset`] rather than
    /// turning them into byte offsets by hand.
    pub mod descriptor_layout {
        use super::FieldIndex;

        /// `UNIT_FIELD_POWER1..=UNIT_FIELD_POWER7` and matching max/regen arrays.
        pub const POWER_TYPE_COUNT: FieldIndex = 7;
        /// Strength, agility, stamina, intellect, and spirit.
        pub const PRIMARY_STAT_COUNT: FieldIndex = 5;
        /// Armor plus the six magic schools.
        pub const RESISTANCE_COUNT: FieldIndex = 7;
        /// Item-enchantment descriptor records.
        pub const ITEM_ENCHANTMENT_SLOT_COUNT: FieldIndex = 12;
        /// Spell ID, duration, and charges per item enchantment.
        pub const ITEM_ENCHANTMENT_STRIDE: FieldIndex = 3;
        /// 64-bit item GUIDs carried by a container.
        pub const CONTAINER_SLOT_COUNT: FieldIndex = 36;
        /// Displayed equipment records on a player.
        pub const VISIBLE_ITEM_COUNT: FieldIndex = 19;
        /// Entry ID plus its displayed enchantment for one visible item.
        pub const VISIBLE_ITEM_STRIDE: FieldIndex = 2;
        /// Player inventory GUIDs, including equipped items and bag slots.
        pub const INVENTORY_GUID_COUNT: FieldIndex = 23;
        /// Standard backpack GUIDs.
        pub const PACK_GUID_COUNT: FieldIndex = 16;
        /// Bank item GUIDs.
        pub const BANK_GUID_COUNT: FieldIndex = 28;
        /// Bank bag GUIDs.
        pub const BANK_BAG_GUID_COUNT: FieldIndex = 7;
        /// Vendor buyback item GUIDs.
        pub const BUYBACK_GUID_COUNT: FieldIndex = 12;
        /// Keyring item GUIDs.
        pub const KEYRING_GUID_COUNT: FieldIndex = 32;
        /// Currency-token item GUIDs.
        pub const CURRENCY_TOKEN_GUID_COUNT: FieldIndex = 32;
        /// Schools represented by the player spell-critical and damage arrays.
        pub const SPELL_SCHOOL_COUNT: FieldIndex = 7;
        /// Explored-zone bitmasks.
        pub const EXPLORED_ZONE_MASK_COUNT: FieldIndex = 128;
        /// Combat-rating entries in the descriptor.
        pub const COMBAT_RATING_COUNT: FieldIndex = 25;
        /// Descriptor fields allocated to arena-team information.
        pub const ARENA_TEAM_INFO_FIELD_COUNT: FieldIndex = 21;
        /// Death-knight rune regeneration fields.
        pub const RUNE_REGEN_COUNT: FieldIndex = 4;
        /// Power types with no reagent cost bitmasks.
        pub const NO_REAGENT_COST_MASK_COUNT: FieldIndex = 3;
        /// Glyph slots and glyph IDs.
        pub const GLYPH_SLOT_COUNT: FieldIndex = 6;
    }

    /// Update-field indices common to all world objects.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum ObjectFields {
        /// 64-bit GUID occupying indices `0x0..=0x1`.
        Guid = 0x0,
        Type = 0x2,
        Entry = 0x3,
        ScaleX = 0x4,
        /// Protocol alignment field. It still occupies one descriptor index.
        Padding = 0x5,
        /// One past the final object field.
        End = 0x6,
    }

    /// Update-field indices for items.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum ItemFields {
        Owner = ObjectFields::End as FieldIndex,
        Contained = ObjectFields::End as FieldIndex + 0x2,
        Creator = ObjectFields::End as FieldIndex + 0x4,
        GiftCreator = ObjectFields::End as FieldIndex + 0x6,
        StackCount = ObjectFields::End as FieldIndex + 0x8,
        Duration = ObjectFields::End as FieldIndex + 0x9,
        SpellCharges = ObjectFields::End as FieldIndex + 0xA,
        Flags = ObjectFields::End as FieldIndex + 0xF,
        Enchantment = ObjectFields::End as FieldIndex + 0x10,
        PropertySeed = ObjectFields::End as FieldIndex + 0x34,
        RandomPropertiesId = ObjectFields::End as FieldIndex + 0x35,
        Durability = ObjectFields::End as FieldIndex + 0x36,
        MaxDurability = ObjectFields::End as FieldIndex + 0x37,
        CreatePlayedTime = ObjectFields::End as FieldIndex + 0x38,
        Padding = ObjectFields::End as FieldIndex + 0x39,
        End = ObjectFields::End as FieldIndex + 0x3A,
    }

    /// Update-field indices for containers.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum ContainerFields {
        NumSlots = ItemFields::End as FieldIndex,
        Padding = ItemFields::End as FieldIndex + 0x1,
        /// Array of 36 64-bit item GUIDs (72 descriptor fields).
        Slots = ItemFields::End as FieldIndex + 0x2,
        End = ItemFields::End as FieldIndex + 0x4A,
    }

    /// Update-field indices for units, including NPCs and pets.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum UnitFields {
        Charm = ObjectFields::End as FieldIndex,
        Summon = ObjectFields::End as FieldIndex + 0x2,
        Critter = ObjectFields::End as FieldIndex + 0x4,
        CharmedBy = ObjectFields::End as FieldIndex + 0x6,
        SummonedBy = ObjectFields::End as FieldIndex + 0x8,
        CreatedBy = ObjectFields::End as FieldIndex + 0xA,
        Target = ObjectFields::End as FieldIndex + 0xC,
        ChannelObject = ObjectFields::End as FieldIndex + 0xE,
        ChannelSpell = ObjectFields::End as FieldIndex + 0x10,
        /// Race, class, gender, and power type.
        Bytes0 = ObjectFields::End as FieldIndex + 0x11,
        Health = ObjectFields::End as FieldIndex + 0x12,
        Power1 = ObjectFields::End as FieldIndex + 0x13,
        Power2 = ObjectFields::End as FieldIndex + 0x14,
        Power3 = ObjectFields::End as FieldIndex + 0x15,
        Power4 = ObjectFields::End as FieldIndex + 0x16,
        Power5 = ObjectFields::End as FieldIndex + 0x17,
        Power6 = ObjectFields::End as FieldIndex + 0x18,
        Power7 = ObjectFields::End as FieldIndex + 0x19,
        MaxHealth = ObjectFields::End as FieldIndex + 0x1A,
        MaxPower1 = ObjectFields::End as FieldIndex + 0x1B,
        MaxPower2 = ObjectFields::End as FieldIndex + 0x1C,
        MaxPower3 = ObjectFields::End as FieldIndex + 0x1D,
        MaxPower4 = ObjectFields::End as FieldIndex + 0x1E,
        MaxPower5 = ObjectFields::End as FieldIndex + 0x1F,
        MaxPower6 = ObjectFields::End as FieldIndex + 0x20,
        MaxPower7 = ObjectFields::End as FieldIndex + 0x21,
        /// Seven `f32` values, one for each power type.
        PowerRegenFlatModifier = ObjectFields::End as FieldIndex + 0x22,
        /// Seven `f32` values, one for each power type.
        PowerRegenInterruptedFlatModifier = ObjectFields::End as FieldIndex + 0x29,
        Level = ObjectFields::End as FieldIndex + 0x30,
        FactionTemplate = ObjectFields::End as FieldIndex + 0x31,
        VirtualItemSlotId = ObjectFields::End as FieldIndex + 0x32,
        Flags = ObjectFields::End as FieldIndex + 0x35,
        Flags2 = ObjectFields::End as FieldIndex + 0x36,
        AuraState = ObjectFields::End as FieldIndex + 0x37,
        /// Main-hand and off-hand attack timings occupy two consecutive fields.
        BaseAttackTime = ObjectFields::End as FieldIndex + 0x38,
        OffhandAttackTime = ObjectFields::End as FieldIndex + 0x39,
        RangedAttackTime = ObjectFields::End as FieldIndex + 0x3A,
        BoundingRadius = ObjectFields::End as FieldIndex + 0x3B,
        CombatReach = ObjectFields::End as FieldIndex + 0x3C,
        DisplayId = ObjectFields::End as FieldIndex + 0x3D,
        NativeDisplayId = ObjectFields::End as FieldIndex + 0x3E,
        MountDisplayId = ObjectFields::End as FieldIndex + 0x3F,
        MinDamage = ObjectFields::End as FieldIndex + 0x40,
        MaxDamage = ObjectFields::End as FieldIndex + 0x41,
        MinOffhandDamage = ObjectFields::End as FieldIndex + 0x42,
        MaxOffhandDamage = ObjectFields::End as FieldIndex + 0x43,
        /// Stand state, visibility flags, and animation tier.
        Bytes1 = ObjectFields::End as FieldIndex + 0x44,
        PetNumber = ObjectFields::End as FieldIndex + 0x45,
        PetNameTimestamp = ObjectFields::End as FieldIndex + 0x46,
        PetExperience = ObjectFields::End as FieldIndex + 0x47,
        PetNextLevelExp = ObjectFields::End as FieldIndex + 0x48,
        DynamicFlags = ObjectFields::End as FieldIndex + 0x49,
        ModCastSpeed = ObjectFields::End as FieldIndex + 0x4A,
        CreatedBySpell = ObjectFields::End as FieldIndex + 0x4B,
        /// Vendor, quest-giver, flight-master, and related flags.
        NpcFlags = ObjectFields::End as FieldIndex + 0x4C,
        NpcEmoteState = ObjectFields::End as FieldIndex + 0x4D,
        Stat0 = ObjectFields::End as FieldIndex + 0x4E,
        Stat1 = ObjectFields::End as FieldIndex + 0x4F,
        Stat2 = ObjectFields::End as FieldIndex + 0x50,
        Stat3 = ObjectFields::End as FieldIndex + 0x51,
        Stat4 = ObjectFields::End as FieldIndex + 0x52,
        PositiveStat0 = ObjectFields::End as FieldIndex + 0x53,
        PositiveStat1 = ObjectFields::End as FieldIndex + 0x54,
        PositiveStat2 = ObjectFields::End as FieldIndex + 0x55,
        PositiveStat3 = ObjectFields::End as FieldIndex + 0x56,
        PositiveStat4 = ObjectFields::End as FieldIndex + 0x57,
        NegativeStat0 = ObjectFields::End as FieldIndex + 0x58,
        NegativeStat1 = ObjectFields::End as FieldIndex + 0x59,
        NegativeStat2 = ObjectFields::End as FieldIndex + 0x5A,
        NegativeStat3 = ObjectFields::End as FieldIndex + 0x5B,
        NegativeStat4 = ObjectFields::End as FieldIndex + 0x5C,
        /// Seven `i32` resistance values.
        Resistances = ObjectFields::End as FieldIndex + 0x5D,
        /// Seven positive resistance modifiers.
        ResistanceBuffModsPositive = ObjectFields::End as FieldIndex + 0x64,
        /// Seven negative resistance modifiers.
        ResistanceBuffModsNegative = ObjectFields::End as FieldIndex + 0x6B,
        BaseMana = ObjectFields::End as FieldIndex + 0x72,
        BaseHealth = ObjectFields::End as FieldIndex + 0x73,
        Bytes2 = ObjectFields::End as FieldIndex + 0x74,
        AttackPower = ObjectFields::End as FieldIndex + 0x75,
        AttackPowerMods = ObjectFields::End as FieldIndex + 0x76,
        AttackPowerMultiplier = ObjectFields::End as FieldIndex + 0x77,
        RangedAttackPower = ObjectFields::End as FieldIndex + 0x78,
        RangedAttackPowerMods = ObjectFields::End as FieldIndex + 0x79,
        RangedAttackPowerMultiplier = ObjectFields::End as FieldIndex + 0x7A,
        MinRangedDamage = ObjectFields::End as FieldIndex + 0x7B,
        MaxRangedDamage = ObjectFields::End as FieldIndex + 0x7C,
        /// Seven `i32` cost modifiers.
        PowerCostModifier = ObjectFields::End as FieldIndex + 0x7D,
        /// Seven `f32` cost multipliers.
        PowerCostMultiplier = ObjectFields::End as FieldIndex + 0x84,
        MaxHealthModifier = ObjectFields::End as FieldIndex + 0x8B,
        HoverHeight = ObjectFields::End as FieldIndex + 0x8C,
        Padding = ObjectFields::End as FieldIndex + 0x8D,
        End = ObjectFields::End as FieldIndex + 0x8E,
    }

    /// Update-field indices for players, extending [`UnitFields`].
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum PlayerFields {
        DuelArbitrator = UnitFields::End as FieldIndex,
        Flags = UnitFields::End as FieldIndex + 0x2,
        GuildId = UnitFields::End as FieldIndex + 0x3,
        GuildRank = UnitFields::End as FieldIndex + 0x4,
        /// Skin, face, hairstyle, and hair colour.
        Bytes = UnitFields::End as FieldIndex + 0x5,
        /// Facial hair and rest state.
        Bytes2 = UnitFields::End as FieldIndex + 0x6,
        /// Gender and arena faction.
        Bytes3 = UnitFields::End as FieldIndex + 0x7,
        DuelTeam = UnitFields::End as FieldIndex + 0x8,
        GuildTimestamp = UnitFields::End as FieldIndex + 0x9,
        /// Intentionally only marks the quest-log range; quest fields are
        /// outside the requested scope and are not individually exposed.
        QuestLogFirst = UnitFields::End as FieldIndex + 0xA,
        VisibleItemEntryFirst = UnitFields::End as FieldIndex + 0x87,
        VisibleItemEnchantmentFirst = UnitFields::End as FieldIndex + 0x88,
        ChosenTitle = UnitFields::End as FieldIndex + 0xAD,
        FakeInebriation = UnitFields::End as FieldIndex + 0xAE,
        Padding = UnitFields::End as FieldIndex + 0xAF,
        /// First 64-bit GUID among equipped and bag inventory slots.
        InventoryFirst = UnitFields::End as FieldIndex + 0xB0,
        /// First 64-bit GUID in the standard backpack.
        PackFirst = UnitFields::End as FieldIndex + 0xDE,
        BankFirst = UnitFields::End as FieldIndex + 0xFE,
        BankBagFirst = UnitFields::End as FieldIndex + 0x136,
        BuybackFirst = UnitFields::End as FieldIndex + 0x144,
        KeyringFirst = UnitFields::End as FieldIndex + 0x15C,
        CurrencyTokenFirst = UnitFields::End as FieldIndex + 0x19C,
        /// 64-bit GUID used for remote vision.
        Farsight = UnitFields::End as FieldIndex + 0x1DC,
        /// Three consecutive 64-bit masks of known titles.
        KnownTitlesFirst = UnitFields::End as FieldIndex + 0x1DE,
        KnownCurrencies = UnitFields::End as FieldIndex + 0x1E4,
        Experience = UnitFields::End as FieldIndex + 0x1E6,
        NextLevelExperience = UnitFields::End as FieldIndex + 0x1E7,
        /// Start of the protocol-defined skill-info block.
        SkillInfoFirst = UnitFields::End as FieldIndex + 0x1E8,
        CharacterPoints1 = UnitFields::End as FieldIndex + 0x368,
        CharacterPoints2 = UnitFields::End as FieldIndex + 0x369,
        TrackCreatures = UnitFields::End as FieldIndex + 0x36A,
        TrackResources = UnitFields::End as FieldIndex + 0x36B,
        BlockPercentage = UnitFields::End as FieldIndex + 0x36C,
        DodgePercentage = UnitFields::End as FieldIndex + 0x36D,
        ParryPercentage = UnitFields::End as FieldIndex + 0x36E,
        Expertise = UnitFields::End as FieldIndex + 0x36F,
        OffhandExpertise = UnitFields::End as FieldIndex + 0x370,
        CritPercentage = UnitFields::End as FieldIndex + 0x371,
        RangedCritPercentage = UnitFields::End as FieldIndex + 0x372,
        OffhandCritPercentage = UnitFields::End as FieldIndex + 0x373,
        /// Seven `f32` values, one per spell school.
        SpellCritPercentageFirst = UnitFields::End as FieldIndex + 0x374,
        ShieldBlock = UnitFields::End as FieldIndex + 0x37B,
        ShieldBlockCritPercentage = UnitFields::End as FieldIndex + 0x37C,
        /// 128 consecutive zone-exploration bitmasks.
        ExploredZonesFirst = UnitFields::End as FieldIndex + 0x37D,
        RestStateExperience = UnitFields::End as FieldIndex + 0x3FD,
        /// Copper, stored as a 64-bit value in two descriptor fields.
        Coinage = UnitFields::End as FieldIndex + 0x3FE,
        /// Seven signed damage modifiers, one per school.
        DamageDonePositiveFirst = UnitFields::End as FieldIndex + 0x3FF,
        DamageDoneNegativeFirst = UnitFields::End as FieldIndex + 0x406,
        /// Seven floating-point damage multipliers, one per school.
        DamageDonePctFirst = UnitFields::End as FieldIndex + 0x40D,
        HealingDonePositive = UnitFields::End as FieldIndex + 0x414,
        HealingPct = UnitFields::End as FieldIndex + 0x415,
        HealingDonePct = UnitFields::End as FieldIndex + 0x416,
        TargetResistanceModifier = UnitFields::End as FieldIndex + 0x417,
        TargetPhysicalResistanceModifier = UnitFields::End as FieldIndex + 0x418,
        /// Later player byte-pack; distinct from appearance [`Self::Bytes`].
        CombatBytes = UnitFields::End as FieldIndex + 0x419,
        AmmoId = UnitFields::End as FieldIndex + 0x41A,
        SelfResurrectionSpell = UnitFields::End as FieldIndex + 0x41B,
        PvpMedals = UnitFields::End as FieldIndex + 0x41C,
        BuybackPriceFirst = UnitFields::End as FieldIndex + 0x41D,
        BuybackTimestampFirst = UnitFields::End as FieldIndex + 0x429,
        Kills = UnitFields::End as FieldIndex + 0x435,
        TodayContribution = UnitFields::End as FieldIndex + 0x436,
        YesterdayContribution = UnitFields::End as FieldIndex + 0x437,
        LifetimeHonorableKills = UnitFields::End as FieldIndex + 0x438,
        /// Later player byte-pack; distinct from appearance [`Self::Bytes2`].
        PvpBytes = UnitFields::End as FieldIndex + 0x439,
        WatchedFactionIndex = UnitFields::End as FieldIndex + 0x43A,
        /// Twenty-five `i32` combat-rating values.
        CombatRatingFirst = UnitFields::End as FieldIndex + 0x43B,
        ArenaTeamInfoFirst = UnitFields::End as FieldIndex + 0x454,
        HonorCurrency = UnitFields::End as FieldIndex + 0x469,
        ArenaCurrency = UnitFields::End as FieldIndex + 0x46A,
        MaxLevel = UnitFields::End as FieldIndex + 0x46B,
        /// Intentionally only marks the daily-quest range.
        DailyQuestsFirst = UnitFields::End as FieldIndex + 0x46C,
        RuneRegenFirst = UnitFields::End as FieldIndex + 0x485,
        NoReagentCostFirst = UnitFields::End as FieldIndex + 0x489,
        GlyphSlotsFirst = UnitFields::End as FieldIndex + 0x48C,
        GlyphsFirst = UnitFields::End as FieldIndex + 0x492,
        GlyphsEnabled = UnitFields::End as FieldIndex + 0x498,
        PetSpellPower = UnitFields::End as FieldIndex + 0x499,
        End = UnitFields::End as FieldIndex + 0x49A,
    }

    /// Update-field indices for game objects.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum GameObjectFields {
        CreatedBy = ObjectFields::End as FieldIndex,
        DisplayId = ObjectFields::End as FieldIndex + 0x2,
        Flags = ObjectFields::End as FieldIndex + 0x3,
        /// Quaternion represented by four consecutive `f32` fields.
        ParentRotation = ObjectFields::End as FieldIndex + 0x4,
        Dynamic = ObjectFields::End as FieldIndex + 0x8,
        Faction = ObjectFields::End as FieldIndex + 0x9,
        Level = ObjectFields::End as FieldIndex + 0xA,
        Bytes1 = ObjectFields::End as FieldIndex + 0xB,
        End = ObjectFields::End as FieldIndex + 0xC,
    }

    /// Update-field indices for dynamic spell objects (persistent area spells,
    /// ground effects, and related client objects).
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum DynamicObjectFields {
        Caster = ObjectFields::End as FieldIndex,
        Bytes = ObjectFields::End as FieldIndex + 0x2,
        SpellId = ObjectFields::End as FieldIndex + 0x3,
        Radius = ObjectFields::End as FieldIndex + 0x4,
        CastTime = ObjectFields::End as FieldIndex + 0x5,
        End = ObjectFields::End as FieldIndex + 0x6,
    }

    /// Update-field indices for corpses.
    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum CorpseFields {
        Owner = ObjectFields::End as FieldIndex,
        Party = ObjectFields::End as FieldIndex + 0x2,
        DisplayId = ObjectFields::End as FieldIndex + 0x4,
        /// Nineteen consecutive item-entry IDs.
        ItemFirst = ObjectFields::End as FieldIndex + 0x5,
        Bytes1 = ObjectFields::End as FieldIndex + 0x18,
        Bytes2 = ObjectFields::End as FieldIndex + 0x19,
        Guild = ObjectFields::End as FieldIndex + 0x1A,
        Flags = ObjectFields::End as FieldIndex + 0x1B,
        DynamicFlags = ObjectFields::End as FieldIndex + 0x1C,
        Padding = ObjectFields::End as FieldIndex + 0x1D,
        End = ObjectFields::End as FieldIndex + 0x1E,
    }

    impl update_fields::Field for ObjectFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for ItemFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for ContainerFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for UnitFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for PlayerFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for GameObjectFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for DynamicObjectFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    impl update_fields::Field for CorpseFields {
        fn index(self) -> FieldIndex {
            self as FieldIndex
        }
    }

    /// Descriptor fields grouped under a discoverable namespace.
    pub mod descriptor_fields {
        pub use super::{
            ContainerFields, CorpseFields, DynamicObjectFields, GameObjectFields, ItemFields,
            ObjectFields, PlayerFields, UnitFields,
        };
    }

    /// Additional combat, casting, camera, and overlay data in one place.
    pub mod advanced_combat {
        use super::{Address, Offset, RemoteAddress};

        pub mod cooldown {
            use super::{Offset, RemoteAddress};

            /// Pointer to the spell-cooldown linked list.
            pub use super::super::memory::game_state::SPELL_COOLDOWN_PTR;

            pub const NEXT_OFFSET: Offset = 0x00;
            pub const SPELL_ID_OFFSET: Offset = 0x08;
            pub const ITEM_ID_OFFSET: Offset = 0x0C;
            pub const START_TIME_OFFSET: Offset = 0x10;
            pub const DURATION_OFFSET: Offset = 0x14;

            /// Node in the x86 client's spell-cooldown linked list.
            #[repr(C)]
            #[derive(Debug, Copy, Clone)]
            pub struct SpellCooldownEntry {
                /// Remote address of the next node, or zero.
                pub next: RemoteAddress,
                pub unknown: u32,
                pub spell_id: u32,
                pub item_id: u32,
                /// `GetTickCount()` when the cooldown started.
                pub start_time: u32,
                /// Cooldown duration in milliseconds.
                pub duration: u32,
            }
        }

        pub mod hooks {
            use super::{Address, Offset};

            /// The two direct method-entry targets required by the injected
            /// D3D9 runtime. The early-D3D9 capture obtains these from the
            /// live `IDirect3DDevice9` vtable; they are never static game
            /// offsets or module-relative guesses.
            #[repr(C)]
            #[derive(Debug, Copy, Clone, PartialEq, Eq)]
            pub struct Direct3d9Targets {
                pub end_scene: Address,
                pub reset: Address,
            }

            impl Direct3d9Targets {
                #[must_use]
                pub const fn is_valid(self) -> bool {
                    self.end_scene != 0 && self.reset != 0 && self.end_scene != self.reset
                }
            }

            /// Loader-only path for resolving the active client's D3D9 device.
            ///
            /// The injected runtime never reads this chain. The DEV loader
            /// resolves the live method entries once and passes only
            /// [`Direct3d9Targets`] across the bootstrap boundary.
            pub mod loader_discovery {
                use super::{Address, Offset};

                /// Static pointer to the client's `GxDevice` owner.
                pub const GX_DEVICE: Address = 0x00C5_DF88;
                /// `IDirect3DDevice9*` member inside the active `GxDevice`.
                pub const DEVICE: Offset = 0x3C;

                pub const RESET_INDEX: u32 = 16;
                pub const PRESENT_INDEX: u32 = 17;
                pub const END_SCENE_INDEX: u32 = 42;
                pub const POINTER_SIZE: u32 = 4;
            }
        }

        pub mod casting {
            use super::{Offset, RemoteAddress};

            /// Current cast spell ID in the unit base.
            pub const CURRENT_SPELL_ID: Offset = 0xC89;
            /// Pointer to current cast information in the unit base.
            pub const SPELL_CAST_STRUCT_PTR: Offset = 0xA8;
            pub const SPELL_ID_OFFSET: Offset = 0x04;
            pub const START_TIME_OFFSET: Offset = 0x08;
            pub const END_TIME_OFFSET: Offset = 0x0C;
            pub const IS_CHANNELING_OFFSET: Offset = 0x10;

            /// Partial cast state read from the x86 game process.
            #[repr(C)]
            #[derive(Debug, Copy, Clone)]
            pub struct SpellCastInfo {
                pub unknown: RemoteAddress,
                pub spell_id: u32,
                /// `GetTickCount()` when the cast started.
                pub cast_start_time: u32,
                /// `GetTickCount()` when the cast ends.
                pub cast_end_time: u32,
                pub is_channeling: u8,
            }
        }

        pub mod camera {
            use super::{Address, Offset};

            /// `CGWorldFrame*` for the shipped 3.3.5a build-12340 client.
            ///
            /// This address and the member offsets below were verified against
            /// the exact `Wow.exe` image (SHA-256
            /// `07c51ead92b0d420247fb8100cd2fc1f0c33117ca4f4743a557cfb1cbdede0bc`).
            /// It is a pointer slot, so it is zero before the world frame is
            /// constructed (login, character selection, and loading screens).
            pub const CURRENT_WORLD_FRAME: Address = 0x00B7_436C;
            /// `CGCamera*` member of `CGWorldFrame`.
            pub const CAMERA_OFFSET: Offset = 0x7E20;
            /// Eye-position vector (`Vector3`).
            pub const EYE_POSITION_OFFSET: Offset = 0x08;
            /// Unit vector pointing along the camera's view direction.
            pub const FORWARD_BASIS_OFFSET: Offset = 0x14;
            /// Unit vector pointing to the camera's left.
            pub const LEFT_BASIS_OFFSET: Offset = 0x20;
            /// Roll angle in radians. It is retained for a full future
            /// projection path but is normally zero in the game camera.
            pub const ROLL_OFFSET: Offset = 0x2C;
            /// Yaw angle in radians.
            pub const YAW_OFFSET: Offset = 0x30;
            /// Pitch angle in radians.
            pub const PITCH_OFFSET: Offset = 0x34;
            /// Field of view in radians.
            pub const FOV_OFFSET: Offset = 0x40;

            #[repr(C)]
            #[derive(Debug, Copy, Clone, PartialEq)]
            pub struct Vector3 {
                pub x: f32,
                pub y: f32,
                pub z: f32,
            }

            #[repr(C)]
            #[derive(Debug, Copy, Clone, PartialEq)]
            pub struct CameraStateLayout {
                pub unknown_00_to_07: [u8; 0x8],
                /// Eye position at offset `0x08`.
                pub position: Vector3,
                /// View direction at offset `0x14`.
                pub forward: Vector3,
                /// Camera-left direction at offset `0x20`.
                pub left: Vector3,
                pub roll: f32,
                pub yaw: f32,
                pub pitch: f32,
                pub unknown_38_to_3f: [u8; 0x8],
                pub field_of_view: f32,
            }
        }

        pub mod auras {
            use super::Offset;

            /// Number of active auras (`u32`).
            pub const BASE_AURA_COUNT: Offset = 0xDD0;
            /// Inline array containing the first 40 aura entries.
            pub const BASE_AURA_ARRAY: Offset = 0xC50;
            /// Dynamic `AuraEntry` array for additional auras.
            pub const DYNAMIC_AURA_POINTER: Offset = 0xDD8;

            /// Size of one remote `AuraEntry` in the x86 client.
            pub const ENTRY_STRIDE: Offset = 0x18;
            pub const CREATOR_GUID_OFFSET: Offset = 0x00;
            pub const SPELL_ID_OFFSET: Offset = 0x08;
            pub const FLAGS_OFFSET: Offset = 0x0C;
            pub const LEVEL_OFFSET: Offset = 0x0D;
            pub const STACK_COUNT_OFFSET: Offset = 0x0E;
            pub const DURATION_OFFSET: Offset = 0x10;
            pub const END_TIME_OFFSET: Offset = 0x14;
        }

        pub mod game_objects {
            use super::Offset;

            /// State byte: zero is ready, one is active/opened.
            pub const GAMEOBJECT_STATE: Offset = 0x1C;
        }

        pub mod threat {
            #[repr(u32)]
            #[derive(Debug, Copy, Clone, PartialEq, Eq)]
            pub enum ThreatState {
                Low = 0,
                HigherThanTank = 1,
                Highest = 2,
                Tanking = 3,
            }
        }

        /// General combat state useful to class rotations.
        pub mod state {
            use super::Address;

            /// Rogue/druid combo points (`u8`).
            pub const COMBO_POINTS: Address = 0x00BD_084D;
            pub const MOUSEOVER_GUID: Address = 0x00BD_07B0;
            pub const FOCUS_GUID: Address = 0x00BD_0780;
            /// Non-zero while the local player is auto-attacking.
            pub const IS_AUTO_ATTACKING: Address = 0x00C7_9D50;
        }
    }

    /// DBC spell-storage layout.
    pub mod spell_dbc {
        use super::Address;

        /// Pointer to the client spell DBC storage.
        ///
        /// This legacy static address is still a candidate for this exact
        /// executable. The record layout below is deliberately separate from
        /// it: the DBC record is build/protocol data, whereas the location of
        /// the client's DBC store is an image-specific implementation detail.
        pub const SPELL_STORE: Address = 0x00C0_D780;

        /// Byte offsets inside a packed 3.3.5a build-12340 `Spell.dbc` record.
        ///
        /// These are data-record byte offsets, not object descriptor indices.
        /// They replace the previous `SpellEntry` approximation, whose fields
        /// were shifted and whose `[RemoteAddress; 16]` strings were incorrect
        /// for the x86 3.3.5 client. Read fields through a typed remote-memory
        /// reader; do not transmute a remote pointer into a Rust reference.
        pub mod entry {
            use super::super::Offset;

            pub const SIZE: Offset = 0x2A8;
            pub const EFFECT_COUNT: Offset = 3;
            pub const REAGENT_COUNT: Offset = 8;
            pub const TOTEM_COUNT: Offset = 2;
            pub const LOCALIZED_STRING_COUNT: Offset = 4;

            pub const ID: Offset = 0x000;
            pub const CATEGORY: Offset = 0x004;
            pub const DISPEL_TYPE: Offset = 0x008;
            pub const MECHANIC: Offset = 0x00C;
            pub const ATTRIBUTES: Offset = 0x010;
            pub const ATTRIBUTES_EX1: Offset = 0x014;
            pub const ATTRIBUTES_EX2: Offset = 0x018;
            pub const ATTRIBUTES_EX3: Offset = 0x01C;
            pub const ATTRIBUTES_EX4: Offset = 0x020;
            pub const ATTRIBUTES_EX5: Offset = 0x024;
            pub const ATTRIBUTES_EX6: Offset = 0x028;
            pub const ATTRIBUTES_EX7: Offset = 0x02C;
            pub const STANCES: Offset = 0x030; // u64
            pub const STANCES_NOT: Offset = 0x038; // u64
            pub const TARGETS: Offset = 0x040;
            pub const TARGET_CREATURE_TYPE: Offset = 0x044;
            pub const REQUIRES_SPELL_FOCUS: Offset = 0x048;
            pub const FACING_CASTER_FLAGS: Offset = 0x04C;
            pub const CASTER_AURA_STATE: Offset = 0x050;
            pub const TARGET_AURA_STATE: Offset = 0x054;
            pub const CASTER_AURA_STATE_NOT: Offset = 0x058;
            pub const TARGET_AURA_STATE_NOT: Offset = 0x05C;
            pub const CASTER_AURA_SPELL: Offset = 0x060;
            pub const TARGET_AURA_SPELL: Offset = 0x064;
            pub const EXCLUDE_CASTER_AURA_SPELL: Offset = 0x068;
            pub const EXCLUDE_TARGET_AURA_SPELL: Offset = 0x06C;
            pub const CAST_TIME_INDEX: Offset = 0x070;
            pub const RECOVERY_TIME: Offset = 0x074;
            pub const CATEGORY_RECOVERY_TIME: Offset = 0x078;
            pub const INTERRUPT_FLAGS: Offset = 0x07C;
            pub const AURA_INTERRUPT_FLAGS: Offset = 0x080;
            pub const CHANNEL_INTERRUPT_FLAGS: Offset = 0x084;
            pub const PROC_FLAGS: Offset = 0x088;
            pub const PROC_CHANCE: Offset = 0x08C;
            pub const PROC_CHARGES: Offset = 0x090;
            pub const MAX_LEVEL: Offset = 0x094;
            pub const BASE_LEVEL: Offset = 0x098;
            pub const SPELL_LEVEL: Offset = 0x09C;
            pub const DURATION_INDEX: Offset = 0x0A0;
            pub const POWER_TYPE: Offset = 0x0A4;
            pub const MANA_COST: Offset = 0x0A8;
            pub const MANA_COST_PER_LEVEL: Offset = 0x0AC;
            pub const MANA_PER_SECOND: Offset = 0x0B0;
            pub const MANA_PER_SECOND_PER_LEVEL: Offset = 0x0B4;
            pub const RANGE_INDEX: Offset = 0x0B8;
            pub const SPEED: Offset = 0x0BC; // f32
            pub const MODAL_NEXT_SPELL: Offset = 0x0C0;
            pub const CUMULATIVE_AURA: Offset = 0x0C4;
            pub const TOTEM_FIRST: Offset = 0x0C8;
            pub const REAGENT_FIRST: Offset = 0x0D0;
            pub const REAGENT_COUNT_FIRST: Offset = 0x0F0;
            pub const EQUIPPED_ITEM_CLASS: Offset = 0x110;
            pub const EQUIPPED_ITEM_SUBCLASS_MASK: Offset = 0x114;
            pub const EQUIPPED_ITEM_INVENTORY_TYPE_MASK: Offset = 0x118;
            pub const EFFECT_FIRST: Offset = 0x11C;
            pub const EFFECT_DIE_SIDES_FIRST: Offset = 0x128;
            pub const EFFECT_REAL_POINTS_PER_LEVEL_FIRST: Offset = 0x134; // f32
            /// Raw DBC base points. Spell semantics can apply a +1 adjustment.
            pub const EFFECT_BASE_POINTS_FIRST: Offset = 0x140;
            pub const EFFECT_MECHANIC_FIRST: Offset = 0x14C;
            pub const EFFECT_IMPLICIT_TARGET_A_FIRST: Offset = 0x158;
            pub const EFFECT_IMPLICIT_TARGET_B_FIRST: Offset = 0x164;
            pub const EFFECT_RADIUS_INDEX_FIRST: Offset = 0x170;
            pub const EFFECT_AURA_FIRST: Offset = 0x17C;
            pub const EFFECT_AMPLITUDE_FIRST: Offset = 0x188; // f32
            pub const EFFECT_MULTIPLE_VALUE_FIRST: Offset = 0x194; // f32
            pub const EFFECT_CHAIN_TARGETS_FIRST: Offset = 0x1A0;
            pub const EFFECT_ITEM_TYPE_FIRST: Offset = 0x1AC;
            pub const EFFECT_MISC_VALUE_FIRST: Offset = 0x1B8;
            pub const EFFECT_MISC_VALUE_B_FIRST: Offset = 0x1C4;
            pub const EFFECT_TRIGGER_SPELL_FIRST: Offset = 0x1D0;
            pub const EFFECT_POINTS_PER_COMBO_FIRST: Offset = 0x1DC; // f32
            pub const EFFECT_SPELL_CLASS_MASK_A_FIRST: Offset = 0x1E8;
            pub const EFFECT_SPELL_CLASS_MASK_B_FIRST: Offset = 0x1F4;
            pub const EFFECT_SPELL_CLASS_MASK_C_FIRST: Offset = 0x200;
            pub const SPELL_VISUAL_FIRST: Offset = 0x20C;
            pub const SPELL_ICON_ID: Offset = 0x214;
            pub const ACTIVE_ICON_ID: Offset = 0x218;
            pub const PRIORITY: Offset = 0x21C;
            pub const NAME_PTR: Offset = 0x220;
            pub const RANK_PTR: Offset = 0x224;
            pub const DESCRIPTION_PTR: Offset = 0x228;
            pub const AURA_DESCRIPTION_PTR: Offset = 0x22C;
            pub const MANA_COST_PCT: Offset = 0x230;
            pub const START_RECOVERY_CATEGORY: Offset = 0x234;
            pub const START_RECOVERY_TIME: Offset = 0x238;
            pub const MAX_TARGET_LEVEL: Offset = 0x23C;
            pub const SPELL_FAMILY: Offset = 0x240;
            pub const SPELL_FAMILY_FLAGS_FIRST: Offset = 0x244;
            pub const MAX_AFFECTED_TARGETS: Offset = 0x250;
            pub const DAMAGE_CLASS: Offset = 0x254;
            pub const PREVENTION_TYPE: Offset = 0x258;
            pub const STANCE_BAR_ORDER: Offset = 0x25C;
            pub const EFFECT_DAMAGE_MULTIPLIER_FIRST: Offset = 0x260; // f32
            pub const MIN_FACTION_ID: Offset = 0x26C;
            pub const MIN_REPUTATION: Offset = 0x270;
            pub const REQUIRED_AURA_VISION: Offset = 0x274;
            pub const REQUIRED_TOTEM_CATEGORY_FIRST: Offset = 0x278;
            pub const REQUIRED_AREA_GROUP: Offset = 0x280;
            pub const SCHOOL_MASK: Offset = 0x284;
            pub const RUNE_COST_ID: Offset = 0x288;
            pub const SPELL_MISSILE_ID: Offset = 0x28C;
            pub const POWER_DISPLAY_ID: Offset = 0x290;
            pub const EFFECT_BONUS_MULTIPLIER_FIRST: Offset = 0x294; // f32
            pub const DESCRIPTION_VARIABLES_ID: Offset = 0x2A0;
            pub const DIFFICULTY_ID: Offset = 0x2A4;
        }
    }

    /// On-disk world-asset layouts for the 3.3.5 client.
    ///
    /// This is deliberately separate from remote-process memory. Static
    /// terrain, WMO buildings, and M2 trees/rocks are streamed from MPQ data;
    /// they are not ordinary entries in the object-manager linked list. These
    /// constants let a future *read-only asset reader* identify placements
    /// without inventing a runtime pointer for every static object.
    pub mod world_assets {
        /// `GameObjectDisplayInfo.dbc` record fields. A runtime GameObject's
        /// `ObjectFields::Entry` resolves through server data to a display ID;
        /// the display record supplies the model path and its coarse bounds.
        pub mod game_object_display_info {
            pub const FIELD_COUNT: u32 = 19;
            pub const ID: u32 = 0;
            /// String-table offset in the raw DBC, or client string pointer
            /// after the client DBC loader has unpacked it.
            pub const MODEL_NAME: u32 = 1;
            pub const SOUND_FIRST: u32 = 2;
            pub const SOUND_COUNT: u32 = 10;
            pub const GEOBOX_MIN_X: u32 = 12;
            pub const GEOBOX_MIN_Y: u32 = 13;
            pub const GEOBOX_MIN_Z: u32 = 14;
            pub const GEOBOX_MAX_X: u32 = 15;
            pub const GEOBOX_MAX_Y: u32 = 16;
            pub const GEOBOX_MAX_Z: u32 = 17;
            pub const OBJECT_EFFECT_PACKAGE_ID: u32 = 18;
        }

        /// Legacy single-file ADT layout used by WotLK 3.3.5a.
        pub mod adt {
            /// An ADT map tile is 16 by 16 terrain chunks.
            pub const CHUNKS_PER_TILE_AXIS: u32 = 16;
            /// World-space width/height of one map tile, in client units.
            pub const TILE_SIZE: f32 = 533.333_3;
            /// World-space width/height of one MCNK terrain chunk.
            pub const CHUNK_SIZE: f32 = 33.333_332;

            /// FourCC identifiers. Read the file's chunk headers rather than
            /// assuming a fixed file offset: chunk order/size varies by tile.
            pub mod chunks {
                pub const MVER: [u8; 4] = *b"MVER";
                pub const MHDR: [u8; 4] = *b"MHDR";
                pub const MCIN: [u8; 4] = *b"MCIN";
                pub const MCNK: [u8; 4] = *b"MCNK";
                pub const MCVT: [u8; 4] = *b"MCVT";
                pub const MCNR: [u8; 4] = *b"MCNR";
                pub const MCLY: [u8; 4] = *b"MCLY";
                pub const MCRF: [u8; 4] = *b"MCRF";
                pub const MCCV: [u8; 4] = *b"MCCV";
                pub const MCSH: [u8; 4] = *b"MCSH";
                pub const MH2O: [u8; 4] = *b"MH2O";
                pub const MMDX: [u8; 4] = *b"MMDX";
                pub const MMID: [u8; 4] = *b"MMID";
                pub const MWMO: [u8; 4] = *b"MWMO";
                pub const MWID: [u8; 4] = *b"MWID";
                pub const MDDF: [u8; 4] = *b"MDDF";
                pub const MODF: [u8; 4] = *b"MODF";
            }

            /// Every IFF chunk starts with a four-byte tag and four-byte
            /// little-endian payload size. Offsets in MCNK point from the
            /// start of the *entire* MCNK chunk (including this header).
            pub const CHUNK_HEADER_SIZE: u32 = 8;

            /// MCIN holds one entry for each of the 16 × 16 MCNK chunks.
            pub mod mcin {
                use super::super::super::Offset;

                pub const ENTRY_COUNT: u32 = 256;
                pub const ENTRY_SIZE: Offset = 0x10;
                /// Absolute ADT-file byte offset of the MCNK chunk header.
                pub const MCNK_OFFSET: Offset = 0x00;
                pub const MCNK_SIZE: Offset = 0x04;
                pub const FLAGS: Offset = 0x08;
                pub const ASYNC_ID: Offset = 0x0C;
            }

            /// The fixed 128-byte WotLK MCNK payload header. It describes one
            /// 33⅓-yard terrain chunk and points to its variable subchunks.
            pub mod mcnk {
                use super::super::super::Offset;

                pub const HEADER_SIZE: Offset = 0x80;
                pub const FLAGS: Offset = 0x00;
                pub const INDEX_X: Offset = 0x04;
                pub const INDEX_Y: Offset = 0x08;
                pub const LAYER_COUNT: Offset = 0x0C;
                pub const DOODAD_REF_COUNT: Offset = 0x10;
                pub const MCVT_OFFSET: Offset = 0x14;
                pub const MCNR_OFFSET: Offset = 0x18;
                pub const MCLY_OFFSET: Offset = 0x1C;
                pub const MCRF_OFFSET: Offset = 0x20;
                pub const MCAL_OFFSET: Offset = 0x24;
                pub const MCAL_SIZE: Offset = 0x28;
                pub const MCSH_OFFSET: Offset = 0x2C;
                pub const MCSH_SIZE: Offset = 0x30;
                pub const AREA_ID: Offset = 0x34;
                pub const WMO_REF_COUNT: Offset = 0x38;
                /// Low-resolution 4 × 4 hole mask. A set bit is a hole.
                pub const HOLES: Offset = 0x3C;
                pub const LOW_RES_TEXTURE_MAP: Offset = 0x40;
                pub const PRED_TEX: Offset = 0x50;
                pub const NO_EFFECT_DOODAD: Offset = 0x54;
                pub const MCSE_OFFSET: Offset = 0x58;
                pub const SOUND_EMITTER_COUNT: Offset = 0x5C;
                /// Legacy MCLQ only; 3.3.5 liquid is in root MH2O.
                pub const LEGACY_LIQUID_OFFSET: Offset = 0x60;
                pub const LEGACY_LIQUID_SIZE: Offset = 0x64;
                /// Stored as `z, x, y` in the WotLK ADT file. Consumers
                /// must convert it to Veyr's world convention `(x, y, z)`.
                pub const FILE_POSITION_Z: Offset = 0x68;
                pub const FILE_POSITION_X: Offset = 0x6C;
                pub const FILE_POSITION_Y: Offset = 0x70;
                pub const MCCV_OFFSET: Offset = 0x74;
                pub const MCLV_OFFSET: Offset = 0x78;
                pub const UNUSED: Offset = 0x7C;

                pub const HAS_SHADOW: u32 = 0x0001;
                pub const IMPASSABLE: u32 = 0x0002;
                pub const HAS_MCCV: u32 = 0x0040;
            }

            /// `MCVT` terrain elevation data. Values are `f32` height deltas
            /// from the parent MCNK position, arranged as alternating 9- and
            /// 8-vertex rows (81 outer + 64 inner control vertices).
            pub mod mcvt {
                pub const OUTER_VERTEX_COUNT: u32 = 81;
                pub const INNER_VERTEX_COUNT: u32 = 64;
                pub const VERTEX_COUNT: u32 = OUTER_VERTEX_COUNT + INNER_VERTEX_COUNT;
                pub const HEIGHT_SIZE: u32 = 4;
                pub const PAYLOAD_SIZE: u32 = VERTEX_COUNT * HEIGHT_SIZE;
            }

            /// WotLK water root: 16 × 16 entries, one per terrain chunk.
            /// Individual liquid layers remain variable-sized and must be
            /// bounds-checked by the future asset reader.
            pub mod mh2o {
                use super::super::super::Offset;

                pub const ENTRY_COUNT: u32 = 256;
                pub const ENTRY_SIZE: Offset = 0x0C;
                pub const LAYERS_OFFSET: Offset = 0x00;
                pub const LAYER_COUNT: Offset = 0x04;
                pub const ATTRIBUTES_OFFSET: Offset = 0x08;
            }

            /// M2 doodad placement (`MDDF`): trees, rocks, crates, lamps,
            /// and most non-building static scene models.
            pub mod mddf {
                use super::super::super::Offset;

                pub const SIZE: Offset = 0x24;
                pub const NAME_ID: Offset = 0x00;
                pub const UNIQUE_ID: Offset = 0x04;
                pub const POSITION_X: Offset = 0x08;
                pub const POSITION_Y: Offset = 0x0C;
                pub const POSITION_Z: Offset = 0x10;
                pub const ROTATION_X: Offset = 0x14;
                pub const ROTATION_Y: Offset = 0x18;
                pub const ROTATION_Z: Offset = 0x1C;
                /// `u16`, where 1024 is normal scale.
                pub const SCALE: Offset = 0x20;
                /// `u16` placement flags.
                pub const FLAGS: Offset = 0x22;
            }

            /// WMO placement (`MODF`): buildings, large structures, interiors,
            /// and other World Map Objects. Bounding extents are coarse; exact
            /// obstruction requires the referenced WMO's collision geometry.
            pub mod modf {
                use super::super::super::Offset;

                pub const SIZE: Offset = 0x40;
                pub const NAME_ID: Offset = 0x00;
                pub const UNIQUE_ID: Offset = 0x04;
                pub const POSITION_X: Offset = 0x08;
                pub const POSITION_Y: Offset = 0x0C;
                pub const POSITION_Z: Offset = 0x10;
                pub const ROTATION_X: Offset = 0x14;
                pub const ROTATION_Y: Offset = 0x18;
                pub const ROTATION_Z: Offset = 0x1C;
                pub const EXTENTS_MIN_X: Offset = 0x20;
                pub const EXTENTS_MIN_Y: Offset = 0x24;
                pub const EXTENTS_MIN_Z: Offset = 0x28;
                pub const EXTENTS_MAX_X: Offset = 0x2C;
                pub const EXTENTS_MAX_Y: Offset = 0x30;
                pub const EXTENTS_MAX_Z: Offset = 0x34;
                pub const FLAGS: Offset = 0x38; // u16
                pub const DOODAD_SET: Offset = 0x3A; // u16
                pub const NAME_SET: Offset = 0x3C; // u16
                /// Padding in WotLK 3.3.5 ADT `MODF`; it is not a scale.
                pub const PADDING: Offset = 0x3E;
            }
        }

        /// WotLK `MD20` M2 model fields that are specifically relevant to
        /// collision. M2 files are versioned: validate the header version
        /// before consuming these file offsets.
        pub mod m2 {
            use super::super::Offset;

            pub const MAGIC: [u8; 4] = *b"MD20";
            pub const COLLISION_INDEX_COUNT: Offset = 0xEC;
            pub const COLLISION_INDEX_OFFSET: Offset = 0xF0;
            pub const COLLISION_VERTEX_COUNT: Offset = 0xF4;
            pub const COLLISION_VERTEX_OFFSET: Offset = 0xF8;
            pub const COLLISION_NORMAL_COUNT: Offset = 0xFC;
            pub const COLLISION_NORMAL_OFFSET: Offset = 0x100;
            pub const COLLISION_INDEX_SIZE: u32 = 2; // u16
            pub const COLLISION_VERTEX_SIZE: u32 = 12; // [f32; 3]
            pub const INDICES_PER_TRIANGLE: u32 = 3;
        }

        /// WMO group geometry and acceleration data. A `MODF` placement
        /// references a WMO root, which in turn has one or more group files.
        /// These data are enough for a future offline raycaster to recover
        /// building/cave collision without pretending that WMO meshes are
        /// ordinary ObjectManager entities.
        pub mod wmo_group {
            pub mod chunks {
                pub const MOGP: [u8; 4] = *b"MOGP";
                pub const MOPY: [u8; 4] = *b"MOPY";
                pub const MOVI: [u8; 4] = *b"MOVI";
                pub const MOVT: [u8; 4] = *b"MOVT";
                pub const MOBN: [u8; 4] = *b"MOBN";
                pub const MOBR: [u8; 4] = *b"MOBR";
            }

            pub const VERTEX_SIZE: u32 = 12; // [f32; 3]
            pub const INDEX_SIZE: u32 = 2; // u16
            pub const INDICES_PER_TRIANGLE: u32 = 3;

            /// `MOPY`: one flags/material pair per triangle.
            pub mod mopy {
                pub const ENTRY_SIZE: u32 = 2;
                pub const COLLISION: u8 = 0x08;
                pub const RENDER: u8 = 0x20;
                /// Material `0xFF` marks collision-only geometry.
                pub const COLLISION_ONLY_MATERIAL: u8 = 0xFF;
            }

            /// `MOBN`: WMO BSP nodes used to cull and accelerate collision.
            pub mod mobn {
                use super::super::super::Offset;

                pub const ENTRY_SIZE: Offset = 0x10;
                pub const PLANE_TYPE: Offset = 0x00;
                pub const NEGATIVE_CHILD: Offset = 0x02;
                pub const POSITIVE_CHILD: Offset = 0x04;
                pub const FACE_COUNT: Offset = 0x06;
                pub const FIRST_FACE: Offset = 0x08;
                pub const PLANE_DISTANCE: Offset = 0x0C;
                pub const LEAF: u16 = 4;
            }
        }
    }

    /// Fixed-size aura entry stored in the client process.
    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct AuraEntry {
        pub creator_guid: u64,
        pub spell_id: u32,
        pub flags: u8,
        pub level: u8,
        pub stack_count: u8,
        pub unknown: u8,
        /// Total aura duration in milliseconds.
        pub duration: i32,
        /// `GetTickCount()` value at which the aura ends.
        pub end_time: i32,
    }

    #[cfg(test)]
    mod tests {
        use super::{update_fields, UnitFields};

        #[test]
        fn update_fields_are_converted_from_indices_to_bytes() {
            let descriptor_array = 0x1000;

            assert_eq!(update_fields::byte_offset_of(UnitFields::Health), 0x60);
            assert_eq!(
                update_fields::address_of(descriptor_array, UnitFields::Health),
                0x1060,
            );
        }
    }
}

pub use self::layout::*;

/// Internal developer API built on top of this memory map.
pub mod api;
