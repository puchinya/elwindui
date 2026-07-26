"""Exercise the running theme-demo through Windows UI Automation.

Install the test-only dependency with `python -m pip install uiautomation`, start
`target/debug/theme-demo.exe`, and then run this script. The test intentionally uses the
UI Automation Invoke pattern instead of screen coordinates.
"""

from __future__ import annotations

import sys
import time

import uiautomation as auto


WINDOW_TITLE = "elwindui Theme Demo"
auto.Logger.SetLogFile("")


def text_names(window: auto.Control) -> list[str]:
    """Return the current names of all text nodes below `window`."""

    return [
        control.Name
        for control, _depth in auto.WalkControl(window, includeTop=False, maxDepth=8)
        if control.ControlTypeName == "TextControl"
    ]


def invoke_and_verify(
    window: auto.Control,
    button_name: str,
    expected_label: str,
) -> str:
    """Invoke one button and return the resulting revision label."""

    button = window.ButtonControl(Name=button_name)
    if not button.Exists(2):
        raise AssertionError(f"UIA button not found: {button_name}")
    if not button.GetInvokePattern().Invoke():
        raise AssertionError(f"UIA Invoke failed: {button_name}")
    time.sleep(0.2)

    names = text_names(window)
    if expected_label not in names:
        raise AssertionError(
            f"{button_name} did not expose {expected_label!r}; text nodes: {names!r}"
        )
    revisions = [name for name in names if name.isdigit()]
    if not revisions:
        raise AssertionError(f"{button_name} did not expose a numeric revision")
    return revisions[-1]


def main() -> None:
    """Run the complete appearance/variant transition sequence."""

    sys.stdout.reconfigure(encoding="utf-8")
    window = auto.WindowControl(Name=WINDOW_TITLE, searchDepth=1)
    if not window.Exists(5):
        raise AssertionError(f"UIA window not found: {WINDOW_TITLE}")

    sequence = [
        ("Ocean", "Ocean"),
        ("Solarized", "Solarized"),
        ("Default", "Default / platform_default"),
        ("Dark", "Dark requested"),
        ("Light", "Light requested"),
        ("System", "System / backend reported"),
    ]
    revisions: list[int] = []
    for button_name, expected_label in sequence:
        revision = invoke_and_verify(window, button_name, expected_label)
        revisions.append(int(revision))
        print(f"{button_name}: revision {revision}, label {expected_label!r}")

    if revisions != sorted(revisions) or len(set(revisions)) != len(revisions):
        raise AssertionError(f"revisions were not strictly increasing: {revisions}")

    disabled = window.ButtonControl(Name="Disabled native state")
    if not disabled.Exists(2) or disabled.IsEnabled:
        raise AssertionError("the disabled-state sample is not exposed as disabled through UIA")

    tab = window.TabItemControl(Name="Second")
    if not tab.Exists(2):
        raise AssertionError("the nested TabView did not expose its second tab through UIA")

    print("UIA theme sequence passed")


if __name__ == "__main__":
    main()
