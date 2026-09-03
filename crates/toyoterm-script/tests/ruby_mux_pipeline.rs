use toyoterm_api::{Command, NativeCommand, SplitDirection};
use toyoterm_mux::Mux;
use toyoterm_script::ConfigManager;

#[derive(Clone, Copy, Debug)]
enum Operation {
    SendText,
    SplitPane,
    ClosePane,
    ActivatePane,
    NewTab,
    CloseTab,
    ActivateTab,
    CreateWindow,
    CloseWindow,
    ActivateWindow,
    ActivateWorkspace,
    SwitchWorkspace,
}

#[test]
fn ruby_object_operations_mutate_the_mux() {
    let cases = [
        Operation::SendText,
        Operation::SplitPane,
        Operation::ClosePane,
        Operation::ActivatePane,
        Operation::NewTab,
        Operation::CloseTab,
        Operation::ActivateTab,
        Operation::CreateWindow,
        Operation::CloseWindow,
        Operation::ActivateWindow,
        Operation::ActivateWorkspace,
        Operation::SwitchWorkspace,
    ];

    for operation in cases {
        run_case(operation);
    }
}

fn run_case(operation: Operation) {
    let mut mux = Mux::new();
    let original_workspace = mux.current_workspace();
    let original_window = mux.current_window().expect("initial window");
    let original_tab = mux.current_tab().expect("initial tab");
    let original_pane = mux.current_pane().expect("initial pane");

    let script = match operation {
        Operation::SendText => format!(
            r#"Toyoterm::Pane.new({}).send_text("from-ruby\n")"#,
            original_pane.0
        ),
        Operation::SplitPane => format!("Toyoterm::Pane.new({}).split(:right)", original_pane.0),
        Operation::ClosePane => {
            let new_pane = split_current_pane(&mut mux);
            format!("Toyoterm::Pane.new({}).close", new_pane.0)
        }
        Operation::ActivatePane => {
            split_current_pane(&mut mux);
            format!("Toyoterm::Pane.new({}).focus", original_pane.0)
        }
        Operation::NewTab => format!("Toyoterm::Window.new({}).new_tab", original_window.0),
        Operation::CloseTab => {
            let new_tab = new_tab(&mut mux);
            format!("Toyoterm::Tab.new({}).close", new_tab.0)
        }
        Operation::ActivateTab => {
            new_tab(&mut mux);
            format!("Toyoterm::Tab.new({}).focus", original_tab.0)
        }
        Operation::CreateWindow => format!(
            "Toyoterm::Workspace.new({}).create_window",
            original_workspace.0
        ),
        Operation::CloseWindow => {
            let new_window = create_window(&mut mux, original_workspace);
            format!("Toyoterm::Window.new({}).close", new_window.0)
        }
        Operation::ActivateWindow => {
            create_window(&mut mux, original_workspace);
            format!("Toyoterm::Window.new({}).focus", original_window.0)
        }
        Operation::ActivateWorkspace => {
            mux.dispatch(Command::SwitchWorkspace("second".into()))
                .expect("create second workspace");
            format!("Toyoterm::Workspace.new({}).activate", original_workspace.0)
        }
        Operation::SwitchWorkspace => "Toyoterm.switch_workspace(:backend)".to_owned(),
    };
    mux.drain_events().for_each(drop);

    let mut config = ConfigManager::new().expect("initialize mruby");
    config
        .set_live_handles(mux.native_handles())
        .expect("sync native handles");
    config.eval(&script).expect("evaluate Ruby operation");
    let commands = config
        .drain_commands_with_context(
            mux.current_workspace(),
            mux.current_window().expect("current window"),
            mux.current_tab().expect("current tab"),
            mux.current_pane().expect("current pane"),
        )
        .expect("decode native command");
    assert_eq!(commands.len(), 1, "{operation:?} emitted {commands:?}");
    let NativeCommand::Mux(command) = commands.into_iter().next().unwrap() else {
        panic!("{operation:?} did not emit a mux command");
    };
    mux.dispatch(command).expect("apply command to mux");

    match operation {
        Operation::SendText => {
            assert_eq!(
                mux.pending_input(original_pane),
                Some(b"from-ruby\n".as_slice())
            );
        }
        Operation::SplitPane => assert_eq!(mux.pane_ids().count(), 2),
        Operation::ClosePane => {
            assert_eq!(mux.pane_ids().count(), 1);
            assert_eq!(mux.current_pane(), Some(original_pane));
        }
        Operation::ActivatePane => assert_eq!(mux.current_pane(), Some(original_pane)),
        Operation::NewTab => assert_eq!(mux.tabs(original_window).unwrap().len(), 2),
        Operation::CloseTab => {
            assert_eq!(mux.tabs(original_window).unwrap(), [original_tab]);
            assert_eq!(mux.current_tab(), Some(original_tab));
        }
        Operation::ActivateTab => assert_eq!(mux.current_tab(), Some(original_tab)),
        Operation::CreateWindow => {
            assert_eq!(mux.workspace_windows(original_workspace).unwrap().len(), 2);
        }
        Operation::CloseWindow => {
            assert_eq!(
                mux.workspace_windows(original_workspace).unwrap(),
                [original_window]
            );
            assert_eq!(mux.current_window(), Some(original_window));
        }
        Operation::ActivateWindow => assert_eq!(mux.current_window(), Some(original_window)),
        Operation::ActivateWorkspace => assert_eq!(mux.current_workspace(), original_workspace),
        Operation::SwitchWorkspace => {
            assert_ne!(mux.current_workspace(), original_workspace);
            assert_eq!(mux.workspace_name(mux.current_workspace()), Some("backend"));
            assert!(mux.current_pane().is_some());
        }
    }
}

fn split_current_pane(mux: &mut Mux) -> toyoterm_api::PaneId {
    let pane = mux.current_pane().expect("current pane");
    let toyoterm_api::CommandResult::Pane(new_pane) = mux
        .dispatch(Command::Split {
            pane,
            direction: SplitDirection::Right,
        })
        .expect("prepare split pane")
    else {
        panic!("split did not return a pane");
    };
    new_pane
}

fn new_tab(mux: &mut Mux) -> toyoterm_api::TabId {
    let window = mux.current_window().expect("current window");
    let toyoterm_api::CommandResult::Tab(tab) = mux
        .dispatch(Command::NewTabIn(window))
        .expect("prepare new tab")
    else {
        panic!("new tab did not return a tab");
    };
    tab
}

fn create_window(mux: &mut Mux, workspace: toyoterm_api::WorkspaceId) -> toyoterm_api::WindowId {
    let toyoterm_api::CommandResult::Window(window) = mux
        .dispatch(Command::CreateWindow(workspace))
        .expect("prepare new window")
    else {
        panic!("create window did not return a window");
    };
    window
}
