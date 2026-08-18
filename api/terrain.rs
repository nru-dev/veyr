//! Read-only WotLK ADT terrain heights.
//!
//! Static terrain is archived game data, not an ObjectManager entity. This
//! module deliberately parses an already supplied ADT byte slice so archive
//! lookup (MPQ path, map ID, and tile cache) remains outside the geometry
//! layer. It is safe to use in developer tools and does not touch game memory.

use core::mem::size_of;
use std::collections::{HashMap, VecDeque};

use super::Vector3;
use crate::offsets::world_assets::adt::{self, mcnk, mcvt};

const MCNK_HEADER_SIZE: usize = mcnk::HEADER_SIZE as usize;
const ADT_CHUNK_HEADER_SIZE: usize = adt::CHUNK_HEADER_SIZE as usize;
const MCVT_PAYLOAD_SIZE: usize = mcvt::PAYLOAD_SIZE as usize;

/// One terrain sample in Veyr world coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TerrainSample {
    pub position: Vector3,
    pub chunk_index_x: u32,
    pub chunk_index_y: u32,
}

/// A decoded WotLK ADT tile containing the terrain chunks that expose MCVT
/// heightmaps. Static models and water are deliberately separate readers.
#[derive(Debug, Clone)]
pub struct TerrainTile {
    chunks: Vec<TerrainChunk>,
}

/// Identifies one ADT tile in the map namespace used by the client files.
///
/// `map_name` is an internal map directory name (for example, `Azeroth`), not
/// a localized display title. The future map resolver derives it from the
/// active client's `Map.dbc`, so custom maps use the same contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TerrainTileKey {
    pub map_name: String,
    pub x: u8,
    pub y: u8,
}

impl TerrainTileKey {
    #[must_use]
    pub fn new(map_name: impl Into<String>, x: u8, y: u8) -> Self {
        Self {
            map_name: map_name.into(),
            x,
            y,
        }
    }
}

/// Loads one static-terrain tile without exposing archive/file details to
/// plugins or render code. The Windows runtime will supply a client-root
/// implementation; unit tests can use a byte-backed implementation instead.
pub trait TerrainTileLoader {
    type Error;

    fn load_tile(&mut self, key: &TerrainTileKey) -> Result<TerrainTile, Self::Error>;
}

/// Small least-recently-used cache around a [`TerrainTileLoader`].
///
/// A terrain-following circle reads neighbouring tiles around seams, so the
/// cache belongs beneath the renderer/plugin layer rather than making every
/// caller rediscover or reopen ADT files each frame.
pub struct TerrainCache<L> {
    loader: L,
    capacity: usize,
    tiles: HashMap<TerrainTileKey, TerrainTile>,
    order: VecDeque<TerrainTileKey>,
}

