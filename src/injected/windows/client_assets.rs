//! Automatic, client-rooted source of static terrain assets.
//!
//! The DLL asks Windows for the path of the executable hosting it, so normal
//! loader usage, launcher paths, and custom client installs need no manually
//! supplied game-directory configuration. Archive decoding is intentionally a
//! separate adapter: this source first supports loose ADT files and reports a
//! clear, non-fatal error when the tile exists only inside an MPQ.

use core::ffi::c_void;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use crate::offsets::api::{TerrainError, TerrainTile, TerrainTileKey, TerrainTileLoader};

extern "system" {
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
}

/// Reads loose terrain files relative to the executable that hosts this DLL.
///
/// It is deliberately not constructed from an arbitrary path: this prevents
/// a plugin from redirecting Veyr to unrelated user files and makes custom
/// content resolve from the same client installation that is actually running.
#[derive(Debug, Clone)]
pub(crate) struct ClientTerrainSource {
    client_root: PathBuf,
}

impl ClientTerrainSource {
    pub(crate) fn from_current_process() -> Result<Self, ClientTerrainSourceError> {
        let executable = current_process_executable()?;
        let client_root =
            executable
                .parent()
                .ok_or_else(|| ClientTerrainSourceError::InvalidExecutablePath {
                    executable: executable.clone(),
                })?;
        Ok(Self {
            client_root: client_root.to_owned(),
        })
    }

    #[must_use]
    pub(crate) fn client_root(&self) -> &Path {
        &self.client_root
    }

    /// Canonical loose-file location for one client map tile.
    #[must_use]
    pub(crate) fn loose_tile_path(&self, key: &TerrainTileKey) -> PathBuf {
        self.client_root
            .join("Data")
            .join("World")
            .join("Maps")
            .join(&key.map_name)
            .join(format!("{}_{}_{}.adt", key.map_name, key.x, key.y))
    }
}

impl TerrainTileLoader for ClientTerrainSource {
    type Error = ClientTerrainSourceError;

    fn load_tile(&mut self, key: &TerrainTileKey) -> Result<TerrainTile, Self::Error> {
        let path = self.loose_tile_path(key);
        let bytes = fs::read(&path).map_err(|source| ClientTerrainSourceError::LooseTile {
            key: key.clone(),
            path,
            source,
        })?;
        TerrainTile::from_adt_bytes(&bytes).map_err(|source| ClientTerrainSourceError::Decode {
            key: key.clone(),
            source,
        })
    }
}

#[derive(Debug)]
pub(crate) enum ClientTerrainSourceError {
    ExecutablePath(io::Error),
    InvalidExecutablePath {
        executable: PathBuf,
    },
    LooseTile {
        key: TerrainTileKey,
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        key: TerrainTileKey,
        source: TerrainError,
    },
}

fn current_process_executable() -> Result<PathBuf, ClientTerrainSourceError> {
    let mut capacity = 260_usize;
    loop {
        let mut buffer = vec![0_u16; capacity];
        let written = unsafe {
            GetModuleFileNameW(
                core::ptr::null_mut(),
                buffer.as_mut_ptr(),
                buffer.len().try_into().expect("path buffer fits u32"),
            )
        } as usize;
        if written == 0 {
            return Err(ClientTerrainSourceError::ExecutablePath(
                io::Error::last_os_error(),
            ));
        }
        if written < buffer.len() - 1 {
            buffer.truncate(written);
            return Ok(PathBuf::from(OsString::from_wide(&buffer)));
        }
        capacity = capacity
            .checked_mul(2)
            .filter(|next| *next <= 32_768)
            .ok_or_else(|| ClientTerrainSourceError::ExecutablePath(io::Error::last_os_error()))?;
    }
}
