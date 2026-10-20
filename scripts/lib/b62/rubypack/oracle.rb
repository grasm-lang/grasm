# The oracle for rubypack: Ruby 2.6's Array#pack, String#unpack and
# String#unpack1. Bytes are hex ('-' when empty); an item is i<decimal>,
# s<hex> or n (nil), joined by '.'; '~' is an empty list.
$VERBOSE = nil

def unhex(h)
  h == '-' ? ''.b : [h].pack('H*')
end

def hx(s)
  s.empty? ? '-' : s.unpack1('H*')
end

def item(t)
  return nil if t == 'n'
  return Integer(t[1..-1]) if t[0] == 'i'
  unhex(t[1..-1])
end

def show(v)
  v.nil? ? 'n' : v.is_a?(Integer) ? "i#{v}" : "s#{hx(v)}"
end

def run(f)
  case f[0]
  when 'pk'
    items = f[2] == '~' ? [] : f[2].split('.').map { |t| item(t) }
    hx(items.pack(unhex(f[1])))
  when 'up'
    r = unhex(f[1]).unpack(unhex(f[2]))
    r.empty? ? '~' : r.map { |v| show(v) }.join(',')
  when 'u1' then show(unhex(f[1]).unpack1(unhex(f[2])))
  end
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << run(f)
  rescue ArgumentError, RangeError, TypeError => e
    out << "E:#{e.class}:#{hx(e.message.b)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
