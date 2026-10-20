-- The oracle for packages/luautf8: Lua 5.5's utf8 library. Text travels
-- as hex ("-" when empty), "_" stands for an argument left out, and the
-- answer lists the results (integers, "fail" for a fail value, s:HEX for
-- a string) joined by ","; errors as E: and the message.
local function unhex(h)
  if h == "-" then return "" end
  return (h:gsub("..", function(x) return string.char(tonumber(x, 16)) end))
end
local function hex(s)
  if s == "" then return "-" end
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end
local function arg(v)
  if v == "_" then return nil end
  if v == "T" then return true end
  if v == "F" then return false end
  return math.tointeger(tonumber(v))
end
local function show(...)
  local t = table.pack(...)
  local parts = {}
  for k = 1, t.n do
    local x = t[k]
    if x == nil or x == false then parts[#parts + 1] = "fail"
    elseif type(x) == "string" then parts[#parts + 1] = "s:" .. hex(x)
    else parts[#parts + 1] = tostring(x) end
  end
  return table.concat(parts, ",")
end
local out = {}
for l in io.lines() do
  if #l > 0 then
    local f = {}
    for w in l:gmatch("%S+") do f[#f + 1] = w end
    table.remove(f, 1)
    local op, r = f[1], nil
    local res
    if op == "char" then
      local args = {}
      for k = 2, #f do args[#args + 1] = arg(f[k]) end
      res = table.pack(pcall(utf8.char, table.unpack(args, 1, #f - 1)))
    elseif op == "codes" then
      local ok, it, st, ctl = pcall(utf8.codes, unhex(f[2]), arg(f[3]))
      if not ok then
        res = {false, it, n = 2}
      else
        local acc = {}
        while true do
          local ok2, p, c = pcall(it, st, ctl)
          if not ok2 then res = {false, p, n = 2}; break end
          if p == nil then res = {true, table.concat(acc, " "), n = 2}; break end
          acc[#acc + 1] = p .. ":" .. c
          ctl = p
        end
      end
    else
      local fn = utf8[op]
      local args = {}
      for k = 3, #f do args[k - 2] = arg(f[k]) end
      res = table.pack(pcall(fn, unhex(f[2]), table.unpack(args, 1, #f - 2)))
    end
    if not res[1] then out[#out + 1] = "E:" .. res[2]
    elseif op == "codes" then out[#out + 1] = "c:" .. res[2]
    else out[#out + 1] = show(table.unpack(res, 2, res.n)) end
  end
end
io.write(table.concat(out, "\n"), #out > 0 and "\n" or "")
