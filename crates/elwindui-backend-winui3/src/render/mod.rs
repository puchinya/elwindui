//! The drawing half of this backend: `elwindui_core::graphics` values -> Win2D command lists and
//! Microsoft.UI.Composition visuals.
//!
//! Knows nothing about `UIElement`, focus, or any `Inner*` control — it is handed a
//! `RenderGroup`/`RenderCommand` tree by `host` and translates it, so every dependency runs one
//! way: `native_ui -> inner -> host -> render`.
//!
//! - `win2d`       — the immediate-mode command-list primitives
//! - `vector`      — SVG scene emission onto a drawing surface
//! - `composition` — the retained Composition-visual renderer

pub(crate) mod composition;
mod text;
mod vector;
mod win2d;

pub(crate) use text::{
    WinUi3TextBackend, apply_cascaded_text_style_to_control, apply_control_background,
    apply_text_style_to_text_block_with_foreground, clear_control_foreground,
};
#[cfg(test)]
pub(crate) use text::{apply_text_style_to_control, apply_text_style_to_text_block};
pub(crate) use vector::*;
pub(crate) use win2d::*;
