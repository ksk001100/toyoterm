# Named themes also become native OSC 1337 color presets.
Toyoterm.theme('recipe.day') do |colors|
  colors.background = '#f5f1e8'
  colors.foreground = '#293340'
  colors.cursor = '#005f87'
  colors.selection = '#b8d8e8'
  colors.tab_bar = '#e5dfd3'
  colors.tab_active = '#c9dce5'
  colors.tab_inactive = '#eee8dd'
  colors.workspace_bar = '#e5dfd3'
  colors.status_bar = '#e5dfd3'
  colors.pane_border = '#005f87'
  colors.zoomed_pane_border = '#a04b00'
  colors.search_match = '#e0c46c'
  colors.search_match_active = '#efb849'
  colors.ansi = %w[#293340 #a52a35 #32643c #805b16 #1e5d91 #80508b #206b70 #d6d0c4
                  #667080 #bd3544 #3d7948 #976c1b #2674ad #9660a1 #268188 #f5f1e8]
end

Toyoterm.theme('recipe.night') do |colors|
  colors.background = '#151b26'
  colors.foreground = '#d5dfed'
  colors.cursor = '#8bd5ca'
  colors.selection = '#385473'
  colors.tab_bar = '#111722'
  colors.tab_active = '#293d55'
  colors.tab_inactive = '#1c2635'
  colors.workspace_bar = '#111722'
  colors.status_bar = '#111722'
  colors.pane_border = '#8bd5ca'
  colors.zoomed_pane_border = '#f5c878'
  colors.search_match = '#80642e'
  colors.search_match_active = '#b78835'
  colors.ansi = %w[#202938 #ed8796 #a6da95 #eed49f #8aadf4 #c6a0f6 #8bd5ca #cad3f5
                  #6e7c94 #f5a9b8 #bce5ae #f5dfb8 #aac4fa #d7bafa #a9e4dd #f0f4fc]
end
