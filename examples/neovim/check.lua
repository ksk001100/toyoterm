-- nvim --headless -u NONE -l examples/neovim/check.lua /path/to/snacks.nvim
vim.opt.runtimepath:append(assert(arg[1], "pass the Snacks plugin directory"))
require("snacks")
vim.env.TERM_PROGRAM = "toyoterm"
for _, key in ipairs({ "TMUX", "ZELLIJ", "SNACKS_KITTY", "SNACKS_SSH", "SNACKS_TOYOTERM" }) do
  vim.env[key] = nil
end
local terminal = require("snacks.image.terminal")
terminal._terminal = { terminal = "toyoterm", version = "test" }
local spec = dofile("examples/neovim/snacks.lua")
local opts = { image = { doc = { enabled = false } } }
spec.opts(nil, opts)
assert(opts.image.enabled and not opts.image.doc.enabled)
local env = terminal.env() -- Also installs the cooperative sender.
assert(env.supported and env.placeholders and env.remote)
local image = require("snacks.image.image")
local installed = image.send
spec.opts(nil, opts)
for _, e in ipairs(terminal.envs()) do
  if e.name == "toyoterm" then
    e.setup()
  end
end
assert(image.send == installed, "sender installed twice")

local writes, completed, notifications = {}, {}, {}
terminal.write = function(data)
  writes[#writes + 1] = data
end
vim.notify = function(message)
  notifications[#notifications + 1] = message
end
local bytes = string.rep("png-test-data", 256 * 1024) .. "!" -- 3 MiB + base64 padding
local file = vim.fn.tempname()
local fd = assert(io.open(file, "wb"))
fd:write(bytes)
fd:close()
local function new(id, path)
  local img = setmetatable({ id = id, file = path, fsize = #bytes, placements = {} }, image)
  img.placements[1] = {
    update = function()
      assert(img:ready(), "placement updated before transfer completed")
      completed[#completed + 1] = id
    end,
  }
  return img
end
local first, broken, second = new(31, file), new(32, file .. ".missing"), new(33, file)
local heartbeat, during_transfer = false, false
vim.defer_fn(function()
  heartbeat = true
end, 1)
vim.defer_fn(function()
  assert(not first:ready(), "fixture finished before interruption check")
  during_transfer = true
  terminal.request({ a = "d", d = "i", i = 99 })
  terminal.request({ a = "p", i = 99, p = 1, C = 1 })
end, 20)
local start = vim.uv.hrtime()
first:send()
broken:send()
second:send()
assert(not first:ready() and not second:ready(), "pending image reported ready")
assert(#completed == 0, "send blocked until completion")
assert(vim.wait(10000, function()
  return #completed == 2
end, 1), "transfer timed out")
assert(heartbeat, "event loop blocked")
assert(during_transfer)
assert(completed[1] == 31 and completed[2] == 33)
assert(broken:failed() and not broken:ready() and not broken.sent)
assert(#notifications == 1, "expected one missing-file error")

local transfers, payload, current, deferred_commands = {}, {}, nil, 0
for _, write in ipairs(writes) do
  local body = assert(write:match("^\27_G(.*)\27\\$"))
  local header, data = body:match("^(.-);(.*)$")
  header = header or body
  if header and (header:find("a=d", 1, true) or header:find("a=p", 1, true)) then
    assert(not current, "another buffer interrupted the active transfer")
    deferred_commands = deferred_commands + 1
  elseif header then
    if header:find("t=d", 1, true) then
      assert(not current, "interleaved image transfers")
      current = assert(header:match("i=(%d+)"))
    end
    assert(current, "continuation without image")
    payload[#payload + 1] = data
    if header:find("m=0", 1, true) then
      transfers[#transfers + 1] = current
      assert(table.concat(payload) == Snacks.util.base64(bytes), "corrupted image data")
      current, payload = nil, {}
    end
  end
end
assert(#transfers == 2 and transfers[1] == "31" and transfers[2] == "33")
assert(not current)
assert(deferred_commands == 3, "deferred commands or failure cleanup lost")
terminal._env = { name = "other", remote = false }
new(34, file):send()
assert(completed[3] == 34 and writes[#writes]:find("t=f", 1, true), "fallback changed")
os.remove(file)
print(("Snacks cooperative sender: passed; two 3 MiB transfers in %.0f ms"):format(
  (vim.uv.hrtime() - start) / 1e6
))
vim.cmd("qa!")
