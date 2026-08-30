use super::core::input::{MouseButton, PointerEventArgs};
use super::core::ui::UIElementExt;
use super::{
    Orientation, SplitterDragCompletedEventArgs, SplitterDragDelta, SplitterDragDeltaEventArgs,
    SplitterDragStarted, SplitterDragStartedEventArgs, SplitterGesture,
    weak_self_from_visual_owner,
};
use std::rc::Rc;

#[elwindui::component(inherits Control)]
pub struct CustomSplitter {
    #[prop(default = Orientation::Horizontal)]
    orientation: Orientation,
    #[state(default = None)]
    drag_started_callback: Option<Rc<dyn Fn(SplitterDragStartedEventArgs)>>,
    #[state(default = None)]
    drag_delta_callback: Option<Rc<dyn Fn(SplitterDragDeltaEventArgs)>>,
    #[state(default = None)]
    drag_completed_callback: Option<Rc<dyn Fn(SplitterDragCompletedEventArgs)>>,
    #[state(default = None)]
    gesture: Option<SplitterGesture>,
    template: template_view!(|this: Self| {
        on_mount {
            this.bind_pointer_handlers();
        }
        match orientation {
            Orientation::Horizontal => {
                Rectangle {
                    width: 6.0
                    fill: "#d0d0d0"
                }
            }
            Orientation::Vertical => {
                Rectangle {
                    height: 6.0
                    fill: "#d0d0d0"
                }
            }
        }
    }),
}

#[elwindui::component]
impl CustomSplitter {}

impl CustomSplitter {
    /// Creates a splitter with the default horizontal orientation.
    pub fn new_splitter() -> Rc<Self> {
        Self::new()
    }

    /// Registers the splitter-start callback.
    pub fn set_on_drag_started(&self, callback: Box<dyn Fn(SplitterDragStartedEventArgs)>) {
        self.set_drag_started_callback(Some(Rc::from(callback)));
    }

    /// Registers the incremental splitter-delta callback.
    pub fn set_on_drag_delta(&self, callback: Box<dyn Fn(SplitterDragDeltaEventArgs)>) {
        self.set_drag_delta_callback(Some(Rc::from(callback)));
    }

    /// Registers the splitter-completed callback.
    pub fn set_on_drag_completed(&self, callback: Box<dyn Fn(SplitterDragCompletedEventArgs)>) {
        self.set_drag_completed_callback(Some(Rc::from(callback)));
    }

    fn bind_pointer_handlers(&self) {
        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, _| {
                if event.button != Some(MouseButton::Left) {
                    return;
                }
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                splitter.set_gesture(Some(SplitterGesture {
                    orientation: splitter.orientation(),
                    position: event.position,
                    screen_position: event.screen_position,
                    cumulative_delta: 0.0,
                }));
                if let Some(callback) = splitter.drag_started_callback() {
                    callback(SplitterDragStarted {
                        position: event.position,
                        screen_position: event.screen_position,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_moved",
            Box::new(move |event, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(mut gesture) = splitter.gesture() else {
                    return;
                };
                let delta = match gesture.orientation {
                    Orientation::Horizontal => event.position.x - gesture.position.x,
                    Orientation::Vertical => event.position.y - gesture.position.y,
                };
                gesture.position = event.position;
                gesture.screen_position = event.screen_position;
                gesture.cumulative_delta += delta;
                splitter.set_gesture(Some(gesture.clone()));
                if delta == 0.0 {
                    return;
                }
                if let Some(callback) = splitter.drag_delta_callback() {
                    callback(SplitterDragDelta {
                        delta,
                        cumulative_delta: gesture.cumulative_delta,
                        position: event.position,
                        screen_position: event.screen_position,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(mut gesture) = splitter.gesture() else {
                    return;
                };
                let final_delta = match gesture.orientation {
                    Orientation::Horizontal => event.position.x - gesture.position.x,
                    Orientation::Vertical => event.position.y - gesture.position.y,
                };
                gesture.position = event.position;
                gesture.screen_position = event.screen_position;
                gesture.cumulative_delta += final_delta;
                splitter.set_gesture(None);
                if let Some(callback) = splitter.drag_completed_callback() {
                    callback(SplitterDragCompletedEventArgs {
                        cumulative_delta: gesture.cumulative_delta,
                        position: gesture.position,
                        screen_position: gesture.screen_position,
                        canceled: false,
                    });
                }
            }),
        );

        let weak_self = self.weak_self();
        self.register_routed_handler::<PointerEventArgs>(
            "on_pointer_canceled",
            Box::new(move |_, _| {
                let Some(splitter) = weak_self.upgrade() else {
                    return;
                };
                let Some(gesture) = splitter.gesture() else {
                    return;
                };
                splitter.set_gesture(None);
                if let Some(callback) = splitter.drag_completed_callback() {
                    callback(SplitterDragCompletedEventArgs {
                        cumulative_delta: gesture.cumulative_delta,
                        position: gesture.position,
                        screen_position: gesture.screen_position,
                        canceled: true,
                    });
                }
            }),
        );
    }

    fn weak_self(&self) -> std::rc::Weak<Self> {
        weak_self_from_visual_owner(self)
    }
}
