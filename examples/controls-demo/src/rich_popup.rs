//! The "Rich Context Popup" content shown by the Context Menu tab's third target
//! (`context_popup: vm.custom_popup_template`).
//!
//! Hosts real interactive children (Button / CheckBox / Slider / native TextBox) plus a
//! `PopupDismissAction`-driven close button, and shows on the popup itself the values a tester
//! needs to judge Environment inheritance and popup-scope isolation. Each build and each unmount
//! is reported through [`trace`], so lifecycle ordering (exactly one unmount per open, popup
//! unmounted before its owner on Window close) is observable from outside the process.
//!
//! Tracing is opt-in: with `ELWINDUI_CONTROLS_DEMO_POPUP_TRACE_PATH` set to an absolute file path,
//! every event is appended there as one JSON line; without it, events go to stderr only.

use elwindui::core::environment::{EnvironmentKey, application_environment};
use elwindui::core::ui::popup::PopupDismissActionKey;
use elwindui::core::ui::{
    ButtonExt, CheckBoxExt, CheckState, LayoutExt, SliderExt, TextBlockExt, TextBoxExt,
    UIElementExt, VerticalLayoutExt, ViewBuildContext,
};
use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;

/// A demo-local Environment key, set once on the application root Environment by
/// [`install_demo_environment`]. The popup content reads it through its own (derived) Environment
/// to show inheritance, and overrides it on that derived Environment to show the override never
/// reaches the owner.
pub(crate) struct DemoInheritedLabelKey;

impl EnvironmentKey for DemoInheritedLabelKey {
    type Value = String;

    fn default_value() -> Self::Value {
        "(default)".to_string()
    }
}

/// Value the application root Environment carries for [`DemoInheritedLabelKey`].
pub(crate) const APP_ENVIRONMENT_LABEL: &str = "app-root";
/// Value the popup-scoped Environment overrides [`DemoInheritedLabelKey`] with.
const POPUP_ENVIRONMENT_LABEL: &str = "popup-override";

pub(crate) fn install_demo_environment() {
    application_environment().set::<DemoInheritedLabelKey>(APP_ENVIRONMENT_LABEL.to_string());
}

