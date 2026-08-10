# Backend Common Architecture Guidelines

Guidelines for AI agents working on backend implementation crates (`elwindui-backend-appkit`, `elwindui-backend-winui3`, `elwindui-backend-gtk4`).

## Layered Module Structure

Backend crates share a file-for-file identical layered module structure with single-direction dependency flow:

```text
native_ui -> inner -> host -> render -> ffi
```

### Layer Responsibilities

1. **`native_ui/`**: Public façade. One `#[class]` per builtin element, delegating calls to `inner`. Must not call OS toolkit APIs directly.
2. **`inner/`**: Raw per-control plumbing, organized by control family.
3. **`host/`**: Owns the tree host view and OS window integration.
4. **`render/`**: Handles drawing only. Must NOT know about `UIElement`, focus, or control logic.
5. **`ffi.rs`**: Toolkit seam holding OS-native view handles (`AnyView` etc.).

### Layering Constraints

- **Strict single-direction dependencies**: Higher layers depend on lower layers. Lower layers (e.g. `render`) must never import from higher layers (e.g. `host` or `native_ui`).
- **Logic placement**: Pure Rust value math (rectangle math, geometry, image fitting, layout algorithms) belongs in `elwindui-core`, never duplicated across backend crates.

## Sibling Backend Exploration Policy

Do not inspect sibling backends unless:
- The user request explicitly involves cross-backend changes.
- Modifying a shared abstraction contract between backends.
- Verifying cross-backend parity or referencing an explicit reference implementation.
