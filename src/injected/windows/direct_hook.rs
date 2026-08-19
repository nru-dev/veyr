use core::ffi::c_void;
use core::mem::size_of;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::offsets::{api::Memory, RemoteAddress};

use super::{d3d9_hook::EndSceneHookError, LocalProcessMemory, LocalProcessMemoryError};

const JMP_SIZE: usize = 5;
const MAX_INSTRUCTION_SIZE: usize = 15;
const MAX_STOLEN_BYTES: usize = 32;
const TRAMPOLINE_SIZE: usize = MAX_STOLEN_BYTES + JMP_SIZE;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;

/// A direct entry patch together with its copied original prologue.
pub(crate) struct DirectHook {
    target: RemoteAddress,
    original: [u8; MAX_STOLEN_BYTES],
    stolen_len: usize,
    trampoline: RemoteAddress,
    active: bool,
}

impl DirectHook {
    /// Copies complete non-relative x86 instructions into a trampoline for the temporary capture/OpenGL path and
    /// atomically coordinates the target patch with all other client threads.
    pub(crate) unsafe fn install(
        target: RemoteAddress,
        replacement: RemoteAddress,
        original_slot: &AtomicU32,
    ) -> Result<Self, EndSceneHookError> {
        if target == 0 {
            return Err(EndSceneHookError::NullTarget);
        }
        if target == replacement {
            return Err(EndSceneHookError::ReplacementIsOriginal);
        }

        let (original, stolen_len) = read_stolen_instructions(target)?;
        let trampoline = allocate_trampoline()?;

        let mut trampoline_bytes = [0_u8; TRAMPOLINE_SIZE];
        trampoline_bytes[..stolen_len].copy_from_slice(&original[..stolen_len]);
        write_relative_jump(
            &mut trampoline_bytes[stolen_len..stolen_len + JMP_SIZE],
            trampoline.checked_add(stolen_len as u32).ok_or(
                EndSceneHookError::AddressOverflow {
                    address: trampoline,
                    offset: stolen_len as u32,
                },
            )?,
            target
                .checked_add(stolen_len as u32)
                .ok_or(EndSceneHookError::AddressOverflow {
                    address: target,
                    offset: stolen_len as u32,
                })?,
        )?;
        write_process_bytes(trampoline, &trampoline_bytes[..stolen_len + JMP_SIZE])?;
        flush_instruction_cache(trampoline, stolen_len + JMP_SIZE)?;

        let mut patch = [0x90_u8; MAX_STOLEN_BYTES];
        write_relative_jump(&mut patch[..JMP_SIZE], target, replacement)?;

        // A five-byte jump is not an atomic write. Suspending every other
        // client thread prevents a renderer thread from executing a partially
        // written instruction stream while this direct patch is changed.
        let _threads = SuspendedThreads::acquire()?;
        // Publishing the trampoline while every other thread is stopped means
        // the callback can never observe a freshly installed jump together
        // with a zero original address.
        original_slot.store(trampoline, Ordering::Release);
        write_protected_code(target, &patch[..stolen_len])?;

        Ok(Self {
            target,
            original,
            stolen_len,
            trampoline,
            active: true,
        })
    }

    pub(crate) fn uninstall(&mut self) -> Result<(), EndSceneHookError> {
        if !self.active {
            return Ok(());
        }

        let _threads = SuspendedThreads::acquire()?;
        write_protected_code(self.target, &self.original[..self.stolen_len])?;
        self.active = false;

        // Keep the tiny trampoline allocated. A suspended thread can have
        // already jumped into it when stop begins; freeing it before that
        // thread resumes would turn a clean unhook into an instruction-fetch
        // fault. The trampoline contains only copied D3D9 code and a jump back
        // to D3D9, so it remains valid even after this DLL unloads.
        Ok(())
    }

    #[must_use]
    pub(crate) const fn trampoline(&self) -> RemoteAddress {
        self.trampoline
    }

    #[must_use]
    pub(crate) const fn is_active(&self) -> bool {
        self.active
    }
}

