-- Copy to LazyVim's lua/plugins/toyoterm-image.lua.
return {
  "folke/snacks.nvim",
  opts = function(_, opts)
    if vim.env.TERM_PROGRAM ~= "toyoterm" or vim.env.TMUX or vim.env.ZELLIJ then
      return
    end

    -- Register before Snacks first caches its terminal environment. Keep the
    -- terminal's identity and let Snacks select its standard transfer method.
    local terminal = require("snacks.image.terminal")
    local found = false
    for _, env in ipairs(terminal.envs()) do
      found = found or env.name == "toyoterm"
    end
    if not found then
      table.insert(terminal.envs(), {
        name = "toyoterm",
        env = { TERM_PROGRAM = "^toyoterm$" },
        supported = true,
        placeholders = true,
      })
    end
    opts.image = vim.tbl_deep_extend("force", opts.image or {}, { enabled = true })
  end,
}
