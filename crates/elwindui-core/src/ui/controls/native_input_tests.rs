//! Mock-peer tests for the Core-owned input controls (Issue #313 stage ③): `CheckBox`,
//! `RadioButton`, `Slider`, `ToggleSwitch`, `TextBox`, `TextArea`, `PasswordBox`.

use super::*;
use crate::ui::native::{
    CheckBoxPeer, NativePeer, NativePeerFactory, PasswordBoxPeer, RadioButtonPeer, SliderPeer,
    TextAreaPeer, TextBoxPeer, ToggleSwitchPeer, clear_native_peer_factory,
    register_native_peer_factory,
};

type Slot<T> = RefCell<Option<Box<dyn Fn(T)>>>;

/// One mock that implements every input peer contract. It records each call (a password only
/// by length, never its value) and keeps the callbacks Core installs so a test can play the
/// native side.
struct MockInputPeer {
    log: Rc<RefCell<Vec<String>>>,
    on_check: Slot<CheckState>,
    on_bool: Slot<bool>,
    on_f32: Slot<f32>,
    on_text: Slot<String>,
    on_click: Slot<()>,
    on_submit: Slot<()>,
}

impl MockInputPeer {
    fn record(&self, entry: impl Into<String>) {
        self.log.borrow_mut().push(entry.into());
    }
}

/// Calls a stored callback without holding the slot borrow, like a backend dispatching a
/// native event.
fn fire<T>(slot: &Slot<T>, value: T) -> bool {
    let callback = slot.borrow_mut().take();
    let Some(callback) = callback else {
        return false;
    };
    callback(value);
    let mut slot = slot.borrow_mut();
    if slot.is_none() {
        *slot = Some(callback);
    }
    true
}

impl NativePeer for MockInputPeer {
    fn handle(&self) -> &dyn Any {
        &()
    }
    fn measure(&self, _available: Size) -> Size {
        self.record("measure");
        Size {
            width: 50.0,
            height: 20.0,
        }
    }
    fn apply_background(&self, _background: Option<&Brush>) -> bool {
        true
    }
    fn set_tooltip(&self, _tooltip: Option<&str>) {}
    fn supports_text_style(&self) -> bool {
        false
    }
    fn apply_text_style(&self, _style: &crate::graphics::CascadedTextStyle) -> bool {
        true
    }
    fn dispose(&self) {
        self.record("dispose");
        self.on_check.borrow_mut().take();
        self.on_bool.borrow_mut().take();
        self.on_f32.borrow_mut().take();
        self.on_text.borrow_mut().take();
        self.on_click.borrow_mut().take();
        self.on_submit.borrow_mut().take();
    }
}

fn store<T: 'static>(slot: &Slot<T>, callback: Option<Box<dyn Fn(T)>>) {
    *slot.borrow_mut() = callback;
}