fn read_stolen_instructions(
    target: RemoteAddress,
) -> Result<([u8; MAX_STOLEN_BYTES], usize), EndSceneHookError> {
    let memory = LocalProcessMemory;
    let mut original = [0_u8; MAX_STOLEN_BYTES];
    let mut stolen_len = 0_usize;

    while stolen_len < JMP_SIZE {
        let instruction_address =
            target
                .checked_add(stolen_len as u32)
                .ok_or(EndSceneHookError::AddressOverflow {
                    address: target,
                    offset: stolen_len as u32,
                })?;
        let bytes = read_instruction(&memory, instruction_address)?;
        let instruction_len = decode_instruction(&bytes).map_err(|opcode| {
            EndSceneHookError::UnsupportedInstruction {
                target: instruction_address,
                opcode,
            }
        })?;
        let next_len =
            stolen_len
                .checked_add(instruction_len)
                .ok_or(EndSceneHookError::AddressOverflow {
                    address: target,
                    offset: instruction_len as u32,
                })?;
        if next_len > MAX_STOLEN_BYTES {
            return Err(EndSceneHookError::UnsupportedInstruction {
                target: instruction_address,
                opcode: bytes[0],
            });
        }
        original[stolen_len..next_len].copy_from_slice(&bytes[..instruction_len]);
        stolen_len = next_len;
    }

    Ok((original, stolen_len))
}

fn read_instruction(
    memory: &LocalProcessMemory,
    address: RemoteAddress,
) -> Result<[u8; MAX_INSTRUCTION_SIZE], EndSceneHookError> {
    let mut bytes = [0_u8; MAX_INSTRUCTION_SIZE];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let byte_address =
            address
                .checked_add(index as u32)
                .ok_or(EndSceneHookError::AddressOverflow {
                    address,
                    offset: index as u32,
                })?;
        *byte = memory
            .read_u8(byte_address)
            .map_err(EndSceneHookError::Read)?;
    }
    Ok(bytes)
}

/// Returns the complete instruction length, rejecting relative control flow.
///
/// A direct trampoline may safely copy ordinary prologue instructions. It may
/// not blindly copy a relative call/jump/conditional branch, because its
/// destination would change at the trampoline address. Rejecting such an
/// entry is intentional: startup fails cleanly instead of corrupting control
/// flow, and we can add a proper relocation rule from verified client bytes.
fn decode_instruction(bytes: &[u8; MAX_INSTRUCTION_SIZE]) -> Result<usize, u8> {
    let mut cursor = 0_usize;
    let mut operand_size = 4_usize;

    loop {
        let prefix = byte_at(bytes, cursor)?;
        match prefix {
            0x66 => {
                operand_size = 2;
                cursor += 1;
            }
            0x67 => return Err(prefix),
            0xF0 | 0xF2 | 0xF3 | 0x2E | 0x36 | 0x3E | 0x26 | 0x64 | 0x65 => cursor += 1,
            _ => break,
        }
        if cursor >= MAX_INSTRUCTION_SIZE {
            return Err(prefix);
        }
    }

    let opcode = byte_at(bytes, cursor)?;
    cursor += 1;
    if opcode == 0x0F {
        let extension = byte_at(bytes, cursor)?;
        cursor += 1;
        if matches!(extension, 0x80..=0x8F) {
            return Err(extension);
        }
        if matches!(
            extension,
            0x05 | 0x06 | 0x07 | 0x08 | 0x09 | 0x0B | 0x30..=0x37 | 0x77 | 0xA0..=0xA2
                | 0xA8..=0xAA
        ) {
            return finish(cursor, 0, extension);
        }
        let immediate = match extension {
            0x0F | 0x70 | 0x71 | 0x72 | 0x73 | 0xA4 | 0xAC | 0xBA | 0xC2 => 1,
            _ => 0,
        };
        return decode_modrm(bytes, cursor, immediate, extension);
    }

    if matches!(opcode, 0x70..=0x7F | 0xE0..=0xE3 | 0xE8 | 0xE9 | 0xEB) {
        return Err(opcode);
    }

    match opcode {
        0x04
        | 0x0C
        | 0x14
        | 0x1C
        | 0x24
        | 0x2C
        | 0x34
        | 0x3C
        | 0x6A
        | 0x82
        | 0xA8
        | 0xB0..=0xB7
        | 0xCD
        | 0xD4
        | 0xD5
        | 0xE4..=0xE7 => finish(cursor, 1, opcode),
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D | 0x68 | 0xA9 | 0xB8..=0xBF => {
            finish(cursor, operand_size, opcode)
        }
        0x9A | 0xEA => finish(cursor, operand_size + 2, opcode),
        0xA0..=0xA3 => finish(cursor, 4, opcode),
        0xC2 | 0xCA => finish(cursor, 2, opcode),
        0xC8 => finish(cursor, 3, opcode),
        0x06
        | 0x07
        | 0x0E
        | 0x16
        | 0x17
        | 0x1E
        | 0x1F
        | 0x27
        | 0x2F
        | 0x37
        | 0x3F
        | 0x40..=0x5F
        | 0x60
        | 0x61
        | 0x6C..=0x6F
        | 0x90..=0x9F
        | 0xA4..=0xA7
        | 0xAA..=0xAF
        | 0xC3
        | 0xC9
        | 0xCB
        | 0xCC
        | 0xCE
        | 0xCF
        | 0xD6
        | 0xD7
        | 0xEC..=0xEF
        | 0xF1
        | 0xF4
        | 0xF5
        | 0xF8..=0xFD => finish(cursor, 0, opcode),
        _ => {
            let immediate = match opcode {
                0x69 | 0x81 | 0xC7 => operand_size,
                0x6B | 0x80 | 0x83 | 0xC0 | 0xC1 | 0xC6 => 1,
                _ => 0,
            };
            decode_modrm(bytes, cursor, immediate, opcode)
        }
    }
}

