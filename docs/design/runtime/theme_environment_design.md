# Theme and Environment implementation design

Related specification: [`../../specs/theme_environment_spec.md`](../../specs/theme_environment_spec.md).

## Context

`ThemeContext` is attached at application and Window host boundaries. Elements resolve the nearest context through the visual host relation while logical inheritance remains available to property cascades. A Window override derives from, rather than mutating, the application default.

The context owns the selected definition, variant, appearance preference, resolved appearance, and monotonically increasing revision. Handles allow controls to observe a context without owning the UI tree.

## Resolution

Typed `ThemeToken<T>` lookup first checks the selected concrete token. Missing standard concrete tokens fall back through the declared base-token chain. An explicit `PlatformDefault` terminates lookup.

Environment values use the same nearest-context principle but retain separate keys and value types. Neither lookup exposes backend helper nodes.

## Change propagation

Controllers update context state and increment the revision only when an observable value changes. Generated `theme!` bindings record tokens and their `ThemeChangeImpact`, allowing paint, measure, or native-style invalidation to be scheduled narrowly.

Backend appearance observers translate OS changes into `ThemeAppearance` and update only contexts using `System` preference.

## Backend synchronization

Common resolution produces `Value` or `PlatformDefault`. AppKit adapters map them to system fonts/colors/appearance and layer properties. WinUI 3 adapters use dependency-property set/clear operations and `RequestedTheme`. Backend status documents record unsupported mappings; they do not change resolution semantics.
