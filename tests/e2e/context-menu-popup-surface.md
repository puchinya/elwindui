# Context Menu / PopupSurface

Durable product E2E case for Issue [#157](https://github.com/puchinya/elwindui/issues/157):
context requests, native and custom-rendered context menus, rich context popups, PopupSurface
placement and lifecycle, and the native menu icon paths added by PR #171. WinUI3 execution on a
single monitor is required for #157. Multi-monitor and mixed-DPI placement are owned by
[#297](https://github.com/puchinya/elwindui/issues/297) and are not part of this case. AppKit may
reuse the same rows later; the expected results below are backend-neutral unless a row says
otherwise.

## Fixture

`examples/controls-demo`, built on the candidate HEAD. Everything this case needs is on the
**Context Menu** tab and the window's **Icons** menu bar menu:

| Fixture element | Purpose |
|---|---|
| `【Native Context Menu Target — Right-click here】` (self-drawn TextBlock) | native `MenuFlyout` with System icons (Cut/Copy/Paste/Delete), Vector, Encoded PNG, Rgba8 and Dynamic icons, a `Ctrl+X` shortcut on Cut and a disabled item |
| `【Custom-rendered Context Menu Target — Right-click here】` | custom-rendered menu on a PopupSurface: shortcut `S`, a disabled item, raster and vector icons |
| `【Custom Context Popup Target — Right-click here】` | rich popup (`rich_popup.rs`): Button, CheckBox, Slider, native TextBox, `Close via PopupDismissAction`, and two text lines showing Environment values |
| `Native Button target` | NativeControl context request (`Button Context Action`) |
| `[Child without own menu - Right-click here]` | ancestor context lookup (`Ancestor Action` comes from the parent layout) |
| `Cycle dynamic icon` + label | cycles the Dynamic icon System(Edit) -> Rgba8 -> none |
| `Context Menu Event Log` | one line per activated item, e.g. `Native > Copy selected` |

Launch the demo with `ELWINDUI_CONTROLS_DEMO_POPUP_TRACE_PATH` set to an absolute file in the run
directory. The rich popup appends one JSON line per event there:
`popup_build` (with `inherited`, `popup_scoped`, `owner`, `popup_dismiss`, `owner_popup_dismiss`),
`popup_unmount`, `popup_button_click`, `popup_checkbox_change`, `popup_slider_change`,
`popup_textbox_change` (length only), `popup_textbox_focus`, `popup_dismiss_action` and
`owner_unmount` (the popup's owner TextBlock).

Use `tools/windows-ui-driver/windows-ui-driver.ps1` and the tester procedure in
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md). Right-click is
`point-click --button right` at a window-relative coordinate recomputed from a fresh
`list-windows`/`search`. Popups and flyouts must be captured with `capture-window --capture-screen`.
Keyboard input uses `send-keys --via send-input`.

## Rows

Every row records PASS / FAIL / NOT RUN / BLOCKED with a screenshot and, where named, the event log
text or trace lines. Seeing a driver `success: true` alone is never PASS.

### Context request and native menu

- **CMP-01 secondary click, native menu.** Right-click the native target. PASS: a native flyout
  appears at the pointer showing every item with its icon (System icons, a blue triangle for
  Vector, a green square for Encoded PNG, an orange square for Rgba8, the Edit symbol for
  Dynamic), `Ctrl+X` beside Cut, and `Disabled (native)` greyed out. The flyout's top-left is within a few pixels of the
  right-click point (or flipped to stay on screen). Click `Copy`; the log gains
  `Native > Copy selected` and the flyout closes. Clicking the disabled item adds no log line.
- **CMP-02 Shift+F10.** A keyboard context request targets the element with keyboard focus, and
  the self-drawn TextBlock targets cannot take focus. Click `Native Button target` (one
  `NativeButton > clicked` line), then send `shift+f10`. PASS: the button's native menu opens at
  the button (not at an unrelated place). `esc` closes it without a log line.
- **CMP-03 Menu key.** As CMP-02 with `vk=0x5D` (Application/Menu key).
- **CMP-04 NativeControl.** Right-click `Native Button target`. PASS: a menu with
  `Button Context Action`; selecting it logs `NativeButton > Button Context Action`, and the button
  itself does not log `NativeButton > clicked`.
- **CMP-05 ancestor lookup.** Right-click the child text. PASS: the menu shows `Ancestor Action`;
  selecting it logs `Ancestor > Ancestor Action`.

### Custom-rendered menu

- **CMP-06 open.** Right-click the custom target. PASS: an ElwindUI-drawn menu on a popup shows the
  five items with icons, the `S` shortcut, and `Disabled Action` visibly disabled.
- **CMP-07 hover.** Move the pointer over an enabled item without clicking, with the driver's
  `hover --selector <item> --dwell-ms 800` (real pointer movement through `winapp ui hover`), then
  capture. PASS: that item is highlighted and nothing is activated (no log line, menu stays open).
  `hover` needs a UIA selector for the item; if `search` inside the open menu finds none, record
  BLOCKED with the search output (the driver has no coordinate pointer-move primitive) — never
  substitute a click.
- **CMP-08 keyboard.** With the menu open, send `down`, `down`, ... PASS: the highlight moves and
  skips `Disabled Action`; `up` moves back; `enter` activates the highlighted item (log line) and
  closes the menu.
- **CMP-09 Escape.** Reopen, send `esc`. PASS: the menu closes and no log line is added.
- **CMP-10 click activation.** Reopen and click `Custom Action 2 (user image icon)`. PASS: log
  `Custom > Action 2 selected` and the menu closes. Clicking `Disabled Action` neither logs nor
  closes.

### Rich context popup

- **CMP-11 contents.** Right-click the popup target. PASS: the popup shows its title, the
  Button/CheckBox/Slider/TextBox and the close button; the popup text reads
  `Env inherited: app-root / popup-scoped: popup-override / owner: app-root` and
  `popup_dismiss: Some / owner popup_dismiss: None`; the trace has one `popup_build` with the same
  values.
- **CMP-12 controls.** Click `Popup Button` twice, toggle the CheckBox, change the Slider, click the
  TextBox and type `abc`. PASS: the popup's own labels show `clicks: 2`, `Checked`, a changed slider
  value and `length: 3`, the trace shows the matching events including `popup_textbox_focus`, and
  the popup stays open throughout.
- **CMP-13 PopupDismissAction.** Click `Close via PopupDismissAction`. PASS: the popup closes and the
  trace shows `popup_dismiss_action` followed by exactly one `popup_unmount` for that instance.

### PopupSurface placement (single monitor)

WinUI 3 presents each popup in a `PopupHost` window that is larger than the popup content by a
shadow margin (observed: 10 px left/right, 2 px top, 18 px bottom at 100 % scale). Judge placement
on the popup content rectangle seen in the screenshot (or the host rect minus that margin), not on
the raw `PopupHost` rect. With `ELWINDUI_WINUI3_DIAGNOSTICS=1` the demo also writes
`[diag popup] shown ... position/size/anchor/work_area` and `[diag popup] re-placed size/position`
lines to stderr (launch with stderr redirected) — record them as supporting evidence.

- **CMP-14 outside client area.** Resize the window so the popup target is near the window's
  bottom edge and right-click it. PASS: the popup extends beyond the window bounds and is fully
  drawn there.
- **CMP-15 over NativeControl.** Open the rich popup so it overlaps a native control (for example
  the `Native Button target` or the log ScrollView). PASS: the popup is drawn in front of it.
- **CMP-16 right edge flip.** Move the window so the right-click point on the rich popup target
  is less than the popup width (about 560 px) from the work-area right edge, e.g. a click at
  x = work-area right − 200. PASS: the popup content opens to the left of the pointer (its right
  edge at the pointer) and stays fully inside the work area.
- **CMP-17 bottom edge flip.** As CMP-16 against the bottom edge. PASS: the popup opens above the
  pointer.
- **CMP-18 work-area clamp.** Right-click just above the taskbar. PASS: no part of the popup covers
  the taskbar.

### Lifecycle

- **CMP-19 outside click.** Open the rich popup, click an empty window area. PASS: it closes; the
  trace shows exactly one `popup_unmount` for that instance.
- **CMP-20 Escape.** Open the custom menu, send `esc` (CMP-09 evidence may be reused). Open the rich
  popup, send `esc`; record whether it closes (it is light-dismiss with no initial focus) and its
  unmount count.
- **CMP-21 replacement.** Per `docs/specs/ui_spec.md` (Dismiss と置換): open the rich popup, then
  right-click the custom-rendered target once. PASS: with that single right-click the rich popup
  closes (exactly one `popup_unmount`) and the custom menu opens at the new pointer position; only
  one popup is visible. Also open the native menu and right-click the custom target once: the
  native flyout closes and the custom menu opens. Then open the rich popup and left-click the
  `Native Button target`: the popup closes and the log gains no `NativeButton > clicked` line
  (a primary outside click only dismisses).
- **CMP-22 focus restoration.** Click `Native Button target` (one `NativeButton > clicked` line),
  send `shift+f10` to open its menu, send `esc`, then send `space`. PASS: a second
  `NativeButton > clicked` line appears, showing keyboard focus returned to the button.
- **CMP-23 repeated open/close.** Open and close the rich popup five times (outside click). PASS:
  five `popup_build` and five `popup_unmount` lines with matching instance numbers, no duplicate
  items, no crash. Repeat three native-menu open/Escape cycles: no duplicated items.
- **CMP-24 owner Window close.** Open the rich popup and close the Window with `alt+f4`
  (`--allow-system-keys`). PASS: the process exits without force and the trace shows the open
  instance's `popup_unmount` before `owner_unmount`.

### Native menu icons (PR #171)

- **CMP-25 MenuBar icons.** Open `Icons` in the menu bar. PASS: System (Copy), Encoded PNG, Rgba8,
  Vector and Dynamic items show their icons, `Disabled (MenuBar)` is greyed, and selecting
  `Rgba8 Icon` logs `MenuBar > Rgba8 Icon selected`.
- **CMP-26 live replace/clear.** Click `Cycle dynamic icon` once (label `Rgba8 raster`), reopen
  `Icons` and the native context menu: the Dynamic item shows the orange square in both. Click it
  again (label `(none)`): the Dynamic item shows no icon in both, and still logs when selected.
  Click again (label `System(Edit)`): the Edit symbol returns. Each reopen of the context menu shows
  the latest icon.

### Regression

- **CMP-27** On the other tabs: a Button click logs, TextArea/TextBox accept typed text, TabView
  switching works, the Dropdown opens and changes selection. PASS when each behaves as before.
- **CMP-28 pointer events and `on_right_tapped`.** The secondary-click dispatch changed in #157,
  so verify the self-drawn pointer route in `examples/custom-controls-demo` on the final HEAD: on
  Probe A (see [`pointer-cancellation-capture-loss.md`](pointer-cancellation-capture-loss.md)), a
  real left click increases `pressed`, `released` and `tapped` by exactly one each, and a real
  right click increases `right_tapped` by exactly one (no double count, no context menu). Also run
  [`self-drawn-pointer-input.md`](self-drawn-pointer-input.md) SDP-01 (CustomTabView real-pointer
  selection).

## Cleanup

`terminate --pid <pid> --timeout 5` after the last row (CMP-24 ends the process itself; record
whether it exited on its own). Evidence lives under `.agent-state/issues/157/e2e/<head>/<run-id>/`.