fn decode_modrm(
    bytes: &[u8; MAX_INSTRUCTION_SIZE],
    mut cursor: usize,
    mut immediate: usize,
    opcode: u8,
) -> Result<usize, u8> {
    let modrm = byte_at(bytes, cursor)?;
    cursor += 1;
    let mode = modrm >> 6;
    let register = (modrm >> 3) & 7;
    let register_memory = modrm & 7;

    if mode != 3 && register_memory == 4 {
        let sib = byte_at(bytes, cursor)?;
        cursor += 1;
        if mode == 0 && (sib & 7) == 5 {
            cursor = cursor.checked_add(4).ok_or(opcode)?;
        }
    }
    cursor = match mode {
        0 if register_memory == 5 => cursor.checked_add(4).ok_or(opcode)?,
        1 => cursor.checked_add(1).ok_or(opcode)?,
        2 => cursor.checked_add(4).ok_or(opcode)?,
        _ => cursor,
    };

    if opcode == 0xF6 && matches!(register, 0 | 1) {
        immediate = 1;
    }
    if opcode == 0xF7 && matches!(register, 0 | 1) {
        immediate = 4;
    }
    finish(cursor, immediate, opcode)
}

fn finish(cursor: usize, tail: usize, opcode: u8) -> Result<usize, u8> {
    let length = cursor.checked_add(tail).ok_or(opcode)?;
    if length > MAX_INSTRUCTION_SIZE {
        Err(opcode)
    } else {
        Ok(length)
    }
}

fn byte_at(bytes: &[u8; MAX_INSTRUCTION_SIZE], index: usize) -> Result<u8, u8> {
    bytes.get(index).copied().ok_or(0)
}

fn allocate_trampoline() -> Result<RemoteAddress, EndSceneHookError> {
    let allocation = unsafe {
        VirtualAlloc(
            core::ptr::null_mut(),
            TRAMPOLINE_SIZE,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_EXECUTE_READWRITE,
        )
    };
    if allocation.is_null() {
        return Err(EndSceneHookError::TrampolineAllocation {
            win32_error: unsafe { GetLastError() },
        });
    }
    u32::try_from(allocation as usize)
        .map_err(|_| EndSceneHookError::TrampolineAllocation { win32_error: 0 })
}

fn write_relative_jump(
    output: &mut [u8],
    source: RemoteAddress,
    destination: RemoteAddress,
) -> Result<(), EndSceneHookError> {
    let next = source
        .checked_add(JMP_SIZE as u32)
        .ok_or(EndSceneHookError::AddressOverflow {
            address: source,
            offset: JMP_SIZE as u32,
        })?;
    let displacement = i64::from(destination) - i64::from(next);
    let displacement =
        i32::try_from(displacement).map_err(|_| EndSceneHookError::RelativeJumpOutOfRange {
            source,
            destination,
        })?;
    output[0] = 0xE9;
    output[1..JMP_SIZE].copy_from_slice(&displacement.to_le_bytes());
    Ok(())
}

fn write_protected_code(target: RemoteAddress, bytes: &[u8]) -> Result<(), EndSceneHookError> {
    let pointer = as_local_mut_pointer(target);
    let mut old_protection = 0_u32;
    let writable = unsafe {
        VirtualProtect(
            pointer,
            bytes.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old_protection,
        )
    };
    if writable == 0 {
        return Err(EndSceneHookError::Protect {
            target,
            win32_error: unsafe { GetLastError() },
        });
    }

    let write_result = write_process_bytes(target, bytes);
    let flush_result = flush_instruction_cache(target, bytes.len());
    let mut ignored = 0_u32;
    let restored = unsafe { VirtualProtect(pointer, bytes.len(), old_protection, &mut ignored) };
    if restored == 0 {
        return Err(EndSceneHookError::RestoreProtection {
            target,
            win32_error: unsafe { GetLastError() },
        });
    }
    write_result?;
    flush_result
}

