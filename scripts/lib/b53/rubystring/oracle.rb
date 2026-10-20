# The oracle for packages/rubystring: Ruby 2.6's String#succ, tr, tr_s,
# delete, squeeze and count on byte strings. Bytes are hex, '-' when empty.
def unhex(h)
  h == '-' ? ''.b : [h].pack('H*').b
end

def hx(s)
  s.empty? ? '-' : s.unpack1('H*')
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    s = unhex(f[1])
    sets = f[2..-1].map { |x| unhex(x) }
    out << case f[0]
           when 'succ' then hx(s.succ)
           when 'tr' then hx(s.tr(sets[0], sets[1]))
           when 'trs' then hx(s.tr_s(sets[0], sets[1]))
           when 'del' then hx(s.delete(*sets))
           when 'sq' then hx(s.squeeze(*sets))
           else s.count(*sets).to_s
           end
  rescue ArgumentError => e
    out << 'E:' + hx(e.message.b)
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
