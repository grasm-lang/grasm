-- The oracle for packages/luapattern: Lua 5.5's own string.find, match,
-- gmatch and gsub. Strings are hex, '-' when empty.
local function unhex(h)
  if h == "-" then return "" end
  return (h:gsub("..", function(x) return string.char(tonumber(x, 16)) end))
end
local function hx(s)
  if #s == 0 then return "-" end
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end
local function caps(...)
  local out = {}
  for i = 1, select("#", ...) do
    local v = select(i, ...)
    if math.type(v) == "integer" then out[#out + 1] = "p" .. v else out[#out + 1] = "s" .. hx(v) end
  end
  if #out == 0 then return "~" end
  return table.concat(out, ",")
end
local function line(f)
  local op = f[1]
  if op == "f" then
    local r = table.pack(string.find(unhex(f[2]), unhex(f[3]), tonumber(f[4]), f[5] == "1"))
    if r[1] == nil then return "nil" end
    return r[1] .. " " .. r[2] .. " " .. caps(table.unpack(r, 3, r.n))
  elseif op == "m" then
    local r = table.pack(string.match(unhex(f[2]), unhex(f[3]), tonumber(f[4])))
    if r[1] == nil then return "nil" end
    return caps(table.unpack(r, 1, r.n))
  elseif op == "g" then
    local out = {}
    local it = string.gmatch(unhex(f[2]), unhex(f[3]), tonumber(f[4]))
    while #out <= 1000 do
      local r = table.pack(it())
      if r[1] == nil then break end
      out[#out + 1] = caps(table.unpack(r, 1, r.n))
    end
    if #out == 0 then return "none" end
    return table.concat(out, ";")
  end
  local repl
  if op == "r" then repl = unhex(f[4]) else
    repl = {}
    if f[4] ~= "-" then
      for kv in f[4]:gmatch("[^,]+") do
        local k, v = kv:match("^([^=]*)=(.*)$")
        repl[unhex(k)] = unhex(v)
      end
    end
  end
  local s, n = string.gsub(unhex(f[2]), unhex(f[3]), repl, f[5] ~= "~" and tonumber(f[5]) or nil)
  return hx(s) .. " " .. n
end
local out = {}
for l in io.lines() do
  if #l > 0 then
    local f = {}
    for w in l:gmatch("%S+") do f[#f + 1] = w end
    local ok, r = pcall(line, f)
    -- An error carries the calling script's file and line; the message
    -- is what follows them.
    out[#out + 1] = ok and r or ("E:" .. hx((tostring(r):gsub("^[^:]*:%d+: ", ""))))
  end
end
io.write(table.concat(out, "\n"), "\n")