fn write_process_bytes(target: RemoteAddress, bytes: &[u8]) -> Result<(), EndSceneHookError> {
    let mut bytes_written = 0_usize;
    let succeeded = unsafe {
        WriteProcessMemory(
            GetCurrentProcess(),
            as_local_mut_pointer(target),
            bytes.as_ptr().cast(),
            bytes.len(),
            &mut bytes_written,
        )
    };
    if succeeded == 0 || bytes_written != bytes.len() {
        return Err(EndSceneHookError::Write(LocalProcessMemoryError::Write {
            address: target,
            win32_error: unsafe { GetLastError() },
        }));
    }
    Ok(())
}

fn flush_instruction_cache(target: RemoteAddress, len: usize) -> Result<(), EndSceneHookError> {
    let flushed =
        unsafe { FlushInstructionCache(GetCurrentProcess(), as_local_pointer(target), len) };
    if flushed == 0 {
        return Err(EndSceneHookError::FlushInstructionCache {
            target,
            win32_error: unsafe { GetLastError() },
        });
    }
    Ok(())
}

struct SuspendedThreads {
    handles: Vec<*mut c_void>,
}

impl SuspendedThreads {
    fn acquire() -> Result<Self, EndSceneHookError> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(EndSceneHookError::ThreadSnapshot {
                win32_error: unsafe { GetLastError() },
            });
        }
        let snapshot = Snapshot(snapshot);
        let process_id = unsafe { GetCurrentProcessId() };
        let current_thread = unsafe { GetCurrentThreadId() };
        let mut suspended = Self {
            handles: Vec::new(),
        };
        let mut entry = ThreadEntry32::new();
        let mut present = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;

        while present {
            if entry.owner_process_id == process_id && entry.thread_id != current_thread {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id) };
                if thread.is_null() {
                    // A thread can terminate after the snapshot. It can no
                    // longer execute our patch, so ignoring that race is safe.
                    let error = unsafe { GetLastError() };
                    if error != 87 {
                        return Err(EndSceneHookError::ThreadSuspend {
                            thread_id: entry.thread_id,
                            win32_error: error,
                        });
                    }
                } else if unsafe { SuspendThread(thread) } == u32::MAX {
                    let error = unsafe { GetLastError() };
                    unsafe {
                        let _ = CloseHandle(thread);
                    }
                    return Err(EndSceneHookError::ThreadSuspend {
                        thread_id: entry.thread_id,
                        win32_error: error,
                    });
                } else {
                    suspended.handles.push(thread);
                }
            }

            entry = ThreadEntry32::new();
            present = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
        }

        Ok(suspended)
    }
}

impl Drop for SuspendedThreads {
    fn drop(&mut self) {
        for thread in self.handles.drain(..).rev() {
            unsafe {
                let _ = ResumeThread(thread);
                let _ = CloseHandle(thread);
            }
        }
    }
}

struct Snapshot(*mut c_void);

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[repr(C)]
struct ThreadEntry32 {
    size: u32,
    usage: u32,
    thread_id: u32,
    owner_process_id: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

impl ThreadEntry32 {
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            usage: 0,
            thread_id: 0,
            owner_process_id: 0,
            base_priority: 0,
            delta_priority: 0,
            flags: 0,
        }
    }
}

/// Converts an x86 remote address only at a local Win32 FFI boundary.
fn as_local_pointer(address: RemoteAddress) -> *const c_void {
    address as usize as *const c_void
}

fn as_local_mut_pointer(address: RemoteAddress) -> *mut c_void {
    address as usize as *mut c_void
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn OpenThread(access: u32, inherit_handle: i32, thread_id: u32) -> *mut c_void;
    fn ResumeThread(thread: *mut c_void) -> u32;
    fn SuspendThread(thread: *mut c_void) -> u32;
    fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry32) -> i32;
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: u32,
        protect: u32,
    ) -> *mut c_void;
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protection: u32,
        old_protection: *mut u32,
    ) -> i32;
    fn WriteProcessMemory(
        process: *mut c_void,
        base_address: *mut c_void,
        buffer: *const c_void,
        size: usize,
        bytes_written: *mut usize,
    ) -> i32;
}
