-- Lua 5.5 arithmetic, one operation per line: <op> <value>... where a
-- value is i<integer>, f<16 hex digits of a double> or s<hex of a string>.
-- Ops: + - * / // % ^ & | ~ << >> neg bnot < <= > >= == ~= tonumber
-- (string, base) and m:<name> for math.<name>. Answers i:<int>, f:<hex
-- bits> (f:nan for any NaN), b:true|false, s:<hex>, n (nil) or e:<kind>.
local function unhex(h)
  if h == "-" then return "" end
  return (h:gsub("..", function(x) return string.char(tonumber(x, 16)) end))
end
local function hex(s)
  if s == "" then return "-" end
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end
local function value(t)
  local k, v = t:sub(1, 1), t:sub(2)
  if k == "i" then return math.tointeger(tonumber(v)) end
  if k == "f" then return (string.unpack(">d", unhex(v))) end
  return unhex(v)
end
local function show(r)
  local mt = math.type(r)
  if mt == "integer" then return "i:" .. string.format("%d", r) end
  if mt == "float" then
    if r ~= r then return "f:nan" end
    return "f:" .. hex(string.pack(">d", r))
  end
  if type(r) == "boolean" then return "b:" .. tostring(r) end
  if type(r) == "string" then return "s:" .. hex(r) end
  if r == nil then return "n" end
  return "?"
end
local function kind(msg)
  msg = tostring(msg)
  if msg:find("attempt to divide by zero", 1, true) then return "div0" end
  if msg:find("'n%0'", 1, true) then return "mod0" end
  if msg:find("has no integer representation", 1, true) then return "noint" end
  if msg:find("bitwise operation", 1, true) then return "bitstr" end
  if msg:find("attempt to compare", 1, true) then return "compare" end
  if msg:find("(zero)", 1, true) then return "fmod0" end
  if msg:find("base out of range", 1, true) then return "base" end
  if msg:find("attempt to perform arithmetic", 1, true) or msg:find("attempt to %a+ a '") then return "arith" end
  if msg:find("bad argument", 1, true) then return "arg" end
  return "other"
end
local ops = {
  ["+"] = function(a, b) return a + b end, ["-"] = function(a, b) return a - b end,
  ["*"] = function(a, b) return a * b end, ["/"] = function(a, b) return a / b end,
  ["//"] = function(a, b) return a // b end, ["%"] = function(a, b) return a % b end,
  ["^"] = function(a, b) return a ^ b end, ["&"] = function(a, b) return a & b end,
  ["|"] = function(a, b) return a | b end, ["~"] = function(a, b) return a ~ b end,
  ["<<"] = function(a, b) return a << b end, [">>"] = function(a, b) return a >> b end,
  neg = function(a) return -a end, bnot = function(a) return ~a end,
  ["<"] = function(a, b) return a < b end, ["<="] = function(a, b) return a <= b end,
  [">"] = function(a, b) return a > b end, [">="] = function(a, b) return a >= b end,
  ["=="] = function(a, b) return a == b end, ["~="] = function(a, b) return a ~= b end,
  tonumber = function(a, b) return tonumber(a, b) end,
}
for line in io.lines() do
  local f = {}
  for w in line:gmatch("%S+") do f[#f + 1] = w end
  local op = f[1]
  local args = {}
  for i = 2, #f do args[#args + 1] = value(f[i]) end
  local fn = ops[op]
  if not fn then
    local name = op:sub(3)
    fn = function(...) return math[name](...) end
  end
  local ok, r = pcall(fn, table.unpack(args, 1, #f - 1))
  print(ok and show(r) or ("e:" .. kind(r)))
end
