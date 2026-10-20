# The oracle for rubyinteger: Ruby 2.6's Integer#to_s(base), digits,
# Integer.sqrt, pow(b, m), bit_length, gcd, lcm, round/floor/ceil/truncate
# with ndigits (and half:), int[i], divmod and remainder. Numbers are
# decimal; a half: mode is hex of its text, '~' for nil.
def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.unpack1('H*')
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  a = f[1].to_i
  begin
    out << case f[0]
           when 'ts' then a.to_s(f[2].to_i)
           when 'dg' then (f[2] == '~' ? a.digits : a.digits(f[2].to_i)).join(',')
           when 'sq' then Integer.sqrt(a).to_s
           when 'pm' then a.pow(f[2].to_i, f[3].to_i).to_s
           when 'bl' then a.bit_length.to_s
           when 'gl' then "#{a.gcd(f[2].to_i)} #{a.lcm(f[2].to_i)}"
           when 'rd' then a.round(f[2].to_i, half: (f[3] == '~' ? nil : unhex(f[3]))).to_s
           when 'fl' then a.floor(f[2].to_i).to_s
           when 'ce' then a.ceil(f[2].to_i).to_s
           when 'tr' then a.truncate(f[2].to_i).to_s
           when 'ba' then a[f[2].to_i].to_s
           when 'dm'
             q, r = a.divmod(f[2].to_i)
             "#{q} #{r} #{a.remainder(f[2].to_i)}"
           end
  rescue ArgumentError, RangeError, ZeroDivisionError, Math::DomainError => e
    out << "E:#{e.class}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