impl<L> TerrainCache<L> {
    /// Creates a cache with at least one slot.
    #[must_use]
    pub fn new(loader: L, capacity: usize) -> Self {
        Self {
            loader,
            capacity: capacity.max(1),
            tiles: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn loaded_tile_count(&self) -> usize {
        self.tiles.len()
    }

    #[must_use]
    pub fn loader(&self) -> &L {
        &self.loader
    }

    #[must_use]
    pub fn loader_mut(&mut self) -> &mut L {
        &mut self.loader
    }

    #[must_use]
    pub fn into_loader(self) -> L {
        self.loader
    }
}

impl<L: TerrainTileLoader> TerrainCache<L> {
    /// Gets a decoded tile, loading it only on the first request or after LRU
    /// eviction. The returned reference is valid until the next mutable cache
    /// operation.
    pub fn tile(
        &mut self,
        key: &TerrainTileKey,
    ) -> Result<&TerrainTile, TerrainCacheError<L::Error>> {
        if !self.tiles.contains_key(key) {
            let tile = self
                .loader
                .load_tile(key)
                .map_err(|source| TerrainCacheError::Load {
                    key: key.clone(),
                    source,
                })?;
            self.insert(key.clone(), tile);
        }
        self.touch(key);
        // The entry was inserted above or was already present, and `touch`
        // changes only the LRU queue.
        Ok(self.tiles.get(key).expect("present cache key"))
    }

    /// Samples a loaded-or-cached tile in world coordinates.
    pub fn height_at(
        &mut self,
        key: &TerrainTileKey,
        x: f32,
        y: f32,
    ) -> Result<Option<f32>, TerrainCacheError<L::Error>> {
        Ok(self.tile(key)?.height_at(x, y))
    }

    fn insert(&mut self, key: TerrainTileKey, tile: TerrainTile) {
        while self.tiles.len() >= self.capacity {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.tiles.remove(&evicted);
        }
        self.tiles.insert(key.clone(), tile);
        self.order.push_back(key);
    }

    fn touch(&mut self, key: &TerrainTileKey) {
        self.order.retain(|queued| queued != key);
        self.order.push_back(key.clone());
    }
}

/// A tile source failed while the cache was serving a request.
#[derive(Debug)]
pub enum TerrainCacheError<E> {
    Load { key: TerrainTileKey, source: E },
}

impl TerrainTile {
    /// Decodes every usable `MCNK → MCVT` heightmap in a WotLK ADT file.
    ///
    /// Both conventional and byte-reversed on-disk FourCC spellings are
    /// accepted. This makes the decoder robust to tools that expose chunk
    /// tags as raw bytes versus normalized little-endian FourCC values.
    pub fn from_adt_bytes(bytes: &[u8]) -> Result<Self, TerrainError> {
        let mut chunks = Vec::with_capacity(adt::CHUNKS_PER_TILE_AXIS as usize);
        let mut offset = 0_usize;

        while offset < bytes.len() {
            let (tag, payload_size) = read_chunk_header(bytes, offset)?;
            let payload_start = offset
                .checked_add(ADT_CHUNK_HEADER_SIZE)
                .ok_or(TerrainError::OffsetOverflow { offset })?;
            let next = payload_start
                .checked_add(payload_size)
                .ok_or(TerrainError::OffsetOverflow { offset })?;
            if next > bytes.len() {
                return Err(TerrainError::TruncatedChunk {
                    offset,
                    declared_size: payload_size,
                });
            }

            if matches_tag(tag, adt::chunks::MCNK) {
                chunks.push(TerrainChunk::from_mcnk(bytes, offset, payload_size)?);
            }
            offset = next;
        }

        if chunks.is_empty() {
            return Err(TerrainError::NoTerrainChunks);
        }

        Ok(Self { chunks })
    }

    /// Returns the terrain surface height at world-space `(x, y)`.
    ///
    /// The returned Z includes the parent MCNK base height. `None` means this
    /// tile has no MCVT chunk covering the requested coordinates; callers can
    /// then request another ADT tile or skip that circle segment safely.
    #[must_use]
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.sample_at(x, y).map(|sample| sample.position.z)
    }

    /// Returns both terrain height and the MCNK which provided it.
    #[must_use]
    pub fn sample_at(&self, x: f32, y: f32) -> Option<TerrainSample> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }

        self.chunks.iter().find_map(|chunk| chunk.sample_at(x, y))
    }

    #[must_use]
    pub const fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[derive(Debug, Clone)]
struct TerrainChunk {
    index_x: u32,
    index_y: u32,
    origin: Vector3,
    heights: [f32; mcvt::VERTEX_COUNT as usize],
}

impl TerrainChunk {
    fn from_mcnk(
        bytes: &[u8],
        chunk_offset: usize,
        payload_size: usize,
    ) -> Result<Self, TerrainError> {
        if payload_size < MCNK_HEADER_SIZE {
            return Err(TerrainError::InvalidMcnkHeader {
                offset: chunk_offset,
                payload_size,
            });
        }

        let header = chunk_offset.checked_add(ADT_CHUNK_HEADER_SIZE).ok_or(
            TerrainError::OffsetOverflow {
                offset: chunk_offset,
            },
        )?;
        let mcnk_end = header
            .checked_add(payload_size)
            .ok_or(TerrainError::OffsetOverflow {
                offset: chunk_offset,
            })?;

        let mcvt_relative_offset = read_u32_at(bytes, header + mcnk::MCVT_OFFSET as usize)?;
        let mcvt_offset = chunk_offset
            .checked_add(mcvt_relative_offset as usize)
            .ok_or(TerrainError::OffsetOverflow {
                offset: chunk_offset,
            })?;
        let (tag, payload_size) = read_chunk_header(bytes, mcvt_offset)?;
        if !matches_tag(tag, adt::chunks::MCVT) || payload_size < MCVT_PAYLOAD_SIZE {
            return Err(TerrainError::InvalidMcvt {
                offset: mcvt_offset,
                payload_size,
            });
        }
        let mcvt_payload =
            mcvt_offset
                .checked_add(ADT_CHUNK_HEADER_SIZE)
                .ok_or(TerrainError::OffsetOverflow {
                    offset: mcvt_offset,
                })?;
        let mcvt_end =
            mcvt_payload
                .checked_add(MCVT_PAYLOAD_SIZE)
                .ok_or(TerrainError::OffsetOverflow {
                    offset: mcvt_offset,
                })?;
        if mcvt_end > mcnk_end || mcvt_end > bytes.len() {
            return Err(TerrainError::TruncatedMcvt {
                offset: mcvt_offset,
            });
        }

        let mut heights = [0.0; mcvt::VERTEX_COUNT as usize];
        for (index, height) in heights.iter_mut().enumerate() {
            *height = read_f32_at(bytes, mcvt_payload + index * size_of::<f32>())?;
            if !height.is_finite() {
                return Err(TerrainError::NonFiniteHeight {
                    offset: mcvt_payload + index * size_of::<f32>(),
                });
            }
        }

        // WotLK stores the MCNK origin as z, x, y. Veyr uses x, y, z.
        let origin = Vector3 {
            x: read_f32_at(bytes, header + mcnk::FILE_POSITION_X as usize)?,
            y: read_f32_at(bytes, header + mcnk::FILE_POSITION_Y as usize)?,
            z: read_f32_at(bytes, header + mcnk::FILE_POSITION_Z as usize)?,
        };
        if !origin.x.is_finite() || !origin.y.is_finite() || !origin.z.is_finite() {
            return Err(TerrainError::NonFiniteChunkOrigin {
                offset: chunk_offset,
            });
        }

        Ok(Self {
            index_x: read_u32_at(bytes, header + mcnk::INDEX_X as usize)?,
            index_y: read_u32_at(bytes, header + mcnk::INDEX_Y as usize)?,
            origin,
            heights,
        })
    }

