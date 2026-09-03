Toyoterm.configure do |config|
  config.font do |font|
    font.family = "monospace"
    font.fallback = ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
    font.size = 14
    font.weight = 400
  end

  config.colors do |colors|
    colors.background = "#090b0e"
    colors.foreground = "#dce1e8"
    colors.cursor = "#f5f7fa"
    colors.selection = "#375891"
    colors.tab_bar = "#11151b"
    colors.tab_active = "#18243a"
    colors.tab_inactive = "#15191f"
    colors.workspace_bar = "#0d1014"
    colors.status_bar = "#101419"
    colors.pane_border = "#375891"
    colors.search_match = "#c4972f"
    colors.search_match_active = "#ffbe3a"
  end

  config.window do |window|
    window.opacity = 1.0
    window.width = 960
    window.height = 600
    window.min_width = 320
    window.min_height = 180
    window.decorations = true
    window.resizable = true
    window.always_on_top = false
    window.title = "toyoterm"
  end

  config.ui do |ui|
    ui.padding_x = 8
    ui.padding_y = 8
    ui.line_height = 1.2857143
    ui.tab_bar = true
    ui.tab_bar_height = 30
    ui.tab_width = 160
    ui.workspace_bar = true
    ui.workspace_bar_height = 24
    ui.workspace_width = 160
    ui.status_bar_height = 24
    ui.pane_divider_width = 2
    ui.active_pane_border_width = 2
  end

  config.behavior do |behavior|
    behavior.scroll_lines = 3
    behavior.copy_on_select = false
  end

  config.scrollback_lines = 10_000

  # The leader prefix is optional. These are the former built-in GUI bindings,
  # expressed entirely through the config DSL.
  config.leader key: "b", mods: "CTRL", timeout: 1000

  config.keys do
    key("F11").toggle_fullscreen
    ctrl("TAB").next_tab
    ctrl_shift("TAB").previous_tab
    ctrl_alt("LEFT").previous_workspace
    ctrl_alt("RIGHT").next_workspace

    # Vim-like visual selection. The leader keeps normal v available to the shell.
    leader("v").toggle_visual_mode
    key("SPACE").select_visual_selection
    key("ESCAPE").end_visual_selection
    key("h").move_visual_selection(:left)
    key("j").move_visual_selection(:down)
    key("k").move_visual_selection(:up)
    key("l").move_visual_selection(:right)
    key("LEFT").move_visual_selection(:left)
    key("RIGHT").move_visual_selection(:right)
    key("UP").move_visual_selection(:up)
    key("DOWN").move_visual_selection(:down)
    key("0").move_visual_selection(:line_start)
    key("$").move_visual_selection(:line_end)
    key("y").yank_selection

    leader("s").split(:right)

    if Toyoterm.platform == :macos
      ctrl_super("f").toggle_fullscreen
      primary("r").reload_config
      primary_shift("f").search
      primary("t").new_tab
      primary("n").new_workspace
      primary("w").close_tab
      primary_shift("w").close_pane
      primary("d").split(:right)
      primary_shift("d").split(:down)
      primary_alt("left").activate_pane(:left)
      primary_alt("right").activate_pane(:right)
      primary_alt("up").activate_pane(:up)
      primary_alt("down").activate_pane(:down)
      primary("c").copy_selection
      primary("v").paste_clipboard
    else
      ctrl_shift("r").reload_config
      ctrl_shift("f").search
      ctrl_shift("t").new_tab
      ctrl_shift("n").new_workspace
      ctrl_shift("w").close_tab
      ctrl_shift("\\").split(:right)
      ctrl_shift("|").split(:right)
      ctrl_shift("-").split(:down)
      ctrl_shift("_").split(:down)
      ctrl_shift("q").close_pane
      ctrl_shift("left").activate_pane(:left)
      ctrl_shift("right").activate_pane(:right)
      ctrl_shift("up").activate_pane(:up)
      ctrl_shift("down").activate_pane(:down)
      alt("F10").toggle_maximize
      alt("F9").minimize_window
      ctrl_shift("c").copy_selection
      ctrl_shift("v").paste_clipboard
    end
  end
end

# With shell integration enabled, show failed commands in the pane corner.
Toyoterm.on :command_started do |event|
  event.pane.badge = nil
end

Toyoterm.on :command_finished do |event|
  status = event.exit_status
  event.pane.badge = status.nil? || status == 0 ? nil : "exit #{status}"
end
