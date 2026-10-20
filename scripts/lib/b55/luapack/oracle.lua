-- The oracle for packages/luapack: Lua 5.5's string.pack, unpack and
-- packsize. Text travels as hex ("-" when empty); a value is n (nil),
-- t/f (booleans), T (a table), i<decimal>, d<hex of a double's bits> or
-- s<hex>. Answers: s:<hex> for pack, i:<n> for packsize, and for unpack
-- the values (i:, f:<bits>, s:) joined by "," then ";" and the next
-- position; errors as E: and the message.
local function unhex(h)
  if h == "-" then return "" end
  return (h:gsub("..", function(x) return string.char(tonumber(x, 16)) end))
end
local function hex(s)
  if s == "" then return "-" end
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end
local function value(v)
  local k = v:sub(1, 1)
  if k == "n" then return nil
  elseif k == "t" then return true
  elseif k == "f" then return false
  elseif k == "T" then return {}
  elseif k == "i" then return math.tointeger(tonumber(v:sub(2)))
  elseif k == "d" then return (string.unpack(">d", unhex(v:sub(2))))
  else return unhex(v:sub(2)) end
end
local function show(x)
  if math.type(x) == "integer" then return "i:" .. x end
  if math.type(x) == "float" then return "f:" .. hex(string.pack(">d", x)) end
  return "s:" .. hex(x)
end
local out = {}
for l in io.lines() do
  if #l > 0 then
    local f = {}
    for w in l:gmatch("%S+") do f[#f + 1] = w end
    local r
    if f[1] == "p" then
      local args, n = {}, 0
      for k = 3, #f do n = n + 1; args[n] = value(f[k]) end
      local ok, res = pcall(string.pack, unhex(f[2]), table.unpack(args, 1, n))
      r = ok and ("s:" .. hex(res)) or ("E:" .. res)
    elseif f[1] == "z" then
      local ok, res = pcall(string.packsize, unhex(f[2]))
      r = ok and ("i:" .. res) or ("E:" .. res)
    else
      local res = table.pack(pcall(string.unpack, unhex(f[2]), unhex(f[3]), math.tointeger(tonumber(f[4]))))
      if not res[1] then r = "E:" .. res[2] else
        local parts = {}
        for k = 2, res.n - 1 do parts[#parts + 1] = show(res[k]) end
        r = table.concat(parts, ",") .. ";" .. res[res.n]
      end
    end
    out[#out + 1] = r
  end
end
io.write(table.concat(out, "\n"), #out > 0 and "\n" or "")