    fn sample_at(&self, x: f32, y: f32) -> Option<TerrainSample> {
        let local_x = x - self.origin.x;
        let local_y = y - self.origin.y;
        let chunk_size = adt::CHUNK_SIZE;
        if !(0.0..=chunk_size).contains(&local_x) || !(0.0..=chunk_size).contains(&local_y) {
            return None;
        }

        let cell_size = chunk_size / 8.0;
        let cell_x = (local_x / cell_size).min(7.999_999);
        let cell_y = (local_y / cell_size).min(7.999_999);
        let x_index = cell_x as usize;
        let y_index = cell_y as usize;
        let fraction_x = cell_x - x_index as f32;
        let fraction_y = cell_y - y_index as f32;

        let h00 = self.outer_height(x_index, y_index)?;
        let h10 = self.outer_height(x_index + 1, y_index)?;
        let h01 = self.outer_height(x_index, y_index + 1)?;
        let h11 = self.outer_height(x_index + 1, y_index + 1)?;
        let hc = self.inner_height(x_index, y_index)?;
        let relative_height = interpolate_diamond(h00, h10, h01, h11, hc, fraction_x, fraction_y);

        Some(TerrainSample {
            position: Vector3 {
                x,
                y,
                z: self.origin.z + relative_height,
            },
            chunk_index_x: self.index_x,
            chunk_index_y: self.index_y,
        })
    }

    fn outer_height(&self, x: usize, y: usize) -> Option<f32> {
        (x <= 8 && y <= 8).then(|| self.heights[y * 17 + x])
    }

    fn inner_height(&self, x: usize, y: usize) -> Option<f32> {
        (x < 8 && y < 8).then(|| self.heights[y * 17 + 9 + x])
    }
}

fn interpolate_diamond(h00: f32, h10: f32, h01: f32, h11: f32, center: f32, x: f32, y: f32) -> f32 {
    // An MCNK cell is four triangles around its centre vertex, rather than a
    // bilinear quad. The four half-planes choose the triangle, then the
    // expression is its barycentric interpolation. In particular, (0.5, 0.5)
    // must evaluate to `center` exactly.
    if y <= x && x + y <= 1.0 {
        h00 * (1.0 - x - y) + h10 * (x - y) + center * (2.0 * y)
    } else if x <= y && x + y <= 1.0 {
        h00 * (1.0 - x - y) + h01 * (y - x) + center * (2.0 * x)
    } else if x >= y {
        h10 * (x - y) + h11 * (x + y - 1.0) + center * (2.0 * (1.0 - x))
    } else {
        h01 * (y - x) + h11 * (x + y - 1.0) + center * (2.0 * (1.0 - y))
    }
}

