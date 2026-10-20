# The oracle for rubyjson: Ruby 2.6's json 2.1.0 (C extension). Lines are
# "gen <tree>", "pgen <tree>" or "parse <hex of the JSON text bytes>".
# A tree is n, t, f, i<decimal>;, d<16 hex digits of the double>,
# s<hex bytes>;, a<count>: followed by the items, o<count>: followed by
# key/value pairs (keys are s...; trees).
require 'json'

def unhex(h)
  (h.empty? || h == '-') ? ''.b : [h].pack('H*')
end

def read_tree(t, i)
  case t[i]
  when 'n' then [nil, i + 1]
  when 't' then [true, i + 1]
  when 'f' then [false, i + 1]
  when 'i' then j = t.index(';', i); [t[i + 1...j].to_i, j + 1]
  when 'd' then [[t[i + 1, 16]].pack('H*').unpack1('G'), i + 17]
  when 's' then j = t.index(';', i); [unhex(t[i + 1...j]).force_encoding('UTF-8'), j + 1]
  when 'a', 'o'
    kind = t[i]
    j = t.index(':', i)
    n = t[i + 1...j].to_i
    i = j + 1
    if kind == 'a'
      items = []
      n.times { v, i = read_tree(t, i); items << v }
      [items, i]
    else
      h = {}
      n.times { k, i = read_tree(t, i); v, i = read_tree(t, i); h[k] = v }
      [h, i]
    end
  end
end

def write_tree(v)
  case v
  when nil then 'n'
  when true then 't'
  when false then 'f'
  when Integer then "i#{v};"
  when Float then 'd' + [v].pack('G').unpack1('H*')
  when String then 's' + v.b.unpack1('H*') + ';'
  when Array then "a#{v.size}:" + v.map { |x| write_tree(x) }.join
  when Hash then "o#{v.size}:" + v.map { |k, x| write_tree(k) + write_tree(x) }.join
  end
end

def hx(s)
  s.empty? ? '-' : s.b.unpack1('H*')
end

out = []
STDIN.each_line do |l|
  op, arg = l.chomp.split(' ', 2)
  next if op.nil?
  begin
    out << case op
           when 'gen' then hx(JSON.generate(read_tree(arg, 0)[0]))
           when 'pgen' then hx(JSON.pretty_generate(read_tree(arg, 0)[0]))
           else write_tree(JSON.parse(unhex(arg).force_encoding('UTF-8')))
           end
  rescue JSON::JSONError => e
    out << "E:#{e.class}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
