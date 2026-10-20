# The oracle for rubyrational: Ruby 2.6's Rational. Strings are hex of
# UTF-8 ('-' when empty); a rational is written n/d; '-' is no argument.
def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.b.unpack1('H*')
end

def rat(t)
  n, d = t.split('/')
  Rational(Integer(n), Integer(d || '1'))
end

def run(f)
  case f[0]
  when 'ps' then Rational(unhex(f[1])).inspect
  when 'tr' then unhex(f[1]).to_r.inspect
  when 'mk' then Rational(Integer(f[1]), Integer(f[2])).inspect
  when 'ts' then rat(f[1]).to_s
  when 'ar'
    a = rat(f[2])
    b = f[3].include?('/') ? rat(f[3]) : Integer(f[3])
    case f[1]
    when 'add' then (a + b).inspect
    when 'sub' then (a - b).inspect
    when 'mul' then (a * b).inspect
    when 'div' then (a / b).inspect
    when 'cmp' then (a <=> b).inspect
    when 'pow' then (a**b).inspect
    end
  when 'rd'
    r = rat(f[2])
    args = f[3] == '-' ? [] : [Integer(f[3])]
    opts = f[4] == '-' ? {} : { half: f[4].to_sym }
    (f[1] == 'round' ? r.round(*args, **opts) : r.send(f[1], *args)).inspect
  when 'rz' then rat(f[1]).rationalize(*(f[2] == '-' ? [] : [rat(f[2])])).inspect
  end
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << run(f)
  rescue ArgumentError, ZeroDivisionError, FloatDomainError => e
    out << "E:#{e.class}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
