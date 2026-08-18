//! Building blocks used by the Windows x86 injected module.
//!
//! The actual same-process reader is compiled only for the supported Windows
//! x86 target.

#[cfg_attr(not(all(windows, target_arch = "x86")), allow(dead_code))]
mod circle_geometry;

#[cfg_attr(not(all(windows, target_arch = "x86")), allow(dead_code))]
mod overlay_geometry;

#[cfg(all(windows, target_arch = "x86"))]
pub mod windows;
