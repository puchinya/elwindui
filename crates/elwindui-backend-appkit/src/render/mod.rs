//! The drawing half of this backend: `elwindui_core::graphics` values -> Core Animation layers.
//!
//! This layer knows nothing about `UIElement`, focus, or any `Inner*` control. It is handed a
//! `RenderGroup`/`RenderCommand` tree (by `host`, which owns the `NSView` those layers hang off)
//! and translates it, so every dependency runs one way: `native_ui -> inner -> host -> render`.
//! Keeping that direction is what removed the old `inner` <-> `vector_renderer` import cycle —
//! `vector` below is a *sub*module here precisely so it can share `paint`/`path`/`image` without
//! either side reaching back up.
//!
//! Submodules are private and re-exported explicitly below, following
//! `elwindui_core::graphics`'s own `mod.rs` convention.

mod geometry;
mod image;
mod paint;
mod path;
mod vector;

pub(crate) use geometry::{ca_alignment_mode, clip_bounds, clip_mask_layer, color_to_cgcolor, geometry_bounds, parse_color, transform_point};
pub(crate) use image::{build_image_container_layer, fitted_image_rect, resolve_cgimage};
pub(crate) use paint::{GradientMaskShape, add_shape_layer, apply_fill, apply_stroke, gradient_unit_point, try_add_gradient_fill_layer, try_add_image_fill_layer};
pub(crate) use vector::draw_vector_image;
pub(crate) use path::{ellipse_cgpath, path_to_cgpath, rounded_rect_cgpath};