impl CheckBoxPeer for MockInputPeer {
    fn set_text(&self, text: &str) {
        self.record(format!("text:{text}"));
    }
    fn set_checked(&self, checked: CheckState) {
        self.record(format!("checked:{checked:?}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.record(format!("enabled:{enabled}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(CheckState)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_check, callback);
    }
}

impl RadioButtonPeer for MockInputPeer {
    fn set_text(&self, text: &str) {
        self.record(format!("text:{text}"));
    }
    fn set_checked(&self, checked: bool) {
        self.record(format!("checked:{checked}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.record(format!("enabled:{enabled}"));
    }
    fn set_on_click(&self, callback: Option<Box<dyn Fn()>>) {
        self.record(format!("on_click:{}", callback.is_some()));
        store(
            &self.on_click,
            callback.map(|callback| Box::new(move |()| callback()) as Box<dyn Fn(())>),
        );
    }
}

impl SliderPeer for MockInputPeer {
    fn set_value(&self, value: f32) {
        self.record(format!("value:{value}"));
    }
    fn set_min(&self, min: f32) {
        self.record(format!("min:{min}"));
    }
    fn set_max(&self, max: f32) {
        self.record(format!("max:{max}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.record(format!("enabled:{enabled}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(f32)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_f32, callback);
    }
}

impl ToggleSwitchPeer for MockInputPeer {
    fn set_is_on(&self, is_on: bool) {
        self.record(format!("is_on:{is_on}"));
    }
    fn set_enabled(&self, enabled: bool) {
        self.record(format!("enabled:{enabled}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(bool)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_bool, callback);
    }
}

impl TextBoxPeer for MockInputPeer {
    fn set_text(&self, text: &str) {
        self.record(format!("text:{text}"));
    }
    fn set_placeholder(&self, text: &str) {
        self.record(format!("placeholder:{text}"));
    }
    fn set_read_only(&self, read_only: bool) {
        self.record(format!("read_only:{read_only}"));
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.record(format!("max_length:{max_length:?}"));
    }
    fn set_text_alignment(&self, alignment: TextAlignment) {
        self.record(format!("alignment:{alignment:?}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_text, callback);
    }
    fn set_on_submit(&self, callback: Option<Box<dyn Fn()>>) {
        self.record(format!("on_submit:{}", callback.is_some()));
        store(
            &self.on_submit,
            callback.map(|callback| Box::new(move |()| callback()) as Box<dyn Fn(())>),
        );
    }
}

impl TextAreaPeer for MockInputPeer {
    fn set_text(&self, text: &str) {
        self.record(format!("text:{text}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_text, callback);
    }
}

impl PasswordBoxPeer for MockInputPeer {
    fn set_password(&self, password: &str) {
        // Never the value: only its length.
        self.record(format!("password_len:{}", password.chars().count()));
    }
    fn set_placeholder(&self, text: &str) {
        self.record(format!("placeholder:{text}"));
    }
    fn set_max_length(&self, max_length: Option<u32>) {
        self.record(format!("max_length:{max_length:?}"));
    }
    fn set_reveal_enabled(&self, enabled: bool) {
        self.record(format!("reveal:{enabled}"));
    }
    fn set_on_change(&self, callback: Option<Box<dyn Fn(String)>>) {
        self.record(format!("on_change:{}", callback.is_some()));
        store(&self.on_text, callback);
    }
}

#[derive(Default)]
struct MockFactory {
    created: Cell<usize>,
    log: Rc<RefCell<Vec<String>>>,
    peers: RefCell<Vec<Rc<MockInputPeer>>>,
}

impl MockFactory {
    fn create(&self) -> Rc<MockInputPeer> {
        self.created.set(self.created.get() + 1);
        let peer = Rc::new(MockInputPeer {
            log: self.log.clone(),
            on_check: RefCell::new(None),
            on_bool: RefCell::new(None),
            on_f32: RefCell::new(None),
            on_text: RefCell::new(None),
            on_click: RefCell::new(None),
            on_submit: RefCell::new(None),
        });
        self.peers.borrow_mut().push(peer.clone());
        peer
    }

    fn peer(&self, index: usize) -> Rc<MockInputPeer> {
        self.peers.borrow()[index].clone()
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

impl NativePeerFactory for MockFactory {
    fn backend_name(&self) -> &'static str {
        "mock"
    }
    fn create_check_box(&self) -> Rc<dyn CheckBoxPeer> {
        self.create()
    }
    fn create_radio_button(&self) -> Rc<dyn RadioButtonPeer> {
        self.create()
    }
    fn create_slider(&self) -> Rc<dyn SliderPeer> {
        self.create()
    }
    fn create_toggle_switch(&self) -> Rc<dyn ToggleSwitchPeer> {
        self.create()
    }
    fn create_text_box(&self) -> Rc<dyn TextBoxPeer> {
        self.create()
    }
    fn create_text_area(&self) -> Rc<dyn TextAreaPeer> {
        self.create()
    }
    fn create_password_box(&self) -> Rc<dyn PasswordBoxPeer> {
        self.create()
    }
}

struct EmptyFactory;

impl NativePeerFactory for EmptyFactory {
    fn backend_name(&self) -> &'static str {
        "empty"
    }
}

fn install_mock() -> Rc<MockFactory> {
    let factory = Rc::new(MockFactory::default());
    register_native_peer_factory(factory.clone());
    factory
}

fn realize(node: &dyn UIElementExt) {
    node.measure(Size {
        width: 200.0,
        height: 100.0,
    });
}

/// Records every value a control's change callback reports.
fn recorder<T: Clone + 'static>() -> (Rc<RefCell<Vec<T>>>, Box<dyn Fn(T)>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    (
        seen,
        Box::new(move |value: T| sink.borrow_mut().push(value)),
    )
}

fn semantics(node: &dyn UIElementExt) -> AccessibilitySemantics {
    node.intrinsic_accessibility_semantics()
        .expect("native leaves set intrinsic semantics")
}

#[test]
fn check_box_realizes_once_and_separates_programmatic_and_user_changes() {
    let factory = install_mock();
    let check_box = CheckBox::new();
    check_box.set_text("Accept");
    check_box.set_checked(CheckState::Indeterminate);
    check_box.set_enabled(false);
    let (seen, sink) = recorder();
    check_box.set_on_checked_change(sink);
    assert_eq!(factory.created.get(), 0, "no peer before hosted layout");

    realize(check_box.as_ref());
    realize(check_box.as_ref());
    assert_eq!(factory.created.get(), 1);
    assert_eq!(
        factory.take_log(),
        [
            "text:Accept",
            "checked:Indeterminate",
            "enabled:false",
            "on_change:true",
            "measure",
            "measure",
        ]
    );

    // Programmatic: pushed, not notified.
    check_box.set_checked(CheckState::Unchecked);
    assert_eq!(factory.take_log(), ["checked:Unchecked"]);
    assert!(seen.borrow().is_empty());

    // User click: stored and notified once, never pushed back; an echo is dropped.
    let peer = factory.peer(0);
    fire(&peer.on_check, CheckState::Checked);
    fire(&peer.on_check, CheckState::Checked);
    assert_eq!(*seen.borrow(), [CheckState::Checked]);
    assert!(
        factory.take_log().is_empty(),
        "a user change is not echoed to the peer"
    );
    assert_eq!(
        semantics(check_box.as_ref()).state.checked,
        Some(AccessibilityCheckState::On)
    );

    // Accessibility activation is a user change that must reach the widget too.
    assert!(check_box.perform_accessibility_action(AccessibilityAction::Activate));
    assert_eq!(factory.take_log(), ["checked:Unchecked"]);
    assert_eq!(*seen.borrow(), [CheckState::Checked, CheckState::Unchecked]);
    clear_native_peer_factory();
}

#[test]
fn a_user_click_on_an_indeterminate_check_box_lands_on_checked() {
    let factory = install_mock();
    let check_box = CheckBox::new();
    let (seen, sink) = recorder();
    check_box.set_on_checked_change(sink);
    realize(check_box.as_ref());
    check_box.set_checked(CheckState::Indeterminate);
    factory.take_log();
    // A toolkit whose three-state cycle reports Unchecked (WinUI3) is corrected to Checked and
    // the widget is told; a toolkit that already reports Checked (AppKit) needs no correction.
    fire(&factory.peer(0).on_check, CheckState::Unchecked);
    assert_eq!(*seen.borrow(), [CheckState::Checked]);
    assert_eq!(factory.take_log(), ["checked:Checked"]);
    assert_eq!(
        semantics(check_box.as_ref()).state.checked,
        Some(AccessibilityCheckState::On)
    );

    check_box.set_checked(CheckState::Indeterminate);
    factory.take_log();
    fire(&factory.peer(0).on_check, CheckState::Checked);
    assert_eq!(*seen.borrow(), [CheckState::Checked, CheckState::Checked]);
    assert!(factory.take_log().is_empty());

    // From Checked, a click goes back to Unchecked as usual.
    fire(&factory.peer(0).on_check, CheckState::Unchecked);
    assert_eq!(
        *seen.borrow(),
        [
            CheckState::Checked,
            CheckState::Checked,
            CheckState::Unchecked
        ]
    );
    clear_native_peer_factory();
}

#[test]
fn input_callbacks_hold_only_weak_owners_and_dispose_once() {
    let factory = install_mock();
    let check_box = CheckBox::new();
    let (seen, sink) = recorder();
    check_box.set_on_change(sink);
    realize(check_box.as_ref());
    let peer = factory.peer(0);
    let callback = peer
        .on_check
        .borrow_mut()
        .take()
        .expect("callback installed");
    let weak = Rc::downgrade(&check_box);
    drop(check_box);
    assert!(
        weak.upgrade().is_none(),
        "the peer must not keep its control alive"
    );
    assert_eq!(
        factory
            .take_log()
            .iter()
            .filter(|e| *e == "dispose")
            .count(),
        1
    );
    callback(CheckState::Checked);
    assert!(seen.borrow().is_empty(), "a stale native event is a no-op");
    clear_native_peer_factory();
}

#[test]
fn radio_group_is_exclusive_and_notifies_only_changed_members() {
    let factory = install_mock();
    let buttons: Vec<Rc<RadioButton>> = (0..3).map(|_| RadioButton::new()).collect();
    let mut seen = Vec::new();
    for button in &buttons {
        button.set_group("size");
        let (log, sink) = recorder();
        button.set_on_checked_change(sink);
        seen.push(log);
        realize(button.as_ref());
    }
    buttons[0].set_checked(true);
    factory.take_log();

    // A native click on the second button selects it and unchecks the first only.
    fire(&factory.peer(1).on_click, ());
    assert_eq!(semantics(buttons[1].as_ref()).state.selected, Some(true));
    assert_eq!(semantics(buttons[0].as_ref()).state.selected, Some(false));
    assert_eq!(*seen[1].borrow(), [true]);
    assert_eq!(*seen[0].borrow(), [false]);
    assert!(
        seen[2].borrow().is_empty(),
        "an unchanged member is not notified"
    );
    assert_eq!(
        factory.take_log(),
        ["checked:false"],
        "the clicked widget already shows its state; only the sibling is pushed"
    );

    // Clicking the selected button again changes nothing.
    fire(&factory.peer(1).on_click, ());
    assert_eq!(*seen[1].borrow(), [true]);

    // Programmatic selection unchecks siblings without notifying the selected button.
    buttons[2].set_checked(true);
    assert_eq!(*seen[2].borrow(), Vec::<bool>::new());
    assert_eq!(*seen[1].borrow(), [true, false]);
    assert_eq!(semantics(buttons[1].as_ref()).state.selected, Some(false));
    clear_native_peer_factory();
}

#[test]
fn slider_pushes_range_before_value_and_drops_echoes() {
    let factory = install_mock();
    let slider = Slider::new();
    slider.set_min(10.0);
    slider.set_max(20.0);
    slider.set_value(15.0);
    let (seen, sink) = recorder();
    slider.set_on_value_change(sink);
    realize(slider.as_ref());
    assert_eq!(
        factory.take_log(),
        [
            "min:10",
            "max:20",
            "value:15",
            "enabled:true",
            "on_change:true",
            "measure"
        ]
    );
    let peer = factory.peer(0);
    fire(&peer.on_f32, 15.0);
    assert!(
        seen.borrow().is_empty(),
        "an echo of the stored value is dropped"
    );
    fire(&peer.on_f32, 17.5);
    assert_eq!(*seen.borrow(), [17.5]);
    assert!(slider.perform_accessibility_action(AccessibilityAction::Increment));
    assert_eq!(*seen.borrow(), [17.5, 18.5]);
    assert_eq!(factory.take_log(), ["value:18.5"]);
    let range = semantics(slider.as_ref()).state.value_range.expect("range");
    assert_eq!((range.value, range.min, range.max), (18.5, 10.0, 20.0));
    clear_native_peer_factory();
}

#[test]
fn toggle_switch_separates_programmatic_and_user_changes() {
    let factory = install_mock();
    let toggle = ToggleSwitch::new();
    let (seen, sink) = recorder();
    toggle.set_on_is_on_change(sink);
    realize(toggle.as_ref());
    factory.take_log();
    toggle.set_is_on(true);
    assert!(seen.borrow().is_empty());
    let peer = factory.peer(0);
    fire(&peer.on_bool, true);
    fire(&peer.on_bool, false);
    assert_eq!(*seen.borrow(), [false]);
    assert_eq!(
        semantics(toggle.as_ref()).state.checked,
        Some(AccessibilityCheckState::Off)
    );
    clear_native_peer_factory();
}

#[test]
fn text_box_keeps_platform_defaults_drops_echoes_and_submits_once() {
    let factory = install_mock();
    let text_box = TextBox::new();
    text_box.set_text("hello");
    text_box.set_placeholder("Name");
    let (seen, sink) = recorder();
    text_box.set_on_text_change(sink);
    let submits = Rc::new(Cell::new(0));
    text_box
        .as_ui_element()
        .register_routed_handler::<crate::input::KeyEventArgs>(
            "on_key_down",
            Box::new({
                let submits = submits.clone();
                move |args, _| {
                    if args.key == crate::input::Key::Enter {
                        submits.set(submits.get() + 1);
                    }
                }
            }),
        );
    realize(text_box.as_ref());
    assert_eq!(
        factory.take_log(),
        [
            "placeholder:Name",
            "read_only:false",
            "max_length:None",
            "text:hello",
            "on_change:true",
            "on_submit:true",
            "measure",
        ],
        "an unset alignment is not pushed, so the platform default stays"
    );

    let peer = factory.peer(0);
    // The native echo of a programmatic set is dropped; a real edit is notified once.
    fire(&peer.on_text, "hello".to_string());
    fire(&peer.on_text, "hello!".to_string());
    assert_eq!(*seen.borrow(), ["hello!".to_string()]);
    assert_eq!(
        semantics(text_box.as_ref()).value.as_deref(),
        Some("hello!")
    );

    fire(&peer.on_submit, ());
    assert_eq!(submits.get(), 1);

    // An assistive-technology edit is a user change.
    assert!(
        text_box.perform_accessibility_action(AccessibilityAction::SetText("typed".to_string()))
    );
    assert_eq!(*seen.borrow(), ["hello!".to_string(), "typed".to_string()]);
    assert_eq!(factory.take_log(), ["text:typed"]);
    clear_native_peer_factory();
}

#[test]
fn text_area_drops_echoes_and_notifies_user_edits() {
    let factory = install_mock();
    let text_area = TextArea::new();
    let (seen, sink) = recorder();
    text_area.set_on_text_change(sink);
    realize(text_area.as_ref());
    text_area.set_text("a\nb");
    let peer = factory.peer(0);
    fire(&peer.on_text, "a\nb".to_string());
    fire(&peer.on_text, "a\nbc".to_string());
    assert_eq!(*seen.borrow(), ["a\nbc".to_string()]);
    clear_native_peer_factory();
}

/// No-leak policy: the mock records only lengths, and every assertion uses a content-free
/// message, so a failure never prints the password.
#[test]
fn password_box_never_exposes_the_value_and_keeps_reveal_default() {
    let factory = install_mock();
    let password_box = PasswordBox::new();
    password_box.set_password("hunter2");
    let lengths = Rc::new(RefCell::new(Vec::new()));
    {
        let lengths = lengths.clone();
        password_box.set_on_password_change(Box::new(move |password: String| {
            lengths.borrow_mut().push(password.chars().count());
        }));
    }
    realize(password_box.as_ref());
    let log = factory.take_log();
    assert!(
        log == [
            "placeholder:",
            "max_length:None",
            "password_len:7",
            "on_change:true",
            "measure"
        ],
        "unexpected realization calls (an unset reveal must not be pushed)"
    );
    let peer = factory.peer(0);
    fire(&peer.on_text, "hunter2".to_string());
    fire(&peer.on_text, String::new());
    assert!(
        *lengths.borrow() == vec![0],
        "the change callback fired the wrong number of times or with the wrong length"
    );
    assert!(
        semantics(password_box.as_ref()).value.is_none(),
        "the password must never be exposed through accessibility semantics"
    );
    clear_native_peer_factory();
}

#[test]
#[should_panic(expected = "the `empty` backend does not support the native control `TextBox`")]
fn an_input_control_on_a_backend_without_it_panics() {
    register_native_peer_factory(Rc::new(EmptyFactory));
    realize(TextBox::new().as_ref());
}