thread_local! {
    static TRACE_SEQ: Cell<u64> = const { Cell::new(0) };
    static POPUP_INSTANCE: Cell<u64> = const { Cell::new(0) };
    static OWNER_HOOK_REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// Records one lifecycle/interaction event (see this module's doc comment).
pub(crate) fn trace(event: &str, detail: &str) {
    let seq = TRACE_SEQ.with(|s| {
        let next = s.get() + 1;
        s.set(next);
        next
    });
    let line = format!(
        "{{\"seq\":{seq},\"event\":\"{}\",\"detail\":\"{}\"}}",
        json_escape(event),
        json_escape(detail)
    );
    eprintln!("[controls-demo popup-trace] {line}");
    if let Some(path) = std::env::var_os("ELWINDUI_CONTROLS_DEMO_POPUP_TRACE_PATH") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect()
}

/// Registers (once per process) an unmount hook on the popup's owner element, so the tester can
/// compare the owner subtree's unmount position in the trace with the popup's own (owner Window
/// close case).
fn trace_owner_unmount_once(owner: &Rc<dyn UIElementExt>) {
    if OWNER_HOOK_REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    owner.add_unmount_hook(Box::new(|| trace("owner_unmount", "context_popup_target")));
}

fn add(layout: &Rc<elwindui::core::ui::VerticalLayout>, child: Rc<dyn UIElementExt>) {
    LayoutExt::children(&**layout).add(child);
}

fn text(value: &str) -> Rc<elwindui::core::ui::TextBlock> {
    let block = elwindui::core::ui::TextBlock::new();
    block.set_text(value);
    block.text_style.set_foreground(Some(
        elwindui::core::graphics::Color::rgb(20, 20, 25).into(),
    ));
    block
}

/// Builds one independent popup instance. Each call is a new instance with its own unmount hook.
pub(crate) fn build_rich_context_popup(ctx: ViewBuildContext) -> Option<Rc<dyn UIElementExt>> {
    let instance = POPUP_INSTANCE.with(|n| {
        let next = n.get() + 1;
        n.set(next);
        next
    });
    let instance_label = format!("popup#{instance}");

    let inherited = ctx.environment.get::<DemoInheritedLabelKey>();
    let dismiss = ctx.environment.get::<PopupDismissActionKey>();
    ctx.environment
        .set::<DemoInheritedLabelKey>(POPUP_ENVIRONMENT_LABEL.to_string());
    let popup_scoped = ctx.environment.get::<DemoInheritedLabelKey>();
    let owner = ctx.owner.upgrade();
    if let Some(owner) = owner.as_ref() {
        trace_owner_unmount_once(owner);
    }
    let owner_env = owner.as_ref().map(|o| o.effective_environment());
    let owner_label = owner_env
        .as_ref()
        .map(|env| env.get::<DemoInheritedLabelKey>())
        .unwrap_or_else(|| "(no owner)".to_string());
    let owner_dismiss = owner_env
        .as_ref()
        .map(|env| env.get::<PopupDismissActionKey>().is_some());

    trace(
        "popup_build",
        &format!(
            "{instance_label} inherited={inherited} popup_scoped={popup_scoped} \
             owner={owner_label} popup_dismiss={} owner_popup_dismiss={owner_dismiss:?}",
            dismiss.is_some()
        ),
    );

    let layout = elwindui::core::ui::VerticalLayout::new();
    layout.set_margin(12.0);
    // A popup draws no chrome of its own; without a background the window shows through.
    layout.set_background(Some(
        elwindui::core::graphics::Color::rgb(246, 246, 248).into(),
    ));
    layout.set_spacing(6.0);

    add(
        &layout,
        text(&format!("✨ Rich Context Popup ({instance_label})")),
    );
    add(
        &layout,
        text(&format!(
            "Env inherited: {inherited} / popup-scoped: {popup_scoped} / owner: {owner_label}"
        )),
    );
    add(
        &layout,
        text(&format!(
            "popup_dismiss: {} / owner popup_dismiss: {}",
            if dismiss.is_some() { "Some" } else { "None" },
            match owner_dismiss {
                Some(true) => "Some",
                Some(false) => "None",
                None => "(no owner)",
            }
        )),
    );

    let status = text("Popup Button clicks: 0");
    let clicks = Rc::new(Cell::new(0u32));
    let button = elwindui::ui::Button::new();
    button.set_text("Popup Button");
    button.set_on_click(Box::new({
        let status = Rc::clone(&status);
        let clicks = Rc::clone(&clicks);
        move || {
            clicks.set(clicks.get() + 1);
            status.set_text(&format!("Popup Button clicks: {}", clicks.get()));
            trace("popup_button_click", &clicks.get().to_string());
        }
    }));
    add(&layout, button as Rc<dyn UIElementExt>);
    add(&layout, Rc::clone(&status) as Rc<dyn UIElementExt>);

    let check_label = text("Popup CheckBox: Unchecked");
    let check_box = elwindui::ui::CheckBox::new();
    check_box.set_text("Popup CheckBox");
    check_box.set_on_change(Box::new({
        let check_label = Rc::clone(&check_label);
        move |state| {
            let value = match state {
                CheckState::Unchecked => "Unchecked",
                CheckState::Checked => "Checked",
                CheckState::Indeterminate => "Indeterminate",
            };
            check_label.set_text(&format!("Popup CheckBox: {value}"));
            trace("popup_checkbox_change", value);
        }
    }));
    add(&layout, check_box as Rc<dyn UIElementExt>);
    add(&layout, Rc::clone(&check_label) as Rc<dyn UIElementExt>);

    let slider_label = text("Popup Slider: 0.50");
    let slider = elwindui::ui::Slider::new();
    slider.set_width(200.0);
    slider.set_value(0.5);
    slider.set_on_change(Box::new({
        let slider_label = Rc::clone(&slider_label);
        move |value| {
            slider_label.set_text(&format!("Popup Slider: {value:.2}"));
            trace("popup_slider_change", &format!("{value:.2}"));
        }
    }));
    add(&layout, slider as Rc<dyn UIElementExt>);
    add(&layout, Rc::clone(&slider_label) as Rc<dyn UIElementExt>);

    // Length only, never the typed value — the same no-leak convention as the PasswordBox tab.
    let text_box_label = text("Popup TextBox length: 0");
    let text_box = elwindui::ui::TextBox::new();
    text_box.set_placeholder("Popup TextBox (native)");
    text_box.set_width(200.0);
    text_box.set_on_change(Box::new({
        let text_box_label = Rc::clone(&text_box_label);
        move |value| {
            let len = value.chars().count();
            text_box_label.set_text(&format!("Popup TextBox length: {len}"));
            trace("popup_textbox_change", &len.to_string());
        }
    }));
    text_box.register_routed_handler(
        "on_got_focus",
        Box::new(|_: &(), _| trace("popup_textbox_focus", "got")),
    );
    add(&layout, text_box as Rc<dyn UIElementExt>);
    add(&layout, Rc::clone(&text_box_label) as Rc<dyn UIElementExt>);

    let dismiss_button = elwindui::ui::Button::new();
    dismiss_button.set_text("Close via PopupDismissAction");
    dismiss_button.set_on_click(Box::new(move || {
        trace("popup_dismiss_action", "invoked");
        if let Some(dismiss) = dismiss.as_ref() {
            dismiss.dismiss();
        }
    }));
    add(&layout, dismiss_button as Rc<dyn UIElementExt>);

    let root = layout as Rc<dyn UIElementExt>;
    root.add_unmount_hook(Box::new(move || trace("popup_unmount", &instance_label)));
    Some(root)
}
