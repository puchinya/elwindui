#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
mod appkit_lifetime {
    use elwindui::core::ui::WindowExt;
    use elwindui::ui::Window;
    use std::cell::Cell;
    use std::future::poll_fn;
    use std::rc::Rc;
    use std::task::Poll;
    use std::time::Duration;

    /// This fixture deliberately has no props, ViewModel, binding, subscription, lifecycle hook,
    /// or other authored owner that could mask application-registry retention.
    #[elwindui::component(inherits Window)]
    struct WindowLifetimeProbe {
        body: view! {
            title: "window lifetime probe"
            content: TextBlock { text: "probe" }
        },
    }

    #[elwindui::component]
    impl WindowLifetimeProbe {}

    /// Match the existing AppKit executable-test pattern: schedule the continuation from a local
    /// future, wake it from a background thread, and let AppKit service that wake-up on a later
    /// event-loop turn. Assertions never use a fixed sleep as their acceptance condition.
    fn defer_to_next_appkit_turn(callback: impl FnOnce() + 'static) {
        let started = Rc::new(Cell::new(false));
        let started_on_poll = started.clone();
        let mut callback = Some(callback);
        elwindui::core::task::spawn_local(poll_fn(move |context| {
            if !started_on_poll.replace(true) {
                let waker = context.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    waker.wake();
                });
                return Poll::Pending;
            }

            callback
                .take()
                .expect("deferred callback is polled only once after it is scheduled")(
            );
            Poll::Ready(())
        }));
    }

    pub fn run() {
        // Keep one unrelated shown Window alive until every target assertion has completed. This
        // also keeps AppKit's normal last-window policy from ending the process early.
        let sentinel = Window::new();
        sentinel.show();
        let sentinel_weak = Rc::downgrade(&sentinel);

        // LT-A1: the generated owner must survive after the constructing caller drops its Rc.
        let generated = WindowLifetimeProbe::new();
        let generated_weak = Rc::downgrade(&generated);
        generated.show();
        drop(generated);

        defer_to_next_appkit_turn(move || {
            assert!(
                generated_weak.upgrade().is_some(),
                "LT-A1: generated Window must survive caller drop while shown"
            );
            println!("LT-A1 PASS generated owner survives caller drop");

            // LT-A2: close the generated Window through its normal WindowExt path and wait for the
            // native close callback to release the final application owner.
            {
                let retained = generated_weak
                    .upgrade()
                    .expect("LT-A2: generated Window remains retained before close");
                retained.close();
            }

            let sentinel_weak = sentinel_weak.clone();
            defer_to_next_appkit_turn(move || {
                assert!(
                    generated_weak.upgrade().is_none(),
                    "LT-A2: generated Window owner must be released after close"
                );
                assert!(
                    sentinel_weak.upgrade().is_some(),
                    "LT-A2: unrelated sentinel must remain alive"
                );
                println!("LT-A2 PASS generated close releases final owner");

                // LT-A3: bare backend Window show -> hide -> show must retain exactly once.
                let bare = Window::new();
                bare.show();
                bare.hide();
                bare.show();
                let bare_weak = Rc::downgrade(&bare);
                drop(bare);

                let sentinel_weak = sentinel_weak.clone();
                defer_to_next_appkit_turn(move || {
                    assert!(
                        bare_weak.upgrade().is_some(),
                        "LT-A3: bare Window must survive caller drop after hide/re-show"
                    );
                    println!("LT-A3 PASS bare hide/re-show retains once");
                    {
                        let retained = bare_weak
                            .upgrade()
                            .expect("LT-A3: bare Window remains retained before close");
                        retained.close();
                    }

                    let sentinel_weak = sentinel_weak.clone();
                    defer_to_next_appkit_turn(move || {
                        assert!(
                            bare_weak.upgrade().is_none(),
                            "LT-A3: one close must release a hide/re-show Window"
                        );
                        assert!(
                            sentinel_weak.upgrade().is_some(),
                            "LT-A3: unrelated sentinel must remain alive"
                        );

                        // LT-A4: two shown bare Windows must be retained and released independently.
                        let first = Window::new();
                        let second = Window::new();
                        first.show();
                        second.show();
                        let first_weak = Rc::downgrade(&first);
                        let second_weak = Rc::downgrade(&second);
                        drop(first);
                        drop(second);

                        let sentinel_weak = sentinel_weak.clone();
                        defer_to_next_appkit_turn(move || {
                            assert!(
                                first_weak.upgrade().is_some(),
                                "LT-A4: first Window must be retained before close"
                            );
                            assert!(
                                second_weak.upgrade().is_some(),
                                "LT-A4: second Window must be retained before close"
                            );
                            println!("LT-A4 PASS two-window retention is independent");

                            {
                                let first = first_weak
                                    .upgrade()
                                    .expect("LT-A4: first Window remains retained");
                                first.close();
                            }

                            let sentinel_weak = sentinel_weak.clone();
                            defer_to_next_appkit_turn(move || {
                                assert!(
                                    first_weak.upgrade().is_none(),
                                    "LT-A4: closing first Window must release only first owner"
                                );
                                assert!(
                                    second_weak.upgrade().is_some(),
                                    "LT-A4: closing first Window must not release second owner"
                                );
                                println!("LT-A4 PASS first close leaves second Window alive");

                                {
                                    let second = second_weak
                                        .upgrade()
                                        .expect("LT-A4: second Window remains retained");
                                    second.close();
                                }

                                let sentinel_weak = sentinel_weak.clone();
                                defer_to_next_appkit_turn(move || {
                                    assert!(
                                        second_weak.upgrade().is_none(),
                                        "LT-A4: second Window must release after its close"
                                    );
                                    assert!(
                                        sentinel_weak.upgrade().is_some(),
                                        "LT-A4: sentinel must remain alive until final test"
                                    );
                                    println!("LT-A4 PASS second close releases second owner");

                                    // LT-A5: a never-shown Window has no retention id and must not
                                    // release an unrelated shown Window when it closes.
                                    let never_shown = Window::new();
                                    let never_shown_weak = Rc::downgrade(&never_shown);
                                    never_shown.close();
                                    drop(never_shown);

                                    let sentinel_weak = sentinel_weak.clone();
                                    defer_to_next_appkit_turn(move || {
                                        assert!(
                                            never_shown_weak.upgrade().is_none(),
                                            "LT-A5: never-shown Window must be dead after close/drop"
                                        );
                                        assert!(
                                            sentinel_weak.upgrade().is_some(),
                                            "LT-A5: never-shown close must not disturb sentinel"
                                        );
                                        println!("LT-A5 PASS never-shown close/drop is isolated");

                                        // LT-A6: this is the final native Window. No forced
                                        // termination is used; AppKit's last-window policy must end
                                        // the executable normally after this callback returns.
                                        sentinel.close();
                                        println!("LT-A6 PASS normal final termination requested");
                                    });
                                });
                            });
                        });
                    });
                });
            });
        });
    }
}

#[cfg(all(target_os = "macos", feature = "backend-appkit"))]
#[elwindui::main]
fn main() {
    appkit_lifetime::run();
}

#[cfg(not(all(target_os = "macos", feature = "backend-appkit")))]
fn main() {}
