use super::*;
use gpui::{
    Application, Bounds, Context, EntityInputHandler, FocusHandle, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px,
    size,
};
use std::cell::RefCell;
use std::ops::Range;

pub(super) fn run(
    app: ToyotermApplication,
    receiver: async_channel::Receiver<AppEvent>,
) -> Result<(), AppError> {
    let failure = Rc::new(RefCell::new(None));
    let result = failure.clone();
    let launch_failure = failure.clone();
    Application::new().run(move |cx| {
        let config = &app.script_snapshot.config.window;
        let bounds = Bounds::centered(None, size(px(config.width), px(config.height)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: config.decorations.then(|| TitlebarOptions {
                title: Some(app.base_window_title().to_owned().into()),
                ..Default::default()
            }),
            is_resizable: config.resizable,
            window_min_size: Some(size(px(config.min_width), px(config.min_height))),
            window_background: WindowBackgroundAppearance::Transparent,
            #[cfg(target_os = "linux")]
            app_id: app.app_id.clone(),
            ..Default::default()
        };
        let failure = launch_failure.clone();
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| TerminalView::new(app, receiver, failure, window, cx))
        }) {
            Ok(_) => cx.activate(true),
            Err(error) => {
                result.borrow_mut().replace(error.to_string());
                cx.quit();
            }
        }
    });
    match failure.borrow_mut().take() {
        Some(error) => Err(AppError(error)),
        None => Ok(()),
    }
}
struct TerminalView {
    app: ToyotermApplication,
    control: AppControl,
    focus: FocusHandle,
    failure: Rc<RefCell<Option<String>>>,
    marked_selection: Range<usize>,
    timer_deadline: Option<Instant>,
    redraw_scheduled: bool,
}
impl TerminalView {
    fn new(
        mut app: ToyotermApplication,
        receiver: async_channel::Receiver<AppEvent>,
        failure: Rc<RefCell<Option<String>>>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let control = AppControl::default();
        let state = Rc::new(Window::new(
            physical_size(window),
            f64::from(window.scale_factor()),
        ));
        app.initialize(&control, state, window);
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.observe_window_bounds(window, |this, window, cx| {
            this.resize(window);
            this.finish(window, cx);
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            this.app.window_event(
                &this.control,
                WindowEvent::Focused(window.is_window_active()),
            );
            this.finish(window, cx);
        })
        .detach();
        cx.on_release(|this, _| {
            this.app.shutdown();
            if let Some(error) = this.app.fatal_error.take() {
                this.failure.borrow_mut().replace(error);
            }
        })
        .detach();
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            true
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                let mut batch = Vec::with_capacity(128);
                push_batched_event(&mut batch, event);
                for _ in 1..128 {
                    match receiver.try_recv() {
                        Ok(event) => push_batched_event(&mut batch, event),
                        Err(_) => break,
                    }
                }
                if this
                    .update_in(cx, |this, window, cx| {
                        for event in batch {
                            this.app.user_event(&this.control, event);
                            if this.control.exiting.get() {
                                break;
                            }
                        }
                        this.finish(window, cx);
                    })
                    .is_err()
                {
                    break;
                }
                // Yield between bounded batches so sustained TUI output cannot
                // starve keyboard and window events on GPUI's foreground loop.
                cx.background_executor().timer(Duration::ZERO).await;
            }
        })
        .detach();
        let mut this = Self {
            app,
            control,
            focus,
            failure,
            marked_selection: 0..0,
            timer_deadline: None,
            redraw_scheduled: false,
        };
        this.finish(window, cx);
        this
    }
    fn resize(&mut self, window: &gpui::Window) {
        let Some(state) = self.app.window.clone() else {
            return;
        };
        let size = physical_size(window);
        let scale = f64::from(window.scale_factor());
        if state.size.replace(size) != size || state.scale.replace(scale) != scale {
            state.scale.set(scale);
            self.app
                .window_event(&self.control, WindowEvent::Resized(size));
        }
    }
    fn finish(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.control.exiting.get() {
            if let Some(error) = self.app.fatal_error.take() {
                self.failure.borrow_mut().replace(error);
            }
            self.app.shutdown();
            cx.quit();
            return;
        }
        self.app.about_to_wait(&self.control);
        let deadline = self.control.deadline.get();
        if deadline != self.timer_deadline {
            self.timer_deadline = deadline;
            if let Some(deadline) = deadline {
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor()
                        .timer(deadline.saturating_duration_since(Instant::now()))
                        .await;
                    let _ = this.update_in(cx, move |this, window, cx| {
                        if this.timer_deadline == Some(deadline) {
                            this.timer_deadline = None;
                            this.finish(window, cx);
                        }
                    });
                })
                .detach();
            }
        }
        let Some(state) = self.app.window.clone() else {
            return;
        };
        if state.font_changed.replace(false) {
            if let Some(renderer) = &self.app.renderer {
                self.app.cell_metrics.width = f64::from(
                    renderer.terminal_cell_width(self.app.cell_metrics.font_size, window),
                );
            }
            if let Err(error) = self
                .app
                .resize_panes(state.inner_size(), state.scale_factor())
            {
                self.app.fail(&self.control, error);
            }
            self.app.sync_active_renderer(state.scale_factor());
        }
        window.set_window_title(&state.title.borrow());
        for command in state.commands.borrow_mut().drain(..) {
            match command {
                WindowCommand::Maximize(value) => {
                    if window.is_maximized() != value {
                        window.zoom_window();
                    }
                }
                WindowCommand::Minimize => window.minimize_window(),
                WindowCommand::Fullscreen(value) => {
                    if window.is_fullscreen() != value {
                        window.toggle_fullscreen();
                    }
                }
            }
        }
        state.maximized.set(window.is_maximized());
        state.fullscreen.set(window.is_fullscreen());
        if state.redraw.get() && !self.redraw_scheduled {
            state.redraw.set(false);
            self.redraw_scheduled = true;
            cx.notify();
            let entity = cx.entity();
            window.on_next_frame(move |window, cx| {
                entity.update(cx, |this, cx| {
                    this.redraw_scheduled = false;
                    this.finish(window, cx);
                });
            });
        }
    }
    fn key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = ModifiersState::from(event.keystroke.modifiers);
        self.app.modifiers = modifiers;
        let logical = logical_key(&event.keystroke.key);
        let key = KeyEvent {
            logical_key: logical,
            text: event.keystroke.key_char.clone(),
            state: ElementState::Pressed,
            repeat: event.is_held,
        };
        // Text goes through GPUI's input handler, including IME and dead-key composition.
        let needs_finish = if matches!(
            key.logical_key,
            Key::Character(_) | Key::Named(NamedKey::Space)
        ) && !modifiers.control_key()
            && !modifiers.super_key()
            && (!modifiers.alt_key()
                || event
                    .keystroke
                    .key_char
                    .as_ref()
                    .is_some_and(|c| c != &event.keystroke.key))
            && !self.app.search_open
            && self.app.visual_selection.is_none()
        {
            let handled = self
                .app
                .handle_leader_key(&key, modifiers)
                .and_then(|handled| {
                    if handled {
                        Ok(true)
                    } else {
                        self.app.handle_keybinding(&key, modifiers)
                    }
                });
            match handled {
                Ok(true) => {
                    cx.stop_propagation();
                    true
                }
                Ok(false) => false,
                Err(error) => {
                    tracing::warn!(%error,"key binding failed");
                    cx.stop_propagation();
                    true
                }
            }
        } else {
            self.app
                .window_event(&self.control, WindowEvent::KeyboardInput { event: key });
            cx.stop_propagation();
            true
        };
        if needs_finish {
            self.finish(window, cx);
        }
    }
}
impl Render for TerminalView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.resize(window);
        self.app
            .window_event(&self.control, WindowEvent::RedrawRequested);
        let renderer = self.app.renderer.clone();
        let smoke = self.app.exit_after_startup;
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .id("terminal")
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                    this.app.modifiers = event.modifiers.into();
                    let scale = f64::from(window.scale_factor());
                    this.app.window_event(
                        &this.control,
                        WindowEvent::CursorMoved {
                            position: PhysicalPosition::new(
                                f64::from(f32::from(event.position.x)) * scale,
                                f64::from(f32::from(event.position.y)) * scale,
                            ),
                        },
                    );
                    this.finish(window, cx);
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.app.modifiers = event.modifiers.into();
                    let scale = f64::from(window.scale_factor());
                    this.app.mouse_position = PhysicalPosition::new(
                        f64::from(f32::from(event.position.x)) * scale,
                        f64::from(f32::from(event.position.y)) * scale,
                    );
                    this.app.window_event(
                        &this.control,
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                        },
                    );
                    this.finish(window, cx);
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.app.window_event(
                        &this.control,
                        WindowEvent::MouseInput {
                            state: ElementState::Released,
                            button: MouseButton::Left,
                        },
                    );
                    this.finish(window, cx);
                }),
            )
            .on_scroll_wheel(
                cx.listener(|this, event: &gpui::ScrollWheelEvent, window, cx| {
                    this.app.modifiers = event.modifiers.into();
                    let delta = match event.delta {
                        gpui::ScrollDelta::Lines(p) => MouseScrollDelta::LineDelta(p.y),
                        gpui::ScrollDelta::Pixels(p) => {
                            MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                                f64::from(f32::from(p.x)) * f64::from(window.scale_factor()),
                                f64::from(f32::from(p.y)) * f64::from(window.scale_factor()),
                            ))
                        }
                    };
                    this.app
                        .window_event(&this.control, WindowEvent::MouseWheel { delta });
                    this.finish(window, cx);
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, entity),
                            cx,
                        );
                        if let Some(renderer) = renderer {
                            renderer.paint(bounds.origin, bounds, window, cx);
                            if smoke {
                                cx.quit();
                            }
                        }
                    },
                )
                .size_full(),
            )
    }
}
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut gpui::Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.app.ime_preedit.as_deref().unwrap_or("");
        let units: Vec<_> = text.encode_utf16().collect();
        let start = range.start.min(units.len());
        let end = range.end.min(units.len()).max(start);
        *actual = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut gpui::Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection {
            range: self.marked_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(
        &self,
        _: &mut gpui::Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.app
            .ime_preedit
            .as_ref()
            .map(|t| 0..t.encode_utf16().count())
    }
    fn unmark_text(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.app
            .window_event(&self.control, WindowEvent::Ime(Ime::Disabled));
        self.marked_selection = 0..0;
        self.finish(window, cx);
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.app
            .window_event(&self.control, WindowEvent::Ime(Ime::Commit(text.into())));
        self.marked_selection = 0..0;
        self.finish(window, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let mut units: Vec<_> = self
            .app
            .ime_preedit
            .as_deref()
            .unwrap_or("")
            .encode_utf16()
            .collect();
        let range = range.unwrap_or(0..units.len());
        let start = range.start.min(units.len());
        let end = range.end.min(units.len()).max(start);
        units.splice(start..end, text.encode_utf16());
        let len = units.len();
        self.marked_selection = selection
            .map(|r| (start + r.start).min(len)..(start + r.end).min(len))
            .unwrap_or(len..len);
        self.app.window_event(
            &self.control,
            WindowEvent::Ime(Ime::Preedit(String::from_utf16_lossy(&units), None)),
        );
        self.finish(window, cx);
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<gpui::Pixels>,
        window: &mut gpui::Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let (position, dimensions) = self.app.window.as_ref()?.ime_bounds.get();
        let scale = f64::from(window.scale_factor());
        Some(Bounds::new(
            bounds.origin
                + point(
                    px((position.x / scale) as f32),
                    px((position.y / scale) as f32),
                ),
            size(
                px((f64::from(dimensions.width) / scale) as f32),
                px((f64::from(dimensions.height) / scale) as f32),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: gpui::Point<gpui::Pixels>,
        _: &mut gpui::Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
fn physical_size(window: &gpui::Window) -> PhysicalSize<u32> {
    let size = window.viewport_size();
    let scale = window.scale_factor();
    PhysicalSize::new(
        (f32::from(size.width) * scale).round().max(0.) as u32,
        (f32::from(size.height) * scale).round().max(0.) as u32,
    )
}

fn push_batched_event(batch: &mut Vec<AppEvent>, event: AppEvent) {
    if let AppEvent::Output { pane, bytes } = event {
        if let Some(AppEvent::Output {
            pane: previous_pane,
            bytes: previous_bytes,
        }) = batch.last_mut()
            && *previous_pane == pane
        {
            previous_bytes.extend_from_slice(&bytes);
        } else {
            batch.push(AppEvent::Output { pane, bytes });
        }
    } else {
        batch.push(event);
    }
}

fn logical_key(key: &str) -> Key {
    let named = match key {
        "enter" => NamedKey::Enter,
        "backspace" => NamedKey::Backspace,
        "tab" => NamedKey::Tab,
        "escape" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "up" => NamedKey::ArrowUp,
        "down" => NamedKey::ArrowDown,
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "insert" => NamedKey::Insert,
        "delete" => NamedKey::Delete,
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ if key.chars().count() == 1 => return Key::Character(key.into()),
        _ => return Key::Unidentified,
    };
    Key::Named(named)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpui_keys_map_navigation_and_ignore_nontext_platform_names() {
        assert_eq!(logical_key("enter"), Key::Named(NamedKey::Enter));
        assert_eq!(logical_key("pageup"), Key::Named(NamedKey::PageUp));
        assert_eq!(logical_key("f12"), Key::Named(NamedKey::F12));
        assert_eq!(logical_key("é"), Key::Character("é".into()));
        assert_eq!(logical_key("shift"), Key::Unidentified);
    }
    #[test]
    fn foreground_event_channel_preserves_pty_order_and_reports_shutdown() {
        let (sender, receiver) = async_channel::unbounded();
        let sender = EventSender(sender);
        sender
            .send_event(AppEvent::Output {
                pane: PaneId(1),
                bytes: b"one".to_vec(),
            })
            .unwrap();
        sender
            .send_event(AppEvent::Output {
                pane: PaneId(1),
                bytes: b"two".to_vec(),
            })
            .unwrap();
        sender
            .send_event(AppEvent::Eof { pane: PaneId(1) })
            .unwrap();
        for expected in [b"one", b"two"] {
            let AppEvent::Output { bytes, .. } = receiver.try_recv().unwrap() else {
                panic!("out of order")
            };
            assert_eq!(bytes, expected);
        }
        assert!(matches!(receiver.try_recv().unwrap(), AppEvent::Eof { .. }));
        drop(receiver);
        assert!(
            sender
                .send_event(AppEvent::Eof { pane: PaneId(1) })
                .is_err()
        );
    }
    #[test]
    fn adjacent_pty_output_is_coalesced_without_crossing_event_boundaries() {
        let mut batch = Vec::new();
        push_batched_event(
            &mut batch,
            AppEvent::Output {
                pane: PaneId(1),
                bytes: b"one".to_vec(),
            },
        );
        push_batched_event(
            &mut batch,
            AppEvent::Output {
                pane: PaneId(1),
                bytes: b"two".to_vec(),
            },
        );
        push_batched_event(&mut batch, AppEvent::Eof { pane: PaneId(2) });
        push_batched_event(
            &mut batch,
            AppEvent::Output {
                pane: PaneId(1),
                bytes: b"three".to_vec(),
            },
        );

        assert_eq!(batch.len(), 3);
        assert!(matches!(
                &batch[0],
                AppEvent::Output { pane, bytes }
                    if *pane == PaneId(1) && bytes == b"onetwo"
        ));
        assert!(matches!(batch[1], AppEvent::Eof { pane: PaneId(2) }));
        assert!(matches!(
                &batch[2],
                AppEvent::Output { pane, bytes }
                    if *pane == PaneId(1) && bytes == b"three"
        ));
    }
}