fn read_chunk_header(bytes: &[u8], offset: usize) -> Result<([u8; 4], usize), TerrainError> {
    let tag_end = offset
        .checked_add(4)
        .ok_or(TerrainError::OffsetOverflow { offset })?;
    let size_end = offset
        .checked_add(ADT_CHUNK_HEADER_SIZE)
        .ok_or(TerrainError::OffsetOverflow { offset })?;
    let tag = bytes
        .get(offset..tag_end)
        .ok_or(TerrainError::TruncatedHeader { offset })?
        .try_into()
        .expect("a four-byte range has exactly four bytes");
    let size = bytes
        .get(tag_end..size_end)
        .ok_or(TerrainError::TruncatedHeader { offset })?
        .try_into()
        .expect("a four-byte range has exactly four bytes");
    Ok((tag, u32::from_le_bytes(size) as usize))
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, TerrainError> {
    let end = offset
        .checked_add(size_of::<u32>())
        .ok_or(TerrainError::OffsetOverflow { offset })?;
    let raw = bytes
        .get(offset..end)
        .ok_or(TerrainError::TruncatedValue { offset })?
        .try_into()
        .expect("a four-byte range has exactly four bytes");
    Ok(u32::from_le_bytes(raw))
}

fn read_f32_at(bytes: &[u8], offset: usize) -> Result<f32, TerrainError> {
    Ok(f32::from_bits(read_u32_at(bytes, offset)?))
}

fn matches_tag(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual == expected || actual == [expected[3], expected[2], expected[1], expected[0]]
}

/// A malformed or incomplete static-terrain file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TerrainError {
    TruncatedHeader { offset: usize },
    TruncatedValue { offset: usize },
    TruncatedChunk { offset: usize, declared_size: usize },
    OffsetOverflow { offset: usize },
    InvalidMcnkHeader { offset: usize, payload_size: usize },
    InvalidMcvt { offset: usize, payload_size: usize },
    TruncatedMcvt { offset: usize },
    NonFiniteHeight { offset: usize },
    NonFiniteChunkOrigin { offset: usize },
    NoTerrainChunks,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_flat_mcnk_heightmap_in_world_coordinates() {
        let tile = TerrainTile::from_adt_bytes(&test_adt(10.0, 5.0)).expect("valid ADT");

        assert_eq!(tile.chunk_count(), 1);
        assert_eq!(tile.height_at(104.0, 204.0), Some(15.0));
        assert_eq!(tile.height_at(140.0, 204.0), None);
        assert_eq!(tile.height_at(f32::NAN, 204.0), None);
    }

    #[test]
    fn accepts_raw_reversed_wow_chunk_tags() {
        let mut bytes = test_adt(3.0, 2.0);
        bytes[0..4].copy_from_slice(b"KNCM");
        let mcvt = ADT_CHUNK_HEADER_SIZE + MCNK_HEADER_SIZE;
        bytes[mcvt..mcvt + 4].copy_from_slice(b"TVCM");

        let tile = TerrainTile::from_adt_bytes(&bytes).expect("valid reverse-tag ADT");
        assert_eq!(tile.height_at(100.5, 200.5), Some(5.0));
    }

    #[test]
    fn interpolates_the_center_vertex() {
        let mut bytes = test_adt(0.0, 0.0);
        let mcvt_payload = ADT_CHUNK_HEADER_SIZE + MCNK_HEADER_SIZE + ADT_CHUNK_HEADER_SIZE;
        write_f32(&mut bytes, mcvt_payload + 9 * size_of::<f32>(), 8.0);

        let tile = TerrainTile::from_adt_bytes(&bytes).expect("valid ADT");
        let unit = adt::CHUNK_SIZE / 8.0;
        let height = tile
            .height_at(100.0 + 0.5 * unit, 200.0 + 0.5 * unit)
            .expect("the centre belongs to the chunk");
        assert!((height - 8.0).abs() < 0.001);
    }

    fn test_adt(base_height: f32, relative_height: f32) -> Vec<u8> {
        let mut mcnk_payload = vec![0_u8; MCNK_HEADER_SIZE];
        write_u32(&mut mcnk_payload, mcnk::INDEX_X as usize, 7);
        write_u32(&mut mcnk_payload, mcnk::INDEX_Y as usize, 11);
        write_u32(
            &mut mcnk_payload,
            mcnk::MCVT_OFFSET as usize,
            (ADT_CHUNK_HEADER_SIZE + MCNK_HEADER_SIZE) as u32,
        );
        write_f32(
            &mut mcnk_payload,
            mcnk::FILE_POSITION_Z as usize,
            base_height,
        );
        write_f32(&mut mcnk_payload, mcnk::FILE_POSITION_X as usize, 100.0);
        write_f32(&mut mcnk_payload, mcnk::FILE_POSITION_Y as usize, 200.0);

        let mut mcvt = vec![0_u8; MCVT_PAYLOAD_SIZE];
        for index in 0..mcvt::VERTEX_COUNT as usize {
            write_f32(&mut mcvt, index * size_of::<f32>(), relative_height);
        }

        append_chunk(&mut mcnk_payload, *b"MCVT", &mcvt);
        let mut adt = Vec::new();
        append_chunk(&mut adt, *b"MCNK", &mcnk_payload);
        adt
    }

    fn append_chunk(output: &mut Vec<u8>, tag: [u8; 4], payload: &[u8]) {
        output.extend_from_slice(&tag);
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend_from_slice(payload);
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
    }

    fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
        write_u32(bytes, offset, value.to_bits());
    }
}
