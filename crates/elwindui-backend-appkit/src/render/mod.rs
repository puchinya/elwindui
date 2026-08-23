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

mod batch;
mod fastpath;
mod geometry;
mod image;
mod layer;
mod paint;
mod path;
pub(crate) mod stats;
mod text;
mod vector;

pub(crate) use batch::try_batch_fills;
pub(crate) use fastpath::{try_fast_path, try_update_fast_path};
pub(crate) use geometry::{
    clip_bounds, clip_mask_layer, color_to_cgcolor, geometry_bounds, transform_point,
};
pub(crate) use image::{
    build_image_container_layer, cgimage_bytes, fitted_image_rect, resolve_cgimage,
};
pub(crate) use layer::{
    ImplicitAnimationGuard, add_sublayer_scaled, paint_layer_name, set_bounds_if_changed,
    set_contents_scale_if_changed, set_hidden_if_changed, set_mask_scaled, set_position_if_changed,
};
pub(crate) use paint::{
    GradientMaskShape, add_shape_layer, apply_fill, apply_stroke, first_gradient_stop_color,
    gradient_unit_point, try_add_gradient_fill_layer, try_add_image_fill_layer,
};
pub(crate) use path::{ellipse_cgpath, path_to_cgpath, rounded_rect_cgpath};
pub(crate) use text::{AppKitTextBackend, attributed_string, ns_font, secure_text_font};
pub(crate) use vector::{
    draw_vector_image, pixels_to_cgimage, rasterize_calayer_to_pixels,
    rasterize_vector_image_to_cgimage,
};
