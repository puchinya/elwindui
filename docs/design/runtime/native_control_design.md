# Native control design

Related specification: [`../../specs/ui_spec.md`](../../specs/ui_spec.md).

## Boundary

`NativeControl` represents an ElwindUI element whose leaf presentation is owned by an OS widget. Common code owns public properties, logical relationships, and events; backend code owns widget creation, property translation, native event targets, and disposal.

The backend layering remains `native_ui -> inner -> host -> render -> ffi`. Public UI modules do not expose AppKit, WinUI, WinRT, or Objective-C types.

## Hosting and owner mapping

A host maps each native widget to exactly one ElwindUI owner. Helper views used for clipping, scrolling, or content roots are backend implementation nodes and forward ownership to the public control.

`ScrollView` uses a native scroll host, an ElwindUI content root, and the public content subtree. This isolates native viewport mechanics from the public content model and provides an independent layout/render boundary.

## Reconciliation

Native children are reconciled from the active visual tree. Stable controls retain their widget handles across ordinary property updates and host deactivation. Removed controls detach targets/subscriptions and release handles.

Property synchronization is either push-based for direct properties or pull/revision-based when inherited Theme or text values must be resolved. Each property has one authoritative synchronization path.

## Events and layout

Native callbacks are translated to backend-neutral events and dispatched through the input router. Native natural size participates in the common measure pass; arrange applies final bounds without redefining the public layout contract.

Control templates and fully custom-drawn controls remain common runtime concerns. A native widget is selected only where the public control contract intentionally maps to OS behavior.
