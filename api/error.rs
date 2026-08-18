use super::super::RemoteAddress;

/// Errors produced while resolving or reading client-side data.
#[derive(Debug)]
pub enum ApiError<E> {
    /// The underlying process-memory backend failed.
    Memory(E),
    /// A required pointer was null.
    NullPointer { context: &'static str },
    /// Adding an offset to a remote address would overflow the x86 address space.
    AddressOverflow { base: RemoteAddress, offset: u32 },
    /// The object-list traversal encountered an address twice.
    ObjectListCycle { address: RemoteAddress },
    /// The spell-cooldown traversal encountered an address twice.
    CooldownListCycle { address: RemoteAddress },
    /// An object carried a type value not known to this client map.
    UnknownObjectType { address: RemoteAddress, value: u32 },
    /// An operation was requested for an incompatible object kind.
    UnexpectedObjectType {
        address: RemoteAddress,
        operation: &'static str,
    },
}

/// Result returned by internal developer-API operations.
pub type ApiResult<T, E> = Result<T, ApiError<E>>;
