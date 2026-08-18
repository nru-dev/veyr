//! Privileged OpenGL implementation of the API render-command backend.
//!
//! WoW's OpenGL path uses a legacy compatibility context, so the first HUD
//! command set can be rendered without context-owned buffers or textures. The
//! renderer changes state only between a complete save/restore pair and never
//! consumes the client's OpenGL error queue with `glGetError`.

use core::ffi::{c_char, c_void};

use crate::injected::circle_geometry::hud_segments;
use crate::injected::overlay_geometry::{circle_strip, line_strip, OverlayVertex};
use crate::offsets::api::{QueuedRenderCommand, RenderCommand};

use super::render_backend::{publish_renderer_stats, NativeFrame, NativeRenderer, RendererStats};

type DeviceContext = *mut c_void;
type GlContext = *mut c_void;
type GlUseProgramFn = unsafe extern "system" fn(program: u32);

const GL_ALPHA_TEST: u32 = 0x0BC0;
const GL_BLEND: u32 = 0x0BE2;
const GL_CULL_FACE: u32 = 0x0B44;
const GL_DEPTH_TEST: u32 = 0x0B71;
const GL_FOG: u32 = 0x0B60;
const GL_LIGHTING: u32 = 0x0B50;
const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_STENCIL_TEST: u32 = 0x0B90;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_VERTEX_PROGRAM_ARB: u32 = 0x8620;
const GL_FRAGMENT_PROGRAM_ARB: u32 = 0x8804;

const GL_SRC_ALPHA: u32 = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const GL_TRIANGLE_STRIP: u32 = 0x0005;

const GL_VIEWPORT: u32 = 0x0BA2;
const GL_MATRIX_MODE: u32 = 0x0BA0;
const GL_MODELVIEW: u32 = 0x1700;
const GL_PROJECTION: u32 = 0x1701;
const GL_CURRENT_PROGRAM: u32 = 0x8B8D;
const GL_ALL_ATTRIB_BITS: u32 = 0x000F_FFFF;

const GL_FALSE: u8 = 0;

// Internal diagnostics are deliberately namespaced with the ASCII bytes
// `GL` in their high half. They are not native GLenum values and are exposed
// only by the private DEV-loader statistics ABI.
const ERROR_INVALID_NATIVE_FRAME: u32 = 0x474C_0001;
const ERROR_NO_CURRENT_CONTEXT: u32 = 0x474C_0002;
const ERROR_NO_CURRENT_DEVICE_CONTEXT: u32 = 0x474C_0003;
const ERROR_DEVICE_CONTEXT_MISMATCH: u32 = 0x474C_0004;
const ERROR_INVALID_VIEWPORT: u32 = 0x474C_0005;

#[link(name = "opengl32")]
extern "system" {
    fn wglGetCurrentContext() -> GlContext;
    fn wglGetCurrentDC() -> DeviceContext;
    fn wglGetProcAddress(name: *const c_char) -> *const c_void;

    fn glBegin(mode: u32);
    fn glBlendFunc(source_factor: u32, destination_factor: u32);
    fn glColor4ub(red: u8, green: u8, blue: u8, alpha: u8);
    fn glDepthMask(enabled: u8);
    fn glDisable(capability: u32);
    fn glEnable(capability: u32);
    fn glEnd();
    fn glGetIntegerv(parameter: u32, values: *mut i32);
    fn glLoadIdentity();
    fn glMatrixMode(mode: u32);
    fn glOrtho(left: f64, right: f64, bottom: f64, top: f64, near: f64, far: f64);
    fn glPopAttrib();
    fn glPopMatrix();
    fn glPushAttrib(mask: u32);
    fn glPushMatrix();
    fn glVertex2f(x: f32, y: f32);
}

/// Legacy fixed-function backend for the first screen-space command set.
///
/// `HudLine` and `HudCircle` are rendered immediately. Text still needs a
/// context-local font implementation, while world primitives need the
/// validated camera path, so those commands remain visible as skipped in the
/// developer diagnostics.
pub struct OpenGlOverlayRenderer {
    /// Minimum quality floor for screen-space circles. Actual density is
    /// selected from pixel circumference so a large circle never inherits the
    /// visibly coarse 48-point tessellation used by the old backend.
    circle_segments: usize,
    last_frame: RendererStats,
}

impl Default for OpenGlOverlayRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenGlOverlayRenderer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            circle_segments: 32,
            last_frame: RendererStats {
                submitted_commands: 0,
                drawn_commands: 0,
                skipped_commands: 0,
                draw_failures: 0,
                state_setup_failed: false,
                last_error: 0,
            },
        }
    }

    /// Selects the minimum circle tessellation floor. Larger HUD circles
    /// still receive extra segments based on their pixel circumference.
    #[must_use]
    pub const fn with_circle_segments(mut self, segments: usize) -> Self {
        self.circle_segments = if segments < 32 {
            32
        } else if segments > 768 {
            768
        } else {
            segments
        };
        self
    }

    #[must_use]
    pub const fn last_frame(&self) -> RendererStats {
        self.last_frame
    }

    fn fail_state_setup(&mut self, error: u32) {
        self.last_frame.state_setup_failed = true;
        self.last_frame.skipped_commands = self.last_frame.submitted_commands;
        self.last_frame.last_error = error;
        publish_renderer_stats(self.last_frame);
    }
}

