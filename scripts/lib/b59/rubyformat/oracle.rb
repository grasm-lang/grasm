# The oracle for rubyformat: Ruby 2.6's Kernel#format. A line is the
# format (hex of UTF-8, '-' when empty) and its arguments: i:<decimal>,
# f:<16 hex digits of the double's bits>, s:<hex of UTF-8>, n (nil),
# t (true), F (false), h:<key hex>=<arg>,... (a Hash with Symbol keys).
def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.unpack1('H*')
end

def arg(t)
  case t[0]
  when 'i' then t[2..-1].to_i
  when 'f' then [t[2..-1]].pack('H*').unpack1('G')
  when 's' then unhex(t[2..-1])
  when 'n' then nil
  when 't' then true
  when 'F' then false
  when 'h'
    body = t[2..-1]
    body.empty? ? {} : body.split(',').map { |kv| k, v = kv.split('=', 2); [unhex(k).to_sym, arg(v)] }.to_h
  end
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << hx(format(unhex(f[0]), *f[1..-1].map { |t| arg(t) }).b)
  rescue ArgumentError, TypeError, KeyError, RangeError => e
    out << "E:#{e.class}:#{hx(e.message.b)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
