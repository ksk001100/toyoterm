-- Copy to LazyVim's lua/plugins/toyoterm-image.lua.
local function install_sender()
  local image = require("snacks.image.image")
  if image._toyoterm_cooperative_send then
    return
  end
  image._toyoterm_cooperative_send = true

  local terminal = require("snacks.image.terminal")
  local send, ready, failed = image.send, image.ready, image.failed
  local write_request = terminal.request
  local pending = setmetatable({}, { __mode = "k" })
  local errors = setmetatable({}, { __mode = "k" })
  local queue, active = {}, false
  local deferred, transmitting = {}, false
  local pump

  -- Placement/deletion requests from other buffers must not interrupt the
  -- current chunk sequence either (for example when closing a buffer).
  terminal.request = function(request)
    if transmitting then
      deferred[#deferred + 1] = vim.deepcopy(request)
    else
      write_request(request)
    end
  end

  -- A Kitty chunked transfer must finish before another image starts.
  pump = function()
    local img = table.remove(queue, 1)
    if not img then
      active = false
      return
    end
    local fd, remaining, first
    transmitting = true
    local function finish(err)
      if fd then
        fd:close()
        fd = nil
      end
      pending[img] = nil
      if err then
        img.sent = false
        errors[img] = err
        -- Discard an incomplete transfer before the next queued image.
        pcall(write_request, { a = "d", d = "i", i = img.id })
        vim.notify("toyoterm image transfer: " .. tostring(err), vim.log.levels.ERROR)
      end
      transmitting = false
      local requests = deferred
      deferred = {}
      for _, request in ipairs(requests) do
        write_request(request)
      end
      if not err then
        local ok, message = pcall(img.on_send, img)
        if not ok then
          vim.notify("toyoterm image placement: " .. tostring(message), vim.log.levels.ERROR)
        end
      end
      vim.defer_fn(pump, 1)
    end
    local step
    step = function()
      local ok, done = pcall(function()
        if not fd then
          fd = assert(io.open(img.file, "rb"))
          remaining = assert(fd:seek("end"))
          assert(remaining > 0, "empty image")
          -- The terminal limits accumulated base64 to 32 MiB.
          assert(remaining <= 24 * 1024 * 1024, "image exceeds transfer limit")
          assert(fd:seek("set", 0))
          first = true
        end
        -- At most 64 KiB of encoded data per turn. Encode 3072-byte blocks
        -- so only the final base64 chunk can contain padding.
        for _ = 1, 16 do
          local bytes = assert(fd:read(math.min(3072, remaining)), "image read failed")
          remaining = remaining - #bytes
          local request = { m = remaining == 0 and 0 or 1, data = Snacks.util.base64(bytes) }
          if first then
            request.t, request.i, request.f = "d", img.id, 100
            first = false
          end
          write_request(request)
          if remaining == 0 then
            return true
          end
        end
      end)
      if not ok then
        finish(done)
      elseif done then
        finish()
      else
        -- Unlike uv.sleep(), a timer returns control to Neovim's input loop.
        vim.defer_fn(step, 1)
      end
    end
    step()
  end

  image.ready = function(self)
    return not pending[self] and not errors[self] and ready(self)
  end
  image.failed = function(self)
    return errors[self] ~= nil or failed(self)
  end
  image.send = function(self)
    if not terminal.env().name:find("toyoterm", 1, true) then
      return send(self)
    end
    assert(not self.sent, "Image already sent")
    self.sent = true
    pending[self], errors[self] = true, nil
    queue[#queue + 1] = self
    if not active then
      active = true
      vim.defer_fn(pump, 1)
    end
  end
end

return {
  "folke/snacks.nvim",
  opts = function(_, opts)
    if vim.env.TERM_PROGRAM ~= "toyoterm" or vim.env.TMUX or vim.env.ZELLIJ then
      return
    end

    -- Register before Snacks first caches its terminal environment. Keep the
    -- terminal's identity, and use in-band data rather than local file paths.
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
        remote = true,
        setup = install_sender,
      })
    end
    opts.image = vim.tbl_deep_extend("force", opts.image or {}, { enabled = true })
  end,
}