impl NativeRenderer for OpenGlOverlayRenderer {
    fn render(&mut self, frame: NativeFrame<'_>, commands: &[QueuedRenderCommand]) {
        self.last_frame = RendererStats {
            submitted_commands: commands.len().try_into().unwrap_or(u32::MAX),
            ..RendererStats::default()
        };

        let Some(frame_device_context) = frame.opengl_device_context() else {
            self.fail_state_setup(ERROR_INVALID_NATIVE_FRAME);
            return;
        };

        let current_context = unsafe { wglGetCurrentContext() };
        if current_context.is_null() {
            self.fail_state_setup(ERROR_NO_CURRENT_CONTEXT);
            return;
        }

        let current_device_context = unsafe { wglGetCurrentDC() };
        if current_device_context.is_null() {
            self.fail_state_setup(ERROR_NO_CURRENT_DEVICE_CONTEXT);
            return;
        }
        if current_device_context != frame_device_context {
            self.fail_state_setup(ERROR_DEVICE_CONTEXT_MISMATCH);
            return;
        }

        let mut viewport = [0_i32; 4];
        unsafe { glGetIntegerv(GL_VIEWPORT, viewport.as_mut_ptr()) };
        let [_, _, width, height] = viewport;
        if width <= 0 || height <= 0 {
            self.fail_state_setup(ERROR_INVALID_VIEWPORT);
            return;
        }

        if commands.is_empty() {
            publish_renderer_stats(self.last_frame);
            return;
        }

        // Constructed only after every fallible validation above. Its Drop
        // restores matrices, the original matrix mode, all pushed attributes,
        // and an active GLSL program on every ordinary early return/unwind.
        let saved_state = unsafe { SavedGlState::configure(width, height) };

        for queued in commands {
            match &queued.command {
                RenderCommand::HudLine { from, to, stroke } => {
                    let Some(vertices) = line_strip(*from, *to, *stroke) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    unsafe { draw_strip(&vertices) };
                    self.last_frame.drawn_commands += 1;
                }
                RenderCommand::HudCircle {
                    center,
                    radius,
                    stroke,
                } => {
                    let Some(segments) = hud_segments(*radius, self.circle_segments) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    let Some(vertices) = circle_strip(*center, *radius, *stroke, segments) else {
                        self.last_frame.skipped_commands += 1;
                        continue;
                    };
                    unsafe { draw_strip(&vertices) };
                    self.last_frame.drawn_commands += 1;
                }
                RenderCommand::HudText { .. } | RenderCommand::WorldCircle { .. } => {
                    self.last_frame.skipped_commands += 1;
                }
            }
        }

        // Finish restoring the game's GL state before publishing this frame
        // as complete to diagnostics running on another thread.
        drop(saved_state);
        publish_renderer_stats(self.last_frame);
    }
}

/// Every state changed by [`configure_overlay`] that is not covered by the
/// legacy attribute stack is retained explicitly here.
struct SavedGlState {
    original_matrix_mode: u32,
    original_program: Option<(GlUseProgramFn, u32)>,
}

impl SavedGlState {
    unsafe fn configure(width: i32, height: i32) -> Self {
        let mut original_matrix_mode = GL_MODELVIEW as i32;
        unsafe {
            glGetIntegerv(GL_MATRIX_MODE, &mut original_matrix_mode);
            glPushAttrib(GL_ALL_ATTRIB_BITS);
        }

        let original_program = unsafe { resolve_gl_use_program() }.map(|use_program| {
            let mut program = 0_i32;
            unsafe {
                glGetIntegerv(GL_CURRENT_PROGRAM, &mut program);
                use_program(0);
            }
            // Preserve the raw GLuint bit pattern returned through GLint.
            (use_program, program as u32)
        });

        unsafe {
            glMatrixMode(GL_PROJECTION);
            glPushMatrix();
            glLoadIdentity();
            glOrtho(0.0, f64::from(width), f64::from(height), 0.0, -1.0, 1.0);

            glMatrixMode(GL_MODELVIEW);
            glPushMatrix();
            glLoadIdentity();

            configure_overlay();
        }

        Self {
            original_matrix_mode: original_matrix_mode as u32,
            original_program,
        }
    }
}

impl Drop for SavedGlState {
    fn drop(&mut self) {
        unsafe {
            glMatrixMode(GL_MODELVIEW);
            glPopMatrix();
            glMatrixMode(GL_PROJECTION);
            glPopMatrix();
            glMatrixMode(self.original_matrix_mode);
            glPopAttrib();

            if let Some((use_program, program)) = self.original_program {
                use_program(program);
            }
        }
    }
}

unsafe fn configure_overlay() {
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glDepthMask(GL_FALSE);
        glDisable(GL_CULL_FACE);
        glDisable(GL_ALPHA_TEST);
        glDisable(GL_LIGHTING);
        glDisable(GL_FOG);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_TEXTURE_2D);
        glDisable(GL_VERTEX_PROGRAM_ARB);
        glDisable(GL_FRAGMENT_PROGRAM_ARB);

        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    }
}

unsafe fn draw_strip(vertices: &[OverlayVertex]) {
    debug_assert!(vertices.len() >= 3);
    unsafe {
        glBegin(GL_TRIANGLE_STRIP);
        for vertex in vertices {
            glColor4ub(
                vertex.color.r,
                vertex.color.g,
                vertex.color.b,
                vertex.color.a,
            );
            glVertex2f(vertex.x, vertex.y);
        }
        glEnd();
    }
}

unsafe fn resolve_gl_use_program() -> Option<GlUseProgramFn> {
    let address = unsafe { wglGetProcAddress(c"glUseProgram".as_ptr()) };
    let numeric = address as usize;
    if address.is_null() || numeric <= 3 || numeric == usize::MAX {
        None
    } else {
        // Extension function pointers use APIENTRY (`__stdcall`) on Windows,
        // matching the `extern "system"` function type above.
        Some(unsafe { core::mem::transmute::<*const c_void, GlUseProgramFn>(address) })
    }
}
